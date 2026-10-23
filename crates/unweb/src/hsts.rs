//! The self-test's HSTS servers.
//!
//! A site that sends `Strict-Transport-Security` over HTTPS tells the
//! browser to use HTTPS for that host from then on. A browser that keeps
//! this record across strangers can be recognised: the site sees whether
//! a plain `http://` visit arrives as HTTPS. Here two servers share the
//! host hsts.unweb.test (pointed at 127.0.0.1 by scripts/test-ca.sh):
//!
//! - an HTTPS server that sends the HSTS header on every answer;
//! - a plain HTTP server on another port. An engine that remembers HSTS
//!   for the host upgrades `http://hsts.unweb.test:<port>/` to HTTPS on the
//!   same port, so this server then receives a TLS handshake instead of an
//!   HTTP request. It notes which one it got.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

pub const HOST: &str = "hsts.unweb.test";

/// What a server received.
#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub enum Seen {
    /// An HTTPS request (on the HTTPS server), with its path and query.
    Https(String),
    /// A plain HTTP request on the plain server, with its path and query.
    Plain(String),
    /// A TLS handshake on the plain server: the engine upgraded the visit.
    Upgraded,
}

#[derive(Clone)]
pub struct Hsts {
    pub https_port: u16,
    pub plain_port: u16,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Hsts {
    /// Starts both servers with the certificate made by
    /// scripts/test-ca.sh in `dir` (chain.pem and leaf.key).
    pub fn start(dir: &Path) -> Result<Hsts, String> {
        let certs: Vec<CertificateDer<'static>> =
            CertificateDer::pem_file_iter(dir.join("chain.pem"))
                .map_err(|e| format!("chain.pem: {e}"))?
                .collect::<Result<_, _>>()
                .map_err(|e| format!("chain.pem: {e}"))?;
        let key = PrivateKeyDer::from_pem_file(dir.join("leaf.key"))
            .map_err(|e| format!("leaf.key: {e}"))?;
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .map_err(|e| e.to_string())?;
        let config = Arc::new(config);
        let seen = Arc::new(Mutex::new(Vec::new()));

        let https = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
        let https_port = https.local_addr().map_err(|e| e.to_string())?.port();
        let s = seen.clone();
        std::thread::spawn(move || {
            for stream in https.incoming().flatten() {
                let (s, config) = (s.clone(), config.clone());
                std::thread::spawn(move || {
                    let _ = serve_https(stream, config, &s);
                });
            }
        });

        let plain = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
        let plain_port = plain.local_addr().map_err(|e| e.to_string())?.port();
        let s = seen.clone();
        std::thread::spawn(move || {
            for stream in plain.incoming().flatten() {
                let s = s.clone();
                std::thread::spawn(move || {
                    let _ = serve_plain(stream, &s);
                });
            }
        });
        Ok(Hsts {
            https_port,
            plain_port,
            seen,
        })
    }

    pub fn https_url(&self, path: &str) -> String {
        format!("https://{HOST}:{}{path}", self.https_port)
    }

    pub fn plain_url(&self, path: &str) -> String {
        format!("http://{HOST}:{}{path}", self.plain_port)
    }

    pub fn seen(&self) -> Vec<Seen> {
        self.seen.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

fn request_target(reader: &mut impl BufRead) -> std::io::Result<String> {
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let target = line.split_whitespace().nth(1).unwrap_or("/").to_string();
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h)? == 0 || h == "\r\n" || h == "\n" {
            break;
        }
    }
    Ok(target)
}

const PAGE: &str = "<!doctype html><title>hsts</title><p>hsts</p>";

fn serve_https(
    stream: TcpStream,
    config: Arc<rustls::ServerConfig>,
    seen: &Mutex<Vec<Seen>>,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let conn = rustls::ServerConnection::new(config).map_err(std::io::Error::other)?;
    let mut tls = rustls::StreamOwned::new(conn, stream);
    let target = {
        let mut reader = BufReader::new(&mut tls);
        request_target(&mut reader)?
    };
    if let Ok(mut s) = seen.lock() {
        s.push(Seen::Https(target));
    }
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nStrict-Transport-Security: max-age=3600\r\nConnection: close\r\n\r\n",
        PAGE.len()
    );
    tls.write_all(head.as_bytes())?;
    tls.write_all(PAGE.as_bytes())?;
    tls.flush()?;
    tls.conn.send_close_notify();
    let _ = tls.flush();
    Ok(())
}

fn serve_plain(mut stream: TcpStream, seen: &Mutex<Vec<Seen>>) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut first = [0u8; 1];
    let n = stream.peek(&mut first)?;
    if n == 1 && first[0] == 0x16 {
        // A TLS handshake record: the engine came with HTTPS.
        if let Ok(mut s) = seen.lock() {
            s.push(Seen::Upgraded);
        }
        let mut sink = [0u8; 512];
        let _ = stream.read(&mut sink);
        return Ok(());
    }
    let target = {
        let mut reader = BufReader::new(stream.try_clone()?);
        request_target(&mut reader)?
    };
    if let Ok(mut s) = seen.lock() {
        s.push(Seen::Plain(target));
    }
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        PAGE.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(PAGE.as_bytes())?;
    stream.flush()
}
