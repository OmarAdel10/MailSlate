//! SMTP submission adapter: the only place that talks SMTP to send
//! mail (PRD F5.6/F5.7, ARCHITECTURE §3 `smtp/`, §9.6 idempotency).
//!
//! lettre owns the wire conversation (EHLO, STARTTLS, AUTH, DATA,
//! dot-stuffing); this module owns the policy: which TLS mode a
//! security setting maps to, which auth mechanisms an account may
//! use, how an outgoing envelope is assembled (Bcc included), and
//! how a failure is classified for the outbox.
//!
//! ## Failure phases (ARCHITECTURE §9.6)
//!
//! SMTP has no duplicate suppression: a message is delivered exactly
//! when the server answers the final `250` after DATA. A failure is
//! therefore only safe to retry when we know the transaction never
//! reached that point:
//!
//! - [`SendError::PreData`] — connect, TLS, greeting, EHLO, or auth
//!   failed. No message was offered; always safe to retry.
//! - [`SendError::Transient`] — the server answered `4xx`. It
//!   explicitly refused the message; safe to retry.
//! - [`SendError::Rejected`] — the server answered `5xx`. Permanent;
//!   never auto-retry.
//! - [`SendError::Ambiguous`] — the connection failed after the
//!   transaction started without a final reply (timeout, reset,
//!   parse failure). The message may or may not have been accepted,
//!   so the outbox must check the Sent mailbox for the stamped
//!   Message-ID before any resend.
//!
//! A cheap [`AsyncSmtpTransport::test_connection`] probe runs before
//! submission so connect/TLS/auth failures are classified as
//! pre-data instead of ambiguous. The extra connection is the price
//! of phase certainty through lettre's public error API.
//!
//! The compose domain (later M4 slices) is the first consumer;
//! until it lands these items are intentionally unused outside
//! tests, so dead-code is allowed module-wide (same precedent as
//! `mime.rs` and `mime_out.rs`).

#![allow(dead_code)]

use std::str::FromStr;
use std::time::Duration;

use lettre::address::Envelope;
use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{Address, AsyncSmtpTransport, AsyncTransport, Tokio1Executor};

use crate::domain::accounts::Security;

/// How long any single submission attempt may take. A stuck server
/// must surface as a failure the outbox can retry, not a hang.
const SEND_TIMEOUT: Duration = Duration::from_secs(30);

/// Where and how to reach one account's submission server
/// (`accounts.smtp_host` / `smtp_port` / `smtp_security`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SmtpConfig {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) security: Security,
}

/// How to authenticate the submission.
pub(crate) enum SmtpAuth {
    /// Generic IMAP/SMTP accounts (PRD F1.3).
    Password { username: String, password: String },
    /// Gmail / Outlook OAuth2 accounts (PRD F1.1/F1.2): the access
    /// token goes out as an XOAUTH2 bearer assertion.
    Xoauth2 { email: String, access_token: String },
}

/// The envelope a message is submitted with. `bcc` recipients are
/// envelope-only: the builder never writes them to the wire, but
/// they must still receive the message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OutgoingEnvelope {
    pub(crate) from: String,
    pub(crate) to: Vec<String>,
    pub(crate) cc: Vec<String>,
    pub(crate) bcc: Vec<String>,
}

/// Submission failures, classified by whether a retry can duplicate.
#[derive(Debug, thiserror::Error)]
pub(crate) enum SendError {
    /// A sender or recipient string is not a valid addr-spec.
    #[error("invalid envelope: {detail}")]
    InvalidEnvelope { detail: String },
    /// Failure before the message was offered (connect/TLS/auth).
    #[error("send failed before DATA ({stage}): {detail}")]
    PreData {
        /// Coarse stage: `connect`, `tls`, `auth`, or `connect timeout`.
        stage: &'static str,
        detail: String,
    },
    /// The server answered `4xx`: refused, retry later.
    #[error("server transiently refused ({code}): {detail}")]
    Transient { code: u16, detail: String },
    /// The server answered `5xx`: permanent refusal.
    #[error("server rejected the message ({code}): {detail}")]
    Rejected { code: u16, detail: String },
    /// No final reply: the message may have been accepted.
    #[error("ambiguous send outcome: {detail}")]
    Ambiguous { detail: String },
}

impl SendError {
    /// True when resending cannot duplicate a delivered message.
    pub(crate) fn retry_safe(&self) -> bool {
        matches!(self, Self::PreData { .. } | Self::Transient { .. })
    }

    /// True when the outbox must check the Sent mailbox for the
    /// stamped Message-ID before deciding to resend (ARCHITECTURE
    /// §9.6, "check before resend").
    pub(crate) fn needs_sent_check(&self) -> bool {
        matches!(self, Self::Ambiguous { .. })
    }
}

/// Submits one already-built RFC 5322 message (the wire bytes from
/// [`crate::infra::mime_out`]).
///
/// `wire` must be CRLF-normalized and terminated. lettre performs
/// dot-stuffing and appends `\r\n.\r\n` itself, so the final CRLF is
/// dropped here: the terminator restores it and the server receives
/// exactly the built bytes. Callers must not pre-stuff dots or
/// append the terminator.
pub(crate) async fn send(
    config: &SmtpConfig,
    auth: &SmtpAuth,
    envelope: &OutgoingEnvelope,
    wire: &[u8],
) -> Result<(), SendError> {
    let envelope = build_envelope(envelope)?;
    let transport = transport(config, auth)?;
    let wire = wire.strip_suffix(&b"\r\n"[..]).unwrap_or(wire);

    // Phase probe: everything that can fail here happens before the
    // message is offered, so any failure is safe to retry.
    transport
        .test_connection()
        .await
        .map_err(|error| SendError::PreData {
            stage: pre_data_stage(&error),
            detail: error.to_string(),
        })?;

    transport
        .send_raw(&envelope, wire)
        .await
        .map(|_response| ())
        .map_err(|error| {
            if let Some(code) = error.status() {
                let code = u16::from(code);
                return if error.is_transient() {
                    SendError::Transient {
                        code,
                        detail: error.to_string(),
                    }
                } else {
                    SendError::Rejected {
                        code,
                        detail: error.to_string(),
                    }
                };
            }
            // No final reply: the transaction may have reached DATA.
            SendError::Ambiguous {
                detail: error.to_string(),
            }
        })
}

/// Builds the lettre transport for one account's security setting
/// and credentials.
fn transport(
    config: &SmtpConfig,
    auth: &SmtpAuth,
) -> Result<AsyncSmtpTransport<Tokio1Executor>, SendError> {
    let tls = tls_mode(config.security, &config.host)?;
    let credentials = match auth {
        SmtpAuth::Password { username, password } => {
            Credentials::new(username.clone(), password.clone())
        }
        SmtpAuth::Xoauth2 {
            email,
            access_token,
        } => Credentials::new(email.clone(), access_token.clone()),
    };

    Ok(
        AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&config.host)
            .port(config.port)
            .tls(tls)
            .credentials(credentials)
            .authentication(mechanisms(auth))
            .timeout(Some(SEND_TIMEOUT))
            .build(),
    )
}

/// Maps the account's stored security level to lettre's TLS mode.
/// `Plaintext` is only valid for local relays; remote providers are
/// configured with `tls` or `starttls` (ARCHITECTURE DB schema).
fn tls_mode(security: Security, host: &str) -> Result<Tls, SendError> {
    let parameters = || {
        TlsParameters::new(host.to_string()).map_err(|error| SendError::PreData {
            stage: "tls config",
            detail: error.to_string(),
        })
    };
    match security {
        Security::Tls => Ok(Tls::Wrapper(parameters()?)),
        Security::StartTls => Ok(Tls::Required(parameters()?)),
        Security::Plaintext => Ok(Tls::None),
    }
}

/// Auth mechanisms an account may use, most preferred first.
/// lettre builds the XOAUTH2 assertion itself from the credentials,
/// so the token must be the secret, not a pre-formatted payload.
fn mechanisms(auth: &SmtpAuth) -> Vec<Mechanism> {
    match auth {
        SmtpAuth::Password { .. } => vec![Mechanism::Plain, Mechanism::Login],
        SmtpAuth::Xoauth2 { .. } => vec![Mechanism::Xoauth2],
    }
}

/// Assembles the SMTP envelope: reverse-path plus every To, Cc, and
/// Bcc recipient. Display names are not part of an envelope, so the
/// strings must be bare addr-specs.
fn build_envelope(envelope: &OutgoingEnvelope) -> Result<Envelope, SendError> {
    let from = parse_address(&envelope.from)?;
    let mut recipients =
        Vec::with_capacity(envelope.to.len() + envelope.cc.len() + envelope.bcc.len());
    for address in envelope.to.iter().chain(&envelope.cc).chain(&envelope.bcc) {
        recipients.push(parse_address(address)?);
    }
    Envelope::new(Some(from), recipients).map_err(|error| SendError::InvalidEnvelope {
        detail: error.to_string(),
    })
}

fn parse_address(value: &str) -> Result<Address, SendError> {
    Address::from_str(value.trim()).map_err(|error| SendError::InvalidEnvelope {
        detail: format!("{value:?}: {error}"),
    })
}

/// Coarse stage for a pre-data failure. `test_connection` only
/// returns server status codes from the EHLO/auth exchange, so a
/// present status means authentication was refused.
fn pre_data_stage(error: &lettre::transport::smtp::Error) -> &'static str {
    if error.status().is_some() {
        "auth"
    } else if error.is_tls() {
        "tls"
    } else if error.is_timeout() {
        "connect timeout"
    } else {
        "connect"
    }
}

#[cfg(all(test, feature = "greenmail"))]
mod greenmail_tests;
#[cfg(test)]
mod tests;
