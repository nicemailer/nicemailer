//! The connection under the session: TCP, TLS, and the time limit on
//! opening it.
//!
//! # Certificates
//!
//! The server's certificate is checked by macOS itself
//! (`rustls-platform-verifier`, which asks the Security framework): the
//! same trust as Mail and Safari, authorities the user added included, and
//! the system's revocation checks. `ca_file` adds one authority on top of
//! those (`configuration.md`, `imap.ca_file`). There is no way to turn the
//! check off: a mail client that accepts any certificate gives the password
//! to whoever sits on the network.

use std::io;
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, ServerName};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;
use tokio_rustls::rustls::ClientConfig;

use super::error::{ImapError, Result};
use super::types::{Security, ServerSpec, Timeouts};

/// Any byte stream a session can run over. Tests use an in-memory pipe
/// (`tokio::io::duplex`) with a scripted server.
///
/// Rust note: a trait with no method of its own, implemented for every
/// type that has the others (`impl<T: …> Stream for T`), names a
/// combination of traits, so that `Box<dyn Stream>` can hold any of them.
pub trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}

/// The stream of a session. STARTTLS needs the TCP stream itself to wrap
/// it in TLS, hence the variants rather than a single `Box<dyn Stream>`.
pub(crate) enum Conn {
    Plain(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
    Other(Box<dyn Stream>),
}

impl Conn {
    pub fn is_tls(&self) -> bool {
        matches!(self, Conn::Tls(_))
    }
}

// Rust note: `AsyncRead` and `AsyncWrite` are *poll* interfaces: the
// runtime calls `poll_read`, which either has bytes (`Ready`) or registers
// the task to be woken when there are (`Pending`). `Pin` guarantees the
// value does not move while polled; the streams here are `Unpin` (moving
// them is harmless), so `Pin::new` is enough to forward each call.
impl AsyncRead for Conn {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Conn::Plain(s) => Pin::new(s).poll_read(cx, buf),
            Conn::Tls(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
            Conn::Other(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for Conn {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Conn::Plain(s) => Pin::new(s).poll_write(cx, buf),
            Conn::Tls(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
            Conn::Other(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Conn::Plain(s) => Pin::new(s).poll_flush(cx),
            Conn::Tls(s) => Pin::new(s.as_mut()).poll_flush(cx),
            Conn::Other(s) => Pin::new(s.as_mut()).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Conn::Plain(s) => Pin::new(s).poll_shutdown(cx),
            Conn::Tls(s) => Pin::new(s.as_mut()).poll_shutdown(cx),
            Conn::Other(s) => Pin::new(s.as_mut()).poll_shutdown(cx),
        }
    }
}

/// Whether `host` is this machine: the only place clear text is allowed.
pub(crate) fn is_loopback(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost") || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// Opens the TCP connection, and TLS on it for [`Security::Tls`].
pub(crate) async fn open(spec: &ServerSpec, timeouts: &Timeouts) -> Result<Conn> {
    if spec.security == Security::None && !is_loopback(&spec.host) {
        // config.toml already refuses it; checked again here, since a
        // password must never travel in clear text over a network.
        return Err(ImapError::Tls(format!("clear text is only allowed to this machine, not to {}", spec.host)));
    }
    let connector = match spec.security {
        Security::None => None,
        _ => Some(connector(spec)?),
    };
    let what = format!("{}:{}", spec.host, spec.port);
    let tcp = tokio::time::timeout(timeouts.connect, TcpStream::connect((spec.host.as_str(), spec.port)))
        .await
        .map_err(|_| ImapError::Timeout("connecting"))?
        .map_err(|e| ImapError::Connect(format!("{what}: {e}")))?;
    // IMAP is a conversation of short lines: send each one at once.
    let _ = tcp.set_nodelay(true);
    match (spec.security, connector) {
        (Security::Tls, Some(c)) => Ok(Conn::Tls(Box::new(handshake(&c, spec, tcp, timeouts).await?))),
        _ => Ok(Conn::Plain(tcp)),
    }
}

/// The TLS client of an account: the system's trust, plus `ca_file`.
pub(crate) fn connector(spec: &ServerSpec) -> Result<TlsConnector> {
    // Rust note: `Arc` is a shared pointer counted across threads: the
    // provider (the cryptography, here `ring`) is shared by the verifier
    // and the configuration.
    let provider = Arc::new(tokio_rustls::rustls::crypto::ring::default_provider());
    let verifier = match &spec.ca_file {
        None => rustls_platform_verifier::Verifier::new(provider.clone()),
        Some(path) => {
            let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(path)
                .and_then(|it| it.collect::<std::result::Result<Vec<_>, _>>())
                .map_err(|e| ImapError::Tls(format!("ca_file {}: {e}", path.display())))?;
            if certs.is_empty() {
                return Err(ImapError::Tls(format!("ca_file {}: no certificate in the file", path.display())));
            }
            rustls_platform_verifier::Verifier::new_with_extra_roots(certs, provider.clone())
        }
    }
    .map_err(|e| ImapError::Tls(e.to_string()))?;
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| ImapError::Tls(e.to_string()))?
        // "dangerous" is rustls's name for any verifier other than its own;
        // this one is the system's, which checks more than the default.
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    Ok(TlsConnector::from(Arc::new(config)))
}

/// The TLS handshake on an open TCP stream, the certificate checked for
/// `spec.host`.
pub(crate) async fn handshake(c: &TlsConnector, spec: &ServerSpec, tcp: TcpStream, timeouts: &Timeouts) -> Result<TlsStream<TcpStream>> {
    let name = ServerName::try_from(spec.host.clone()).map_err(|_| ImapError::Tls(format!("invalid host name {}", spec.host)))?;
    tokio::time::timeout(timeouts.connect, c.connect(name, tcp))
        .await
        .map_err(|_| ImapError::Timeout("TLS handshake"))?
        .map_err(|e| ImapError::Tls(tls_message(&e)))
}

/// A readable message for a failed handshake: rustls wraps its errors in
/// an `io::Error`.
fn tls_message(e: &io::Error) -> String {
    match e.get_ref() {
        Some(inner) => inner.to_string(),
        None => e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_only() {
        assert!(is_loopback("localhost"));
        assert!(is_loopback("LOCALHOST"));
        assert!(is_loopback("127.0.0.1"));
        assert!(is_loopback("127.2.3.4"));
        assert!(is_loopback("::1"));
        assert!(!is_loopback("192.168.1.10"));
        assert!(!is_loopback("localhost.example.com"));
        assert!(!is_loopback("mail.infomaniak.com"));
    }

    #[tokio::test]
    async fn clear_text_to_a_remote_host_is_refused_before_connecting() {
        let spec = ServerSpec { host: "192.0.2.1".into(), port: 143, security: Security::None, ca_file: None };
        let Err(e) = open(&spec, &Timeouts::default()).await else { panic!("accepted") };
        assert!(e.to_string().contains("clear text"), "{e}");
    }

    #[test]
    fn a_missing_or_empty_ca_file_is_an_error() {
        let dir = std::env::temp_dir().join(format!("nm-ca-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let empty = dir.join("empty.pem");
        std::fs::write(&empty, "").unwrap();
        for path in [dir.join("missing.pem"), empty] {
            let spec = ServerSpec { host: "localhost".into(), port: 993, security: Security::Tls, ca_file: Some(path) };
            let Err(e) = connector(&spec) else { panic!("accepted") };
            assert!(e.to_string().contains("ca_file"), "{e}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
