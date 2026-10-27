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
//! - [`SendError::PreData`] — connect, TLS, greeting, EHLO, or a
//!   transient auth failure. No message was offered; safe to retry.
//! - [`SendError::Transient`] — the server answered `4xx`. It
//!   explicitly refused the message; safe to retry.
//! - [`SendError::Rejected`] — the server answered `5xx`, whether at
//!   greeting, auth, envelope, or DATA. Permanent; never auto-retry.
//! - [`SendError::Ambiguous`] — the connection failed after the
//!   transaction started without a final reply (timeout, reset,
//!   parse failure). The message may or may not have been accepted,
//!   so the outbox must check the Sent mailbox for the stamped
//!   Message-ID before any resend.
//!
//! A cheap [`AsyncSmtpTransport::test_connection`] probe runs before
//! submission so connect/TLS/auth failures are classified as
//! pre-data instead of ambiguous. The probe and the submission each
//! take a pooled connection: the first message may use two, and a
//! send burst then reuses the parked connections instead of
//! handshaking per message. Idle connections close after
//! [`POOL_IDLE_TIMEOUT`] (PERFORMANCE_GUIDELINES).
//!
//! ## Transport safety
//!
//! `Security::Plaintext` carries the SMTP password or OAuth bearer
//! token in the clear, so it is only accepted for loopback and
//! private-network hosts. A remote account row with
//! `smtp_security = 'plaintext'` is refused with
//! [`SendError::InsecureTransport`] before any connection is opened.
//!
//! Error details are truncated to their first line ([`DETAIL_LIMIT`]
//! characters) so a hostile server cannot smuggle a long reply —
//! including an echoed AUTH payload — into the outbox UI or logs.
//!
//! The compose domain (later M4 slices) is the first consumer;
//! until it lands these items are intentionally unused outside
//! tests, so dead-code is allowed module-wide (same precedent as
//! `mime.rs` and `mime_out.rs`).

#![allow(dead_code)]

use std::net::IpAddr;
use std::str::FromStr;
use std::time::Duration;

use lettre::address::Envelope;
use lettre::transport::smtp::PoolConfig;
use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{Address, AsyncSmtpTransport, AsyncTransport, Tokio1Executor};

use crate::domain::accounts::Security;

/// Per-command network timeout for a submission attempt (connect,
/// read, write each get this budget; it is not a total deadline).
/// A stuck server must surface as a failure the outbox can retry,
/// not a hang.
const SEND_TIMEOUT: Duration = Duration::from_secs(30);

/// Idle lifetime of a pooled submission connection
/// (PERFORMANCE_GUIDELINES: close after 30 s idle).
const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Longest server detail kept in a [`SendError`]: first line only.
const DETAIL_LIMIT: usize = 200;

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
    /// Plaintext submission to a host that is not on this machine or
    /// the local network: credentials would cross the wire in clear.
    #[error("refusing plaintext SMTP to a non-local host: {detail}")]
    InsecureTransport { detail: String },
    /// Failure before the message was offered (connect/TLS/greeting/
    /// EHLO/transient auth).
    #[error("send failed before DATA ({stage}): {detail}")]
    PreData {
        /// Coarse stage: `connect`, `tls`, `connect timeout`,
        /// `greeting`, or `pre-data`.
        stage: &'static str,
        detail: String,
    },
    /// The server answered `4xx`: refused, retry later.
    #[error("server transiently refused ({code}): {detail}")]
    Transient { code: u16, detail: String },
    /// The server answered `5xx`: permanent refusal, at any stage.
    #[error("server permanently refused ({code}): {detail}")]
    Rejected { code: u16, detail: String },
    /// No final reply: the message may have been accepted.
    #[error("ambiguous send outcome: {detail}")]
    Ambiguous { detail: String },
}

impl SendError {
    /// True when the outbox may auto-retry: the failure happened
    /// before the message could have been accepted. `InvalidEnvelope`
    /// and `InsecureTransport` are configuration errors — retrying
    /// them cannot help, even though nothing was sent.
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

/// A long-lived submission handle for one account. Holding it across
/// a send burst reuses the pooled connection (one TCP+TLS+AUTH
/// handshake for many messages); dropping it closes the pool.
pub(crate) struct SmtpSender {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    security: Security,
}

impl SmtpSender {
    /// Builds the transport for one account's security setting and
    /// credentials. Fails before any connection when the security
    /// configuration is unsafe (see [`tls_mode`]).
    pub(crate) fn new(config: &SmtpConfig, auth: &SmtpAuth) -> Result<Self, SendError> {
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

        let transport = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&config.host)
            .port(config.port)
            .tls(tls)
            .credentials(credentials)
            .authentication(mechanisms(auth))
            .timeout(Some(SEND_TIMEOUT))
            .pool_config(PoolConfig::new().idle_timeout(POOL_IDLE_TIMEOUT))
            .build();
        Ok(Self {
            transport,
            security: config.security,
        })
    }

    /// Submits one already-built RFC 5322 message (the wire bytes
    /// from [`crate::infra::mime_out`]).
    ///
    /// `wire` must be CRLF-normalized and terminated. lettre performs
    /// dot-stuffing and appends `\r\n.\r\n` itself, so the final CRLF
    /// is dropped here: the terminator restores it and the server
    /// receives exactly the built bytes. Callers must not pre-stuff
    /// dots or append the terminator.
    pub(crate) async fn send(
        &self,
        envelope: &OutgoingEnvelope,
        wire: &[u8],
    ) -> Result<(), SendError> {
        let envelope = build_envelope(envelope)?;
        let wire = wire.strip_suffix(&b"\r\n"[..]).unwrap_or(wire);

        // Phase probe: everything that can fail here happens before
        // the message is offered. A permanent 5xx (bad credentials,
        // blocked IP, greeting rejection) is not retryable; every
        // other probe failure is.
        let healthy = self
            .transport
            .test_connection()
            .await
            .map_err(|error| classify_probe_error(error, self.security))?;
        if !healthy {
            return Err(SendError::PreData {
                stage: "greeting",
                detail: "server did not accept NOOP".to_string(),
            });
        }

        self.transport
            .send_raw(&envelope, wire)
            .await
            .map(|_response| ())
            .map_err(classify_send_error)
    }
}

/// One-shot submission: builds a sender, sends, and drops the pool.
/// Prefer [`SmtpSender`] when sending more than one message.
pub(crate) async fn send(
    config: &SmtpConfig,
    auth: &SmtpAuth,
    envelope: &OutgoingEnvelope,
    wire: &[u8],
) -> Result<(), SendError> {
    SmtpSender::new(config, auth)?.send(envelope, wire).await
}

/// A probe failure means nothing was offered to the server. A
/// permanent `5xx` (auth, greeting, policy block) must never be
/// retried; a transient `4xx`, TLS, or network failure may be.
fn classify_probe_error(error: lettre::transport::smtp::Error, security: Security) -> SendError {
    match (error.is_permanent(), error.status()) {
        (true, Some(code)) => SendError::Rejected {
            code: u16::from(code),
            detail: error_detail(&error),
        },
        _ => SendError::PreData {
            stage: pre_data_stage(&error, security),
            detail: error_detail(&error),
        },
    }
}

/// Maps a submission failure: a server status means the message was
/// not accepted (`4xx` retry, `5xx` permanent); no status means the
/// outcome is unknown and the outbox must check Sent first.
fn classify_send_error(error: lettre::transport::smtp::Error) -> SendError {
    if let Some(code) = error.status() {
        let code = u16::from(code);
        return if error.is_transient() {
            SendError::Transient {
                code,
                detail: error_detail(&error),
            }
        } else {
            SendError::Rejected {
                code,
                detail: error_detail(&error),
            }
        };
    }
    SendError::Ambiguous {
        detail: error_detail(&error),
    }
}

/// First line of the server error, capped at [`DETAIL_LIMIT`] chars.
/// A hostile or chatty server cannot push an unbounded reply (or an
/// echoed credential) into the outbox UI or logs.
fn error_detail(error: &lettre::transport::smtp::Error) -> String {
    let text = error.to_string();
    let first = text.lines().next().unwrap_or_default();
    let mut detail: String = first.chars().take(DETAIL_LIMIT).collect();
    if first.chars().count() > DETAIL_LIMIT {
        detail.push('…');
    }
    detail
}

/// Maps the account's stored security level to lettre's TLS mode.
/// `Plaintext` is only valid for local relays; remote providers are
/// configured with `tls` or `starttls` (ARCHITECTURE DB schema).
fn tls_mode(security: Security, host: &str) -> Result<Tls, SendError> {
    match security {
        Security::Tls => Ok(Tls::Wrapper(tls_parameters(host)?)),
        Security::StartTls => Ok(Tls::Required(tls_parameters(host)?)),
        Security::Plaintext if is_local_host(host) => Ok(Tls::None),
        Security::Plaintext => Err(SendError::InsecureTransport {
            detail: host.to_string(),
        }),
    }
}

/// TLS parameters for the submission server. lettre's default root
/// store is webpki-roots (the `rustls-native-certs` feature is off),
/// matching the IMAP client's posture.
fn tls_parameters(host: &str) -> Result<TlsParameters, SendError> {
    TlsParameters::new(host.to_string()).map_err(|error| SendError::PreData {
        stage: "tls config",
        detail: error_detail(&error),
    })
}

/// Loopback, private, and link-local hosts only: plaintext SMTP is
/// accepted for local relays and the GreenMail test harness, never
/// for a remote provider.
fn is_local_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(address)) => {
            address.is_loopback() || address.is_private() || address.is_link_local()
        }
        Ok(IpAddr::V6(address)) => address.is_loopback() || address.is_unique_local(),
        Err(_) => false,
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

/// Coarse stage for a retryable pre-data failure. lettre exposes no
/// transaction stage, so a server status is reported neutrally
/// rather than guessed as auth (it may come from EHLO). rustls
/// handshake failures are wrapped as connection errors, so the
/// source chain is inspected to label them as TLS.
fn pre_data_stage(error: &lettre::transport::smtp::Error, security: Security) -> &'static str {
    if error.is_tls() {
        "tls"
    } else if error.is_timeout() {
        "connect timeout"
    } else if error.status().is_some() {
        "pre-data"
    } else if matches!(security, Security::Tls | Security::StartTls) && has_rustls_source(error) {
        "tls"
    } else {
        "connect"
    }
}

/// True when the error chain contains a rustls failure (handshake or
/// certificate validation). lettre wraps the rustls connector error
/// as a connection error, so [`lettre::transport::smtp::Error::is_tls`]
/// alone cannot identify it.
fn has_rustls_source(error: &lettre::transport::smtp::Error) -> bool {
    error_contains_rustls(error)
}

/// Walks an error chain, including the inner error of an
/// [`std::io::Error`] (`source` skips it, `get_ref` exposes it).
fn error_contains_rustls(error: &(dyn std::error::Error + 'static)) -> bool {
    if error.downcast_ref::<rustls::Error>().is_some() {
        return true;
    }
    let inner_is_rustls = error
        .downcast_ref::<std::io::Error>()
        .and_then(std::io::Error::get_ref)
        .is_some_and(|inner| error_contains_rustls(inner));
    if inner_is_rustls {
        return true;
    }
    error.source().is_some_and(error_contains_rustls)
}

#[cfg(all(test, feature = "greenmail"))]
mod greenmail_tests;
#[cfg(test)]
mod tests;
