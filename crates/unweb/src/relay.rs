//! A stranger's relay to its proxy, for macOS.
//!
//! macOS lets a connection go direct when its proxy can't be reached, even
//! with fallback turned off in the proxy configuration (the self-test showed
//! it). So on macOS a stranger with a location is given this relay as its
//! proxy instead: a small HTTP proxy on 127.0.0.1, one per stranger, that
//! forwards every request to the stranger's real proxy (HTTP or SOCKS5).
//! The relay is always there, so macOS has nothing to fall back from; when
//! the real proxy can't be reached, the relay answers with an error and the
//! page gets nothing.
//!
//! The relay only takes proxy requests (CONNECT, or a request for a full
//! http:// address), so a web page pointed at its port gets nothing from it.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tauri::Url;

const CONNECT_WITHIN: Duration = Duration::from_secs(15);
const MAX_HEAD: usize = 64 * 1024;

/// A running relay. Dropping it stops it.
pub struct Relay {
    port: u16,
    stop: Arc<AtomicBool>,
}

impl Relay {
    /// Starts a relay to `upstream` (an http:// or socks5:// proxy that has
    /// passed location::parse_location).
    pub fn start(upstream: &Url) -> std::io::Result<Relay> {
        let up =
            Upstream::of(upstream).ok_or_else(|| std::io::Error::other("not a proxy address"))?;
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        std::thread::spawn(move || {
            for client in listener.incoming() {
                if stopped.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(client) = client else { continue };
                let up = up.clone();
                std::thread::spawn(move || {
                    let _ = handle(client, &up);
                });
            }
        });
        Ok(Relay { port, stop })
    }

    /// The relay's own address, to give the engine as the stranger's proxy.
    pub fn url(&self) -> Url {
        Url::parse(&format!("http://127.0.0.1:{}", self.port)).expect("relay address")
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the listener so it sees the flag.
        let _ = TcpStream::connect_timeout(
            &([127, 0, 0, 1], self.port).into(),
            Duration::from_millis(200),
        );
    }
}

#[derive(Clone)]
struct Upstream {
    socks: bool,
    host: String,
    port: u16,
}

impl Upstream {
    fn of(url: &Url) -> Option<Upstream> {
        Some(Upstream {
            socks: url.scheme() == "socks5",
            host: url.host_str()?.to_string(),
            port: url.port()?,
        })
    }

    fn connect(&self) -> std::io::Result<TcpStream> {
        let addr = (self.host.as_str(), self.port)
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| std::io::Error::other("the proxy's name didn't resolve"))?;
        let s = TcpStream::connect_timeout(&addr, CONNECT_WITHIN)?;
        s.set_read_timeout(Some(CONNECT_WITHIN))?;
        Ok(s)
    }

    /// A connection through the proxy to `host:port`, ready for the
    /// client's bytes.
    fn tunnel(&self, host: &str, port: u16) -> std::io::Result<TcpStream> {
        let mut s = self.connect()?;
        if self.socks {
            socks5_connect(&mut s, host, port)?;
        } else {
            let authority = if host.contains(':') {
                format!("[{host}]:{port}")
            } else {
                format!("{host}:{port}")
            };
            write!(
                s,
                "CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n\r\n"
            )?;
            let mut reader = BufReader::new(s.try_clone()?);
            let mut status = String::new();
            reader.read_line(&mut status)?;
            if status.split_whitespace().nth(1) != Some("200") {
                return Err(std::io::Error::other("the proxy refused"));
            }
            loop {
                let mut h = String::new();
                if reader.read_line(&mut h)? == 0 || h == "\r\n" || h == "\n" {
                    break;
                }
            }
        }
        s.set_read_timeout(None)?;
        Ok(s)
    }
}

fn socks5_connect(s: &mut TcpStream, host: &str, port: u16) -> std::io::Result<()> {
    s.write_all(&[5, 1, 0])?;
    let mut b = [0u8; 2];
    s.read_exact(&mut b)?;
    if b != [5, 0] {
        return Err(std::io::Error::other("the proxy wants a sign-in"));
    }
    let mut req = vec![5, 1, 0];
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => {
            req.push(1);
            req.extend_from_slice(&ip.octets());
        }
        Ok(std::net::IpAddr::V6(ip)) => {
            req.push(4);
            req.extend_from_slice(&ip.octets());
        }
        // Names go to the proxy to look up.
        Err(_) => {
            let name = host.as_bytes();
            if name.is_empty() || name.len() > 255 {
                return Err(std::io::Error::other("bad name"));
            }
            req.push(3);
            req.push(name.len() as u8);
            req.extend_from_slice(name);
        }
    }
    req.extend_from_slice(&port.to_be_bytes());
    s.write_all(&req)?;
    let mut head = [0u8; 4];
    s.read_exact(&mut head)?;
    if head[1] != 0 {
        return Err(std::io::Error::other("the proxy couldn't connect"));
    }
    // The bound address, which isn't needed.
    let skip = match head[3] {
        1 => 4,
        4 => 16,
        3 => {
            let mut n = [0u8; 1];
            s.read_exact(&mut n)?;
            n[0] as usize
        }
        _ => return Err(std::io::Error::other("bad answer")),
    };
    let mut rest = vec![0u8; skip + 2];
    s.read_exact(&mut rest)?;
    Ok(())
}

/// `host:port`, `[v6]:port` or `host` (with `default`).
fn host_port(s: &str, default: u16) -> Option<(String, u16)> {
    if let Some(rest) = s.strip_prefix('[') {
        let (h, after) = rest.split_once(']')?;
        let port = match after.strip_prefix(':') {
            Some(p) => p.parse().ok()?,
            None if after.is_empty() => default,
            None => return None,
        };
        return Some((h.to_string(), port));
    }
    match s.rsplit_once(':') {
        Some((h, p)) => Some((h.to_string(), p.parse().ok()?)),
        None => Some((s.to_string(), default)),
    }
}

fn handle(client: TcpStream, up: &Upstream) -> std::io::Result<()> {
    client.set_read_timeout(Some(CONNECT_WITHIN))?;
    let mut reader = BufReader::new(client.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();
    let mut headers = Vec::new();
    let mut size = line.len();
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h)? == 0 || h == "\r\n" || h == "\n" {
            break;
        }
        size += h.len();
        if size > MAX_HEAD {
            return Ok(());
        }
        headers.push(h);
    }
    let mut out = client.try_clone()?;
    let refuse = |out: &mut TcpStream, status: &str| {
        let _ = write!(
            out,
            "HTTP/1.1 {status}\r\nContent-Type: text/plain\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
    };
    if method.eq_ignore_ascii_case("CONNECT") {
        let Some((host, port)) = host_port(&target, 443) else {
            refuse(&mut out, "400 Bad Request");
            return Ok(());
        };
        let up_stream = match up.tunnel(&host, port) {
            Ok(s) => s,
            Err(_) => {
                refuse(&mut out, "502 Bad Gateway");
                return Ok(());
            }
        };
        out.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")?;
        let early = reader.buffer().to_vec();
        let mut w = up_stream.try_clone()?;
        w.write_all(&early)?;
        client.set_read_timeout(None)?;
        pipe(client, up_stream);
        return Ok(());
    }
    // GET http://host:port/path HTTP/1.1
    let Some(rest) = target.strip_prefix("http://") else {
        refuse(&mut out, "400 Bad Request");
        return Ok(());
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let Some((host, port)) = host_port(authority, 80) else {
        refuse(&mut out, "400 Bad Request");
        return Ok(());
    };
    // One request per connection: the answer, then both close.
    let mut head = String::new();
    let first = if up.socks {
        format!("{method} {path} HTTP/1.1\r\n")
    } else {
        // An HTTP proxy takes the full address.
        format!("{method} {target} HTTP/1.1\r\n")
    };
    head.push_str(&first);
    for h in &headers {
        let lower = h.to_ascii_lowercase();
        if lower.starts_with("proxy-connection:") || lower.starts_with("connection:") {
            continue;
        }
        head.push_str(h);
    }
    head.push_str("Connection: close\r\n\r\n");
    let up_stream = if up.socks {
        up.tunnel(&host, port)
    } else {
        up.connect().and_then(|s| {
            s.set_read_timeout(None)?;
            Ok(s)
        })
    };
    let mut up_stream = match up_stream {
        Ok(s) => s,
        Err(_) => {
            refuse(&mut out, "502 Bad Gateway");
            return Ok(());
        }
    };
    up_stream.write_all(head.as_bytes())?;
    let early = reader.buffer().to_vec();
    up_stream.write_all(&early)?;
    // A request body still coming, then the answer back.
    let body_from = client.try_clone()?;
    let mut body_to = up_stream.try_clone()?;
    client.set_read_timeout(None)?;
    std::thread::spawn(move || {
        let mut body_from = body_from;
        let _ = std::io::copy(&mut body_from, &mut body_to);
    });
    let mut up_r = up_stream.try_clone()?;
    let _ = std::io::copy(&mut up_r, &mut out);
    let _ = out.shutdown(std::net::Shutdown::Both);
    let _ = up_stream.shutdown(std::net::Shutdown::Both);
    Ok(())
}

/// Copies both ways until either side is done.
fn pipe(a: TcpStream, b: TcpStream) {
    let (mut a2, mut b2) = match (a.try_clone(), b.try_clone()) {
        (Ok(x), Ok(y)) => (x, y),
        _ => return,
    };
    let t = std::thread::spawn(move || {
        let mut a = a;
        let mut b = b;
        let _ = std::io::copy(&mut a, &mut b);
        let _ = b.shutdown(std::net::Shutdown::Write);
    });
    let _ = std::io::copy(&mut b2, &mut a2);
    let _ = a2.shutdown(std::net::Shutdown::Write);
    let _ = t.join();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testproxy::{closed_port, Proxy};

    /// A one-request site that answers "ok" and logs where requests came
    /// from.
    fn site() -> (u16, Arc<std::sync::Mutex<Vec<u16>>>) {
        let l = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = l.local_addr().unwrap().port();
        let peers = Arc::new(std::sync::Mutex::new(Vec::new()));
        let p = peers.clone();
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                p.lock().unwrap().push(s.peer_addr().unwrap().port());
                let mut r = BufReader::new(s.try_clone().unwrap());
                let mut line = String::new();
                while r.read_line(&mut line).is_ok() && line != "\r\n" && !line.is_empty() {
                    line.clear();
                }
                let mut s = s;
                let _ = s.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                );
            }
        });
        (port, peers)
    }

    fn get(relay: &Relay, url: &str) -> String {
        let mut c = TcpStream::connect(("127.0.0.1", relay.port)).unwrap();
        write!(
            c,
            "GET {url} HTTP/1.1\r\nHost: x\r\nProxy-Connection: keep-alive\r\n\r\n"
        )
        .unwrap();
        let mut out = String::new();
        let _ = c.read_to_string(&mut out);
        out
    }

    fn connect(relay: &Relay, authority: &str) -> String {
        let mut c = TcpStream::connect(("127.0.0.1", relay.port)).unwrap();
        write!(
            c,
            "CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n\r\n"
        )
        .unwrap();
        let mut r = BufReader::new(c.try_clone().unwrap());
        let mut status = String::new();
        r.read_line(&mut status).unwrap();
        if !status.contains(" 200 ") {
            return status;
        }
        loop {
            let mut h = String::new();
            if r.read_line(&mut h).unwrap() == 0 || h == "\r\n" {
                break;
            }
        }
        c.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
        let mut out = String::new();
        let _ = r.read_to_string(&mut out);
        out
    }

    #[test]
    fn through_an_http_proxy() {
        let (port, peers) = site();
        let proxy = Proxy::http().unwrap();
        let relay = Relay::start(&Url::parse(&format!("http://127.0.0.1:{}", proxy.port)).unwrap())
            .unwrap();
        assert!(get(&relay, &format!("http://far.unweb.test:{port}/x")).ends_with("ok"));
        assert!(connect(&relay, &format!("far.unweb.test:{port}")).ends_with("ok"));
        assert_eq!(proxy.onward().len(), 2);
        assert_eq!(*peers.lock().unwrap(), proxy.onward_ports());
    }

    #[test]
    fn through_a_socks5_proxy_with_names() {
        let (port, peers) = site();
        let proxy = Proxy::socks5().unwrap();
        let relay =
            Relay::start(&Url::parse(&format!("socks5://127.0.0.1:{}", proxy.port)).unwrap())
                .unwrap();
        assert!(get(&relay, &format!("http://far.unweb.test:{port}/x")).ends_with("ok"));
        assert!(connect(&relay, &format!("far.unweb.test:{port}")).ends_with("ok"));
        assert!(proxy.onward().iter().all(|o| o.how == "socks-name"));
        assert_eq!(*peers.lock().unwrap(), proxy.onward_ports());
    }

    #[test]
    fn a_proxy_that_is_down_gets_an_error_never_a_direct_connection() {
        let (port, peers) = site();
        for scheme in ["http", "socks5"] {
            let relay = Relay::start(
                &Url::parse(&format!("{scheme}://127.0.0.1:{}", closed_port())).unwrap(),
            )
            .unwrap();
            assert!(get(&relay, &format!("http://127.0.0.1:{port}/x")).starts_with("HTTP/1.1 502"));
            assert!(connect(&relay, &format!("127.0.0.1:{port}")).contains(" 502 "));
        }
        assert!(peers.lock().unwrap().is_empty());
    }

    #[test]
    fn only_proxy_requests() {
        let proxy = Proxy::http().unwrap();
        let relay = Relay::start(&Url::parse(&format!("http://127.0.0.1:{}", proxy.port)).unwrap())
            .unwrap();
        // What a web page pointed at the relay would send.
        assert!(get(&relay, "/x").starts_with("HTTP/1.1 400"));
        assert!(proxy.onward().is_empty());
        let port = relay.port;
        drop(relay);
        std::thread::sleep(Duration::from_millis(300));
        // Stopped: a new connection gets nothing.
        if let Ok(mut c) = TcpStream::connect(("127.0.0.1", port)) {
            let _ = write!(c, "GET http://127.0.0.1:1/ HTTP/1.1\r\n\r\n");
            let mut out = String::new();
            let _ = c.read_to_string(&mut out);
            assert!(out.is_empty());
        }
    }
}
