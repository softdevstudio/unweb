//! The self-test's own proxies, for the location checks: an HTTP proxy
//! (absolute-form requests and CONNECT) and a SOCKS5 proxy (no sign-in).
//!
//! Both log every connection they make onward: where to, how it was asked
//! for, and the local port of the onward connection. The test site logs the
//! port each request came from, so a request whose port isn't in a proxy's
//! log didn't come through that proxy. Names ending in `.unweb.test` only
//! resolve inside these proxies (to 127.0.0.1), so a stranger can reach
//! them only through its proxy.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, serde::Serialize)]
pub struct Onward {
    /// "absolute" (an HTTP proxy's GET http://...), "connect", "socks-name"
    /// (SOCKS5 given a host name) or "socks-ip".
    pub how: String,
    pub host: String,
    pub port: u16,
    /// The onward connection's local port, as the site sees it.
    pub from_port: u16,
}

#[derive(Clone)]
pub struct Proxy {
    pub port: u16,
    pub onward: Arc<Mutex<Vec<Onward>>>,
}

impl Proxy {
    pub fn http() -> std::io::Result<Proxy> {
        Proxy::start(false)
    }

    pub fn socks5() -> std::io::Result<Proxy> {
        Proxy::start(true)
    }

    fn start(socks: bool) -> std::io::Result<Proxy> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        let onward = Arc::new(Mutex::new(Vec::new()));
        let log = onward.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let log = log.clone();
                std::thread::spawn(move || {
                    let _ = if socks {
                        socks5(stream, &log)
                    } else {
                        http(stream, &log)
                    };
                });
            }
        });
        Ok(Proxy { port, onward })
    }

    pub fn onward(&self) -> Vec<Onward> {
        self.onward.lock().map(|o| o.clone()).unwrap_or_default()
    }

    /// The local ports of every onward connection so far.
    pub fn onward_ports(&self) -> Vec<u16> {
        self.onward().iter().map(|o| o.from_port).collect()
    }
}

/// A port nothing listens on: a proxy that is down.
pub fn closed_port() -> u16 {
    let l = TcpListener::bind(("127.0.0.1", 0)).expect("a free port");
    let port = l.local_addr().expect("its port").port();
    drop(l);
    port
}

/// Where a name goes: `.unweb.test` names to 127.0.0.1, anything else as
/// given (the self-test only uses local addresses).
fn connect(host: &str, port: u16) -> std::io::Result<TcpStream> {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let target = if host.ends_with(".unweb.test") {
        "127.0.0.1"
    } else {
        host
    };
    let s = TcpStream::connect((target, port))?;
    s.set_read_timeout(Some(Duration::from_secs(30)))?;
    Ok(s)
}

fn record(log: &Mutex<Vec<Onward>>, how: &str, host: &str, port: u16, s: &TcpStream) {
    let from_port = s.local_addr().map(|a| a.port()).unwrap_or_default();
    if let Ok(mut l) = log.lock() {
        l.push(Onward {
            how: how.into(),
            host: host.into(),
            port,
            from_port,
        });
    }
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

fn split_host_port(s: &str, default: u16) -> (String, u16) {
    match s.rsplit_once(':') {
        Some((h, p)) if !h.ends_with(':') => (h.to_string(), p.parse().unwrap_or(default)),
        _ => (s.to_string(), default),
    }
}

fn http(client: TcpStream, log: &Mutex<Vec<Onward>>) -> std::io::Result<()> {
    client.set_read_timeout(Some(Duration::from_secs(30)))?;
    let mut reader = BufReader::new(client.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();
    let mut headers = Vec::new();
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h)? == 0 || h == "\r\n" || h == "\n" {
            break;
        }
        headers.push(h);
    }
    let mut client_w = client.try_clone()?;
    if method.eq_ignore_ascii_case("CONNECT") {
        let (host, port) = split_host_port(&target, 443);
        let up = match connect(&host, port) {
            Ok(u) => u,
            Err(_) => {
                client_w.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")?;
                return Ok(());
            }
        };
        record(log, "connect", &host, port, &up);
        client_w.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")?;
        // Whatever the client sent after the request is already buffered.
        let early = reader.buffer().to_vec();
        let mut up_w = up.try_clone()?;
        up_w.write_all(&early)?;
        pipe(client, up);
        return Ok(());
    }
    // GET http://host:port/path HTTP/1.1
    let Some(rest) = target.strip_prefix("http://") else {
        client_w.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n")?;
        return Ok(());
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = split_host_port(authority, 80);
    let mut up = match connect(&host, port) {
        Ok(u) => u,
        Err(_) => {
            client_w.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")?;
            return Ok(());
        }
    };
    record(log, "absolute", &host, port, &up);
    let mut head = format!("{method} {path} HTTP/1.1\r\n");
    for h in &headers {
        let lower = h.to_ascii_lowercase();
        if lower.starts_with("proxy-connection:") || lower.starts_with("connection:") {
            continue;
        }
        head.push_str(h);
    }
    head.push_str("Connection: close\r\n\r\n");
    up.write_all(head.as_bytes())?;
    let early = reader.buffer().to_vec();
    up.write_all(&early)?;
    // One request per connection: the answer, then both close.
    let mut up_r = up.try_clone()?;
    let _ = std::io::copy(&mut up_r, &mut client_w);
    let _ = client_w.shutdown(std::net::Shutdown::Both);
    Ok(())
}

fn socks5(mut client: TcpStream, log: &Mutex<Vec<Onward>>) -> std::io::Result<()> {
    client.set_read_timeout(Some(Duration::from_secs(30)))?;
    let mut b = [0u8; 2];
    client.read_exact(&mut b)?;
    if b[0] != 5 {
        return Ok(());
    }
    let mut methods = vec![0u8; b[1] as usize];
    client.read_exact(&mut methods)?;
    if !methods.contains(&0) {
        client.write_all(&[5, 0xff])?;
        return Ok(());
    }
    client.write_all(&[5, 0])?;
    let mut req = [0u8; 4];
    client.read_exact(&mut req)?;
    if req[1] != 1 {
        client.write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0])?;
        return Ok(());
    }
    let (how, host) = match req[3] {
        1 => {
            let mut a = [0u8; 4];
            client.read_exact(&mut a)?;
            ("socks-ip", std::net::Ipv4Addr::from(a).to_string())
        }
        3 => {
            let mut n = [0u8; 1];
            client.read_exact(&mut n)?;
            let mut name = vec![0u8; n[0] as usize];
            client.read_exact(&mut name)?;
            ("socks-name", String::from_utf8_lossy(&name).into_owned())
        }
        4 => {
            let mut a = [0u8; 16];
            client.read_exact(&mut a)?;
            ("socks-ip", std::net::Ipv6Addr::from(a).to_string())
        }
        _ => return Ok(()),
    };
    let mut p = [0u8; 2];
    client.read_exact(&mut p)?;
    let port = u16::from_be_bytes(p);
    let up = match connect(&host, port) {
        Ok(u) => u,
        Err(_) => {
            client.write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0])?;
            return Ok(());
        }
    };
    record(log, how, &host, port, &up);
    client.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])?;
    pipe(client, up);
    Ok(())
}

/// A UDP listener that counts what arrives: a STUN server's address for
/// WebRTC, which would only be reached around a proxy.
#[derive(Clone)]
pub struct Stun {
    pub port: u16,
    pub packets: Arc<Mutex<usize>>,
}

impl Stun {
    pub fn start() -> std::io::Result<Stun> {
        let sock = UdpSocket::bind(("127.0.0.1", 0))?;
        let port = sock.local_addr()?.port();
        let packets = Arc::new(Mutex::new(0usize));
        let p = packets.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 2048];
            while let Ok((n, from)) = sock.recv_from(&mut buf) {
                if let Ok(mut c) = p.lock() {
                    *c += 1;
                }
                // A STUN binding request (type 0x0001): answer with a
                // binding success carrying the sender's address, so the
                // engine counts it as a real server.
                if n >= 20 && buf[0] == 0 && buf[1] == 1 {
                    let mut resp = Vec::with_capacity(32);
                    resp.extend_from_slice(&[0x01, 0x01, 0x00, 0x0c]);
                    resp.extend_from_slice(&buf[4..20]);
                    if let std::net::SocketAddr::V4(a) = from {
                        let cookie = [0x21u8, 0x12, 0xa4, 0x42];
                        let xport = a.port() ^ 0x2112;
                        let ip = a.ip().octets();
                        resp.extend_from_slice(&[0x00, 0x20, 0x00, 0x08, 0x00, 0x01]);
                        resp.extend_from_slice(&xport.to_be_bytes());
                        for i in 0..4 {
                            resp.push(ip[i] ^ cookie[i]);
                        }
                        let _ = sock.send_to(&resp, from);
                    }
                }
            }
        });
        Ok(Stun { port, packets })
    }

    pub fn count(&self) -> usize {
        self.packets.lock().map(|c| *c).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site() -> (u16, Arc<Mutex<Vec<u16>>>) {
        let l = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = l.local_addr().unwrap().port();
        let peers = Arc::new(Mutex::new(Vec::new()));
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

    #[test]
    fn http_proxy_forwards_and_logs() {
        let (port, peers) = site();
        let proxy = Proxy::http().unwrap();
        let mut c = TcpStream::connect(("127.0.0.1", proxy.port)).unwrap();
        write!(c, "GET http://far.unweb.test:{port}/x?y HTTP/1.1\r\nHost: far.unweb.test:{port}\r\nProxy-Connection: keep-alive\r\n\r\n").unwrap();
        let mut out = String::new();
        c.read_to_string(&mut out).unwrap();
        assert!(out.ends_with("ok"), "{out}");
        let onward = proxy.onward();
        assert_eq!(onward.len(), 1);
        assert_eq!(onward[0].host, "far.unweb.test");
        assert_eq!(peers.lock().unwrap().clone(), proxy.onward_ports());
    }

    #[test]
    fn socks5_proxy_forwards_names_and_logs() {
        let (port, peers) = site();
        let proxy = Proxy::socks5().unwrap();
        let mut c = TcpStream::connect(("127.0.0.1", proxy.port)).unwrap();
        c.write_all(&[5, 1, 0]).unwrap();
        let mut b = [0u8; 2];
        c.read_exact(&mut b).unwrap();
        assert_eq!(b, [5, 0]);
        let name = b"far.unweb.test";
        let mut req = vec![5, 1, 0, 3, name.len() as u8];
        req.extend_from_slice(name);
        req.extend_from_slice(&port.to_be_bytes());
        c.write_all(&req).unwrap();
        let mut rep = [0u8; 10];
        c.read_exact(&mut rep).unwrap();
        assert_eq!(rep[1], 0);
        c.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
        let mut out = String::new();
        c.read_to_string(&mut out).unwrap();
        assert!(out.ends_with("ok"));
        assert_eq!(proxy.onward()[0].how, "socks-name");
        assert_eq!(peers.lock().unwrap().clone(), proxy.onward_ports());
    }

    #[test]
    fn a_closed_port_refuses() {
        assert!(TcpStream::connect(("127.0.0.1", closed_port())).is_err());
    }
}
