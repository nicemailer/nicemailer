//! One-click unsubscribe (RFC 8058): a single POST of
//! `List-Unsubscribe=One-Click` to the https URL of the `List-Unsubscribe`
//! header, when the mail also has `List-Unsubscribe-Post`. No browser, no
//! page to read: the server is expected to unsubscribe the address at once.
//!
//! The request is deliberately plain: no cookie, no credential, the same
//! anonymous user agent for everyone, a time limit, and only https (RFC 8058
//! requires it; tests on a local server lift it).

use std::time::Duration;

/// Why a one-click unsubscribe failed. The messages reach the user, in
/// French like the core's other errors.
#[derive(Debug, thiserror::Error)]
pub enum UnsubscribeError {
    #[error("le serveur de désabonnement a répondu {0}")]
    Status(u16),
    #[error("désabonnement impossible : {0}")]
    Http(String),
}

/// How long the whole request may take, redirections included.
const TIMEOUT: Duration = Duration::from_secs(15);

/// Sends the one-click unsubscribe request to `url`. `https_only` is false
/// only in tests. Blocking: call it off the async runtime.
///
/// Rust note: `Result<(), E>` returns nothing on success (`()`, the "unit"
/// type), only an error on failure.
pub fn one_click(url: &str, https_only: bool) -> Result<(), UnsubscribeError> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .https_only(https_only)
        .max_redirects(5)
        .user_agent("NiceMailer")
        // A 4xx or 5xx answer is a result to report, not a transport error.
        .http_status_as_error(false)
        .build()
        .into();
    let response = agent
        .post(url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .send("List-Unsubscribe=One-Click")
        .map_err(|e| UnsubscribeError::Http(e.to_string()))?;
    let status = response.status().as_u16();
    // 2xx: done (200, 202 Accepted, 204 No Content).
    if (200..300).contains(&status) { Ok(()) } else { Err(UnsubscribeError::Status(status)) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    /// A one-request HTTP server on localhost answering `status`; sends back
    /// the request line, headers and body it received.
    fn server(status: u16) -> (String, mpsc::Receiver<(String, String)>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/u?id=7", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut head = String::new();
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap();
                }
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                head.push_str(&line);
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let mut stream = stream;
            write!(stream, "HTTP/1.1 {status} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            tx.send((head, String::from_utf8(body).unwrap())).unwrap();
        });
        (url, rx)
    }

    #[test]
    fn posts_the_one_click_body_without_credentials() {
        let (url, rx) = server(200);
        one_click(&url, false).unwrap();
        let (head, body) = rx.recv().unwrap();
        assert!(head.starts_with("POST /u?id=7 HTTP/1.1"), "{head}");
        let lower = head.to_ascii_lowercase();
        assert!(lower.contains("content-type: application/x-www-form-urlencoded"));
        assert!(lower.contains("user-agent: nicemailer"));
        assert!(!lower.contains("cookie") && !lower.contains("authorization"));
        assert_eq!(body, "List-Unsubscribe=One-Click");
    }

    #[test]
    fn an_error_status_is_reported() {
        let (url, _rx) = server(404);
        assert!(matches!(one_click(&url, false), Err(UnsubscribeError::Status(404))));
        let (url, _rx) = server(202);
        assert!(one_click(&url, false).is_ok());
    }

    #[test]
    fn plain_http_is_refused_outside_tests() {
        // Refused before any connection is made.
        assert!(matches!(one_click("http://127.0.0.1:9/u", true), Err(UnsubscribeError::Http(_))));
        assert!(matches!(one_click("not a url", false), Err(UnsubscribeError::Http(_))));
    }
}
