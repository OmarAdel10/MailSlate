//! OAuth loopback receiver: catches the browser redirect.
//!
//! The daemon binds a tiny HTTP listener on an ephemeral 127.0.0.1
//! port, the GUI opens the browser at the provider's authorize URL
//! with `redirect_uri=http://127.0.0.1:{port}/callback`, and the
//! provider redirects back with `code` and `state` query parameters.
//! The listener validates `state` against the value it issued and
//! hands back the authorization `code` (PKCE exchange, ARCHITECTURE
//! §8). Binds are local-only; the listener never serves content.

use super::OAuthError;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

/// A bound loopback receiver that catches the provider redirect.
///
/// Clone a waiter handle (`for_waiter`) before handing the blocking
/// wait to a dedicated thread or `tokio::task::spawn_blocking`; the
/// receiver itself can serve several sequential waits.
pub struct LoopbackListener {
    server: Arc<tiny_http::Server>,
}

impl LoopbackListener {
    /// Binds on an ephemeral 127.0.0.1 port (loopback only).
    pub fn bind() -> std::io::Result<Self> {
        let server = tiny_http::Server::http("127.0.0.1:0")
            .map_err(|err| std::io::Error::other(err.to_string()))?;
        Ok(Self {
            server: Arc::new(server),
        })
    }

    /// The port clients must redirect to.
    pub fn port(&self) -> u16 {
        match self.server.server_addr() {
            tiny_http::ListenAddr::IP(addr) => addr.port(),
            #[allow(unreachable_patterns)]
            _ => 0,
        }
    }

    /// A `Send + 'static` handle for blocking waits off-thread.
    pub fn for_waiter(&self) -> Self {
        Self {
            server: Arc::clone(&self.server),
        }
    }

    /// Blocks until a redirect with a matching `state` arrives, then
    /// returns `(code, state)`. Call from a blocking context, never
    /// inside an async poll. On deadline the recv thread stays blocked
    /// until the next request or process exit (one thread per
    /// abandoned OAuth start; acceptable for a manual browser flow).
    pub fn wait_for_code(
        &self,
        state: &str,
        deadline: Duration,
    ) -> Result<(String, String), OAuthError> {
        let (tx, rx) = mpsc::channel();
        {
            let server = Arc::clone(&self.server);
            let state = state.to_string();
            std::thread::spawn(move || recv_one(server, &state, tx));
        }
        match rx.recv_timeout(deadline) {
            Ok(result) => result,
            Err(_) => Err(OAuthError::ListenerTimeout),
        }
    }
}

/// Serves requests until a callback resolves the flow.
fn recv_one(
    server: Arc<tiny_http::Server>,
    state: &str,
    tx: mpsc::Sender<Result<(String, String), OAuthError>>,
) {
    let result = loop {
        let request = match server.recv() {
            Ok(request) => request,
            Err(err) => break Err(OAuthError::Network(err.to_string())),
        };
        let url = request.url().to_string();
        let (code, got_state) = match parse_query(&url) {
            Some(pair) => pair,
            // Not a callback (favicon, probe): keep waiting.
            None => {
                let response = tiny_http::Response::from_string("not a callback");
                let _ = request.respond(response);
                continue;
            }
        };
        if got_state != state {
            let response = tiny_http::Response::from_string("state mismatch").with_status_code(400);
            let _ = request.respond(response);
            break Err(OAuthError::StateMismatch);
        }
        let response = tiny_http::Response::from_string("sign-in complete");
        let _ = request.respond(response);
        break Ok((code, got_state));
    };
    let _ = tx.send(result);
}

/// Extracts `(code, state)` from a `/callback` request's query
/// string, percent-decoding both values, or `None` when the path is
/// not `/callback` or either parameter is missing.
fn parse_query(url: &str) -> Option<(String, String)> {
    let (path, query) = url.split_once('?')?;
    if path != "/callback" {
        return None;
    }
    let mut code = None;
    let mut state = None;
    for (key, value) in form_urlencoded::parse(query.as_bytes()) {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            _ => {}
        }
    }
    Some((code?, state?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::time::Duration;

    /// Query values must be percent-decoded before use (providers may
    /// encode punctuation in codes and states).
    #[test]
    fn parse_query_percent_decodes_values() {
        let (code, state) =
            parse_query("/callback?code=cd%209%2Fx&state=st%2D1").expect("decoded pair");
        assert_eq!(code, "cd 9/x");
        assert_eq!(state, "st-1");
    }

    /// Only `/callback` resolves the flow; anything else keeps waiting
    /// (defensive path check against probes hitting the port).
    #[test]
    fn parse_query_rejects_paths_other_than_callback() {
        assert!(parse_query("/evil?code=a&state=b").is_none());
        assert!(parse_query("/?code=a&state=b").is_none());
        assert!(parse_query("/callback?code=a&state=b").is_some());
    }

    /// A successful redirect hands back both parameters.
    #[test]
    fn loopback_returns_code_for_the_issued_state() {
        let listener = LoopbackListener::bind().expect("bind loopback");
        assert!(listener.port() > 0, "ephemeral port must be assigned");

        let waiter = {
            let listener = listener.for_waiter();
            std::thread::spawn(move || listener.wait_for_code("st-1", Duration::from_secs(10)))
        };

        let url = format!(
            "http://127.0.0.1:{}/callback?code=cd-9&state=st-1",
            listener.port()
        );
        http_get(&url).expect("GET callback");

        let (code, state) = waiter.join().expect("waiter thread").expect("code");
        assert_eq!(code, "cd-9");
        assert_eq!(state, "st-1");
    }

    /// A forged redirect (wrong state) is rejected, not trusted.
    #[test]
    fn state_mismatch_is_rejected() {
        let listener = LoopbackListener::bind().expect("bind loopback");

        let waiter = {
            let listener = listener.for_waiter();
            std::thread::spawn(move || listener.wait_for_code("expected", Duration::from_secs(10)))
        };

        let url = format!(
            "http://127.0.0.1:{}/callback?code=cd-9&state=other",
            listener.port()
        );
        http_get(&url).expect("GET callback");

        let result = waiter.join().expect("waiter thread");
        match result {
            Err(OAuthError::StateMismatch) => {}
            other => panic!("expected StateMismatch, got {other:?}"),
        }
    }

    /// A callback missing `code` or `state` is not a real redirect:
    /// the listener answers and keeps waiting for the genuine one.
    #[test]
    fn incomplete_callbacks_keep_the_listener_waiting() {
        let listener = LoopbackListener::bind().expect("bind loopback");

        let waiter = {
            let listener = listener.for_waiter();
            std::thread::spawn(move || listener.wait_for_code("st-1", Duration::from_secs(10)))
        };

        let codeless = format!("http://127.0.0.1:{}/callback?state=st-1", listener.port());
        http_get(&codeless).expect("GET callback without code");
        let stateless = format!("http://127.0.0.1:{}/callback?code=cd-9", listener.port());
        http_get(&stateless).expect("GET callback without state");

        let url = format!(
            "http://127.0.0.1:{}/callback?code=cd-9&state=st-1",
            listener.port()
        );
        http_get(&url).expect("GET callback");

        let (code, state) = waiter.join().expect("waiter thread").expect("code");
        assert_eq!(code, "cd-9");
        assert_eq!(state, "st-1");
    }

    /// No callback within the deadline fails the flow.
    #[test]
    fn listener_times_out_without_a_callback() {
        let listener = LoopbackListener::bind().expect("bind loopback");
        let waiter = listener.for_waiter();

        let started = std::time::Instant::now();
        let result = waiter.wait_for_code("st-1", Duration::from_millis(300));
        assert!(
            started.elapsed() < Duration::from_millis(800),
            "timeout must fire near the deadline"
        );
        match result {
            Err(OAuthError::ListenerTimeout) => {}
            other => panic!("expected ListenerTimeout, got {other:?}"),
        }
    }

    /// Minimal HTTP/1.1 GET so the tiny_http server has a request to
    /// receive (std TcpStream; no client dependency).
    fn http_get(url: &str) -> std::io::Result<()> {
        let authority = url
            .trim_start_matches("http://")
            .split('/')
            .next()
            .unwrap_or_default()
            .to_string();
        let path = format!("/{}", url.splitn(4, '/').nth(3).unwrap_or_default());
        let mut stream = std::net::TcpStream::connect(&authority)?;
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n"
        )?;
        let mut scratch = [0u8; 512];
        let _ = stream.read(&mut scratch)?;
        Ok(())
    }
}
