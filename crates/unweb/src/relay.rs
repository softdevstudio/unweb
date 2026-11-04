//! A stranger's relay: a small SOCKS5 proxy of Unweb's own, on 127.0.0.1,
//! that passes every connection on to the stranger's real proxy and signs
//! in there.
//!
//! Two jobs. No engine takes SOCKS5 credentials (Chromium has no SOCKS5
//! sign-in at all), so a location with a user name and password goes
//! through the relay, which does the sign-in itself: SOCKS5 user name and
//! password (RFC 1929), or HTTP CONNECT with a Basic Proxy-Authorization
//! header. And it is the kill switch: when the real proxy can't be reached
//! or refuses the sign-in, the relay never answers the engine with a
//! failure, since a failed proxy is what makes macOS go direct. It holds the
//! connection instead, and tells the app, which blocks the stranger (its
//! page goes to about:blank and may not open sites until the proxy answers
//! again). On macOS every stranger with a location goes through a relay.
//!
//! The relay takes only CONNECT, passes names on for the proxy to look up,
//! and asks the engine for no sign-in: other programs on this computer
//! could use it while the stranger is open (pages can't, as they can't
//! speak SOCKS5). It stops when the stranger is gone.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use tauri::Url;

/// How long reaching the proxy and signing in may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
/// Connections held while the proxy is down. Past this, new ones wait
/// unaccepted (an engine sees a slow proxy, never a failed one).
pub const MAX_HELD: usize = 64;

/// Why a relay blocked its stranger.
pub const NOT_ANSWERING: &str = "Proxy not answering: nothing loads until it does";
pub const SIGN_IN_REFUSED: &str = "The proxy refused the user name and password: nothing loads";

/// Whether a site the proxy can't reach is answered with success followed
/// by a close rather than a SOCKS5 failure. macOS takes a failure as a
/// reason to go direct.
const NEVER_FAIL: bool = cfg!(target_os = "macos");

#[derive(Clone, Debug)]
struct Upstream {
    socks: bool,
    host: String,
    port: u16,
    creds: Option<(String, String)>,
}

/// What came of asking the real proxy for a connection.
enum Outcome {
    /// The tunnel is open; bytes the proxy sent after its answer.
    Open(TcpStream, Vec<u8>),
    /// The proxy works, but can't reach that site (a SOCKS5 reply code).
    Unreachable(u8),
    /// The proxy can't be reached, or refused the sign-in.
    Down(&'static str),
}

type OnBlock = Box<dyn Fn(&'static str) + Send + Sync>;

struct Inner {
    up: Upstream,
    blocked: Mutex<Option<&'static str>>,
    stopping: AtomicBool,
    held: Mutex<Vec<TcpStream>>,
    held_count: AtomicUsize,
    on_block: OnBlock,
}

pub struct Relay {
    port: u16,
    inner: Arc<Inner>,
}

impl Relay {
    /// Starts a relay for `location` (already checked by
    /// location::parse_location). `on_block` is called, on a thread of the
    /// relay's, each time the relay blocks its stranger.
    pub fn start(location: &Url, on_block: OnBlock) -> std::io::Result<Relay> {
        let up = Upstream {
            socks: location.scheme() == "socks5",
            host: location
                .host_str()
                .unwrap_or_default()
                .trim_start_matches('[')
                .trim_end_matches(']')
                .to_string(),
            port: location.port().unwrap_or_default(),
            creds: crate::location::credentials(location),
        };
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        let inner = Arc::new(Inner {
            up,
            blocked: Mutex::new(None),
            stopping: AtomicBool::new(false),
            held: Mutex::new(Vec::new()),
            held_count: AtomicUsize::new(0),
            on_block,
        });
        let i = inner.clone();
        std::thread::spawn(move || accept_loop(listener, i));
        Ok(Relay { port, inner })
    }

    /// The address the engine is given instead of the proxy.
    pub fn url(&self) -> Url {
        Url::parse(&format!("socks5://127.0.0.1:{}", self.port)).expect("a relay address")
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Why the stranger is blocked, if it is.
    pub fn blocked(&self) -> Option<&'static str> {
        self.inner.blocked.lock().ok().and_then(|b| *b)
    }

    /// How many connections are being held.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn held(&self) -> usize {
        self.inner.held_count.load(Ordering::SeqCst)
    }

    /// Reaches the proxy and signs in (an HTTP proxy is only reached: it
    /// asks for the sign-in with the first connection). On success the
    /// stranger is no longer blocked.
    pub fn check(&self) -> Result<(), &'static str> {
        let r = check_upstream(&self.inner.up);
        if r.is_ok() {
            if let Ok(mut b) = self.inner.blocked.lock() {
                *b = None;
            }
        }
        r
    }

    /// Stops the relay and closes every connection it holds. Call only once
    /// the stranger's page is gone: an engine still running would see its
    /// proxy fail.
    pub fn stop(&self) {
        if self.inner.stopping.swap(true, Ordering::SeqCst) {
            return;
        }
        // Wakes the accept loop.
        let _ = TcpStream::connect_timeout(
            &SocketAddr::from(([127, 0, 0, 1], self.port)),
            Duration::from_secs(1),
        );
        if let Ok(mut h) = self.inner.held.lock() {
            for s in h.drain(..) {
                let _ = s.shutdown(Shutdown::Both);
            }
        }
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.stop();
    }
}

fn accept_loop(listener: TcpListener, inner: Arc<Inner>) {
    for stream in listener.incoming() {
        if inner.stopping.load(Ordering::SeqCst) {
            break;
        }
        let Ok(stream) = stream else { continue };
        // Only this computer, though it listens on loopback anyway.
        if !stream
            .peer_addr()
            .map(|a| a.ip().is_loopback())
            .unwrap_or(false)
        {
            continue;
        }
        let i = inner.clone();
        std::thread::spawn(move || {
            let _ = serve(stream, &i);
        });
        // While the proxy is down, connections are held; past the cap,
        // leave new ones waiting in the queue.
        while inner.held_count.load(Ordering::SeqCst) >= MAX_HELD
            && !inner.stopping.load(Ordering::SeqCst)
        {
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

/// A SOCKS5 destination as the engine gave it.
#[derive(Clone, Debug, PartialEq)]
enum Dest {
    V4([u8; 4]),
    V6([u8; 16]),
    Name(String),
}

impl Dest {
    /// As SOCKS5 writes it (address type and address).
    fn socks(&self) -> Vec<u8> {
        match self {
            Dest::V4(a) => [&[1u8][..], a].concat(),
            Dest::V6(a) => [&[4u8][..], a].concat(),
            Dest::Name(n) => [&[3u8, n.len() as u8][..], n.as_bytes()].concat(),
        }
    }

    /// As an HTTP CONNECT target writes the host.
    fn http(&self) -> String {
        match self {
            Dest::V4(a) => std::net::Ipv4Addr::from(*a).to_string(),
            Dest::V6(a) => format!("[{}]", std::net::Ipv6Addr::from(*a)),
            Dest::Name(n) => n.clone(),
        }
    }
}

const SUCCESS: [u8; 10] = [5, 0, 0, 1, 0, 0, 0, 0, 0, 0];

fn reply(code: u8) -> [u8; 10] {
    let mut r = SUCCESS;
    r[1] = code;
    r
}

fn serve(mut engine: TcpStream, inner: &Inner) -> std::io::Result<()> {
    engine.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;
    let mut b = [0u8; 2];
    engine.read_exact(&mut b)?;
    if b[0] != 5 {
        return Ok(());
    }
    let mut methods = vec![0u8; b[1] as usize];
    engine.read_exact(&mut methods)?;
    if !methods.contains(&0) {
        engine.write_all(&[5, 0xff])?;
        return Ok(());
    }
    engine.write_all(&[5, 0])?;
    let mut req = [0u8; 4];
    engine.read_exact(&mut req)?;
    if req[0] != 5 || req[1] != 1 {
        // Only CONNECT.
        engine.write_all(&reply(7))?;
        return Ok(());
    }
    let dest = match req[3] {
        1 => {
            let mut a = [0u8; 4];
            engine.read_exact(&mut a)?;
            Dest::V4(a)
        }
        4 => {
            let mut a = [0u8; 16];
            engine.read_exact(&mut a)?;
            Dest::V6(a)
        }
        3 => {
            let mut n = [0u8; 1];
            engine.read_exact(&mut n)?;
            let mut name = vec![0u8; n[0] as usize];
            engine.read_exact(&mut name)?;
            match String::from_utf8(name) {
                Ok(n) if !n.is_empty() && !n.bytes().any(|c| c.is_ascii_control() || c == b' ') => {
                    Dest::Name(n)
                }
                _ => {
                    engine.write_all(&reply(4))?;
                    return Ok(());
                }
            }
        }
        _ => {
            engine.write_all(&reply(8))?;
            return Ok(());
        }
    };
    let mut p = [0u8; 2];
    engine.read_exact(&mut p)?;
    let port = u16::from_be_bytes(p);

    let blocked = inner.blocked.lock().map(|b| b.is_some()).unwrap_or(true);
    if blocked || inner.stopping.load(Ordering::SeqCst) {
        hold(engine, inner);
        return Ok(());
    }
    match open(&inner.up, &dest, port) {
        Outcome::Open(up, early) => {
            engine.write_all(&SUCCESS)?;
            engine.write_all(&early)?;
            engine.set_read_timeout(None)?;
            up.set_read_timeout(None)?;
            pipe(engine, up);
        }
        Outcome::Unreachable(code) => {
            if NEVER_FAIL {
                engine.write_all(&SUCCESS)?;
                let _ = engine.shutdown(Shutdown::Both);
            } else {
                engine.write_all(&reply(code))?;
            }
        }
        Outcome::Down(why) => {
            block(inner, why);
            hold(engine, inner);
        }
    }
    Ok(())
}

/// Marks the stranger blocked and tells the app (once per time it blocks).
fn block(inner: &Inner, why: &'static str) {
    let newly = match inner.blocked.lock() {
        Ok(mut b) => {
            let was = b.is_some();
            *b = Some(why);
            !was
        }
        Err(_) => false,
    };
    if newly {
        (inner.on_block)(why);
    }
}

/// Keeps the engine's connection open, unanswered, until the engine gives
/// up on it or the relay stops.
fn hold(engine: TcpStream, inner: &Inner) {
    if let Ok(c) = engine.try_clone() {
        if let Ok(mut h) = inner.held.lock() {
            h.push(c);
        }
    }
    inner.held_count.fetch_add(1, Ordering::SeqCst);
    let _ = engine.set_read_timeout(None);
    let mut e = engine;
    let mut buf = [0u8; 512];
    while !inner.stopping.load(Ordering::SeqCst) {
        match e.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
    }
    inner.held_count.fetch_sub(1, Ordering::SeqCst);
    if let (Ok(mut h), Ok(me)) = (inner.held.lock(), e.local_addr()) {
        let peer = e.peer_addr().ok();
        h.retain(|s| !(s.local_addr().ok() == Some(me) && s.peer_addr().ok() == peer));
    }
}

fn connect_upstream(up: &Upstream) -> Result<TcpStream, &'static str> {
    let addrs: Vec<SocketAddr> = (up.host.as_str(), up.port)
        .to_socket_addrs()
        .map_err(|_| NOT_ANSWERING)?
        .collect();
    for a in addrs {
        if let Ok(s) = TcpStream::connect_timeout(&a, CONNECT_TIMEOUT) {
            let _ = s.set_read_timeout(Some(HANDSHAKE_TIMEOUT));
            let _ = s.set_write_timeout(Some(HANDSHAKE_TIMEOUT));
            let _ = s.set_nodelay(true);
            return Ok(s);
        }
    }
    Err(NOT_ANSWERING)
}

/// SOCKS5 greeting and sign-in on a fresh connection to the proxy.
fn socks_sign_in(s: &mut TcpStream, up: &Upstream) -> Result<(), &'static str> {
    let greeting: &[u8] = if up.creds.is_some() {
        &[5, 2, 0, 2]
    } else {
        &[5, 1, 0]
    };
    s.write_all(greeting).map_err(|_| NOT_ANSWERING)?;
    let mut m = [0u8; 2];
    s.read_exact(&mut m).map_err(|_| NOT_ANSWERING)?;
    if m[0] != 5 {
        return Err(NOT_ANSWERING);
    }
    match (m[1], &up.creds) {
        (0, _) => Ok(()),
        (2, Some((user, pass))) => {
            let mut a = vec![1u8, user.len() as u8];
            a.extend_from_slice(user.as_bytes());
            a.push(pass.len() as u8);
            a.extend_from_slice(pass.as_bytes());
            s.write_all(&a).map_err(|_| NOT_ANSWERING)?;
            let mut r = [0u8; 2];
            s.read_exact(&mut r).map_err(|_| NOT_ANSWERING)?;
            if r[1] == 0 {
                Ok(())
            } else {
                Err(SIGN_IN_REFUSED)
            }
        }
        // It wants a sign-in Unweb can't give (or doesn't have).
        _ => Err(SIGN_IN_REFUSED),
    }
}

fn check_upstream(up: &Upstream) -> Result<(), &'static str> {
    let mut s = connect_upstream(up)?;
    if up.socks {
        socks_sign_in(&mut s, up)?;
    }
    let _ = s.shutdown(Shutdown::Both);
    Ok(())
}

fn open(up: &Upstream, dest: &Dest, port: u16) -> Outcome {
    let mut s = match connect_upstream(up) {
        Ok(s) => s,
        Err(why) => return Outcome::Down(why),
    };
    if up.socks {
        if let Err(why) = socks_sign_in(&mut s, up) {
            return Outcome::Down(why);
        }
        let mut req = vec![5u8, 1, 0];
        req.extend_from_slice(&dest.socks());
        req.extend_from_slice(&port.to_be_bytes());
        if s.write_all(&req).is_err() {
            return Outcome::Down(NOT_ANSWERING);
        }
        let mut head = [0u8; 4];
        if s.read_exact(&mut head).is_err() || head[0] != 5 {
            return Outcome::Down(NOT_ANSWERING);
        }
        // The address the proxy bound, which nobody needs.
        let rest = match head[3] {
            1 => 4 + 2,
            4 => 16 + 2,
            3 => {
                let mut n = [0u8; 1];
                if s.read_exact(&mut n).is_err() {
                    return Outcome::Down(NOT_ANSWERING);
                }
                n[0] as usize + 2
            }
            _ => return Outcome::Down(NOT_ANSWERING),
        };
        let mut skip = vec![0u8; rest];
        if s.read_exact(&mut skip).is_err() {
            return Outcome::Down(NOT_ANSWERING);
        }
        if head[1] != 0 {
            return Outcome::Unreachable(head[1]);
        }
        return Outcome::Open(s, Vec::new());
    }
    // An HTTP proxy: CONNECT, with the sign-in when there is one.
    let target = format!("{}:{port}", dest.http());
    let mut ask = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n");
    if let Some((user, pass)) = &up.creds {
        let token = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}"));
        ask.push_str(&format!("Proxy-Authorization: Basic {token}\r\n"));
    }
    ask.push_str("\r\n");
    if s.write_all(ask.as_bytes()).is_err() {
        return Outcome::Down(NOT_ANSWERING);
    }
    let Ok(clone) = s.try_clone() else {
        return Outcome::Down(NOT_ANSWERING);
    };
    let mut reader = BufReader::new(clone);
    let mut status = String::new();
    if reader.read_line(&mut status).unwrap_or(0) == 0 {
        return Outcome::Down(NOT_ANSWERING);
    }
    let code: u16 = status
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    if !status.starts_with("HTTP/") || code == 0 {
        return Outcome::Down(NOT_ANSWERING);
    }
    // The headers, up to the blank line (at most 16 KiB of them).
    let mut seen = 0usize;
    loop {
        let mut h = String::new();
        match reader.read_line(&mut h) {
            Ok(0) | Err(_) => return Outcome::Down(NOT_ANSWERING),
            Ok(n) => seen += n,
        }
        if h == "\r\n" || h == "\n" {
            break;
        }
        if seen > 16 * 1024 {
            return Outcome::Down(NOT_ANSWERING);
        }
    }
    match code {
        200..=299 => Outcome::Open(s, reader.buffer().to_vec()),
        407 => Outcome::Down(SIGN_IN_REFUSED),
        // SOCKS5: 2 not allowed, 4 host unreachable, 1 general failure.
        403 => Outcome::Unreachable(2),
        502..=504 => Outcome::Unreachable(4),
        _ => Outcome::Unreachable(1),
    }
}

/// Copies both ways until either side is done.
fn pipe(a: TcpStream, b: TcpStream) {
    let (mut a2, mut b2) = match (a.try_clone(), b.try_clone()) {
        (Ok(x), Ok(y)) => (x, y),
        _ => return,
    };
    let t = std::thread::spawn(move || {
        let (mut a, mut b) = (a, b);
        let _ = std::io::copy(&mut a, &mut b);
        let _ = b.shutdown(Shutdown::Write);
    });
    let _ = std::io::copy(&mut b2, &mut a2);
    let _ = a2.shutdown(Shutdown::Write);
    let _ = t.join();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testproxy::{closed_port, Proxy};
    use std::sync::mpsc;

    /// A site that answers "ok" and logs the port each request came from.
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

    fn relay(loc: &str) -> (Relay, mpsc::Receiver<&'static str>) {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let l = crate::location::parse_location(loc).unwrap().unwrap();
        let r = Relay::start(
            &l,
            Box::new(move |why| {
                let _ = tx.lock().unwrap().send(why);
            }),
        )
        .unwrap();
        (r, rx)
    }

    /// Asks the relay, as an engine would, for `name:port`; returns the
    /// connection and the reply code (None when nothing came back in time).
    fn ask(relay_port: u16, name: &str, port: u16) -> (TcpStream, Option<u8>) {
        let mut c = TcpStream::connect(("127.0.0.1", relay_port)).unwrap();
        c.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        c.write_all(&[5, 1, 0]).unwrap();
        let mut b = [0u8; 2];
        c.read_exact(&mut b).unwrap();
        assert_eq!(b, [5, 0]);
        let mut req = vec![5, 1, 0, 3, name.len() as u8];
        req.extend_from_slice(name.as_bytes());
        req.extend_from_slice(&port.to_be_bytes());
        c.write_all(&req).unwrap();
        let mut rep = [0u8; 10];
        let code = c.read_exact(&mut rep).ok().map(|()| rep[1]);
        (c, code)
    }

    fn get(mut c: TcpStream) -> String {
        c.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        c.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
        let mut out = String::new();
        let _ = c.read_to_string(&mut out);
        out
    }

    #[test]
    fn chains_through_both_kinds_with_and_without_sign_in() {
        let (port, peers) = site();
        for (proxy, loc) in [
            (Proxy::socks5().unwrap(), "socks5://127.0.0.1:{}"),
            (Proxy::http().unwrap(), "http://127.0.0.1:{}"),
            (
                Proxy::socks5_sign_in("ann", "s3cret").unwrap(),
                "socks5://ann:s3cret@127.0.0.1:{}",
            ),
            (
                Proxy::http_sign_in("ann", "s:3 cret").unwrap(),
                "http://ann:s%3A3%20cret@127.0.0.1:{}",
            ),
        ] {
            let (r, rx) = relay(&loc.replace("{}", &proxy.port.to_string()));
            let (c, code) = ask(r.port(), "far.unweb.test", port);
            assert_eq!(code, Some(0), "{loc}");
            assert!(get(c).ends_with("ok"), "{loc}");
            assert!(rx.try_recv().is_err(), "{loc} blocked");
            let p = peers.lock().unwrap().clone();
            assert!(proxy.onward_ports().iter().all(|x| p.contains(x)), "{loc}");
            assert_eq!(proxy.onward()[0].host, "far.unweb.test", "{loc}");
            assert!(r.check().is_ok());
        }
    }

    #[test]
    fn a_proxy_that_is_down_blocks_and_holds_and_never_fails() {
        let (port, peers) = site();
        let (r, rx) = relay(&format!("socks5://127.0.0.1:{}", closed_port()));
        let (c1, code) = ask(r.port(), "127.0.0.1", port);
        // No answer at all, success or failure.
        assert_eq!(code, None);
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok(NOT_ANSWERING));
        assert_eq!(r.blocked(), Some(NOT_ANSWERING));
        // While blocked, new connections are held too, and the app is told
        // only once.
        let (_c2, code2) = ask(r.port(), "127.0.0.1", port);
        assert_eq!(code2, None);
        assert!(rx.try_recv().is_err());
        assert_eq!(r.held(), 2);
        assert!(r.check().is_err());
        assert!(peers.lock().unwrap().is_empty());
        // The engine giving up frees its place.
        drop(c1);
        let t = std::time::Instant::now();
        while r.held() != 1 && t.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(r.held(), 1);
        r.stop();
        let t = std::time::Instant::now();
        while r.held() != 0 && t.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(r.held(), 0);
    }

    #[test]
    fn a_refused_sign_in_blocks() {
        let (port, _) = site();
        let socks = Proxy::socks5_sign_in("ann", "right").unwrap();
        let (r, rx) = relay(&format!("socks5://ann:wrong@127.0.0.1:{}", socks.port));
        let (_c, code) = ask(r.port(), "127.0.0.1", port);
        assert_eq!(code, None);
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok(SIGN_IN_REFUSED));
        assert_eq!(r.check(), Err(SIGN_IN_REFUSED));
        // No credentials at all, to a proxy that wants them.
        let (r2, rx2) = relay(&format!("socks5://127.0.0.1:{}", socks.port));
        let _ = ask(r2.port(), "127.0.0.1", port);
        assert_eq!(
            rx2.recv_timeout(Duration::from_secs(5)),
            Ok(SIGN_IN_REFUSED)
        );
        let http = Proxy::http_sign_in("ann", "right").unwrap();
        let (r3, rx3) = relay(&format!("http://ann:wrong@127.0.0.1:{}", http.port));
        let _ = ask(r3.port(), "127.0.0.1", port);
        assert_eq!(
            rx3.recv_timeout(Duration::from_secs(5)),
            Ok(SIGN_IN_REFUSED)
        );
        assert!(socks.onward().is_empty() && http.onward().is_empty());
    }

    #[test]
    fn a_site_the_proxy_cant_reach_is_not_a_failed_proxy_on_macos() {
        let proxy = Proxy::socks5().unwrap();
        let (r, rx) = relay(&format!("socks5://127.0.0.1:{}", proxy.port));
        let (_c, code) = ask(r.port(), "127.0.0.1", closed_port());
        if NEVER_FAIL {
            assert_eq!(code, Some(0));
        } else {
            assert_eq!(code, Some(5));
        }
        assert!(rx.try_recv().is_err());
        assert_eq!(r.blocked(), None);
        let http = Proxy::http().unwrap();
        let (r2, _) = relay(&format!("http://127.0.0.1:{}", http.port));
        let (_c, code) = ask(r2.port(), "127.0.0.1", closed_port());
        assert_eq!(code, Some(if NEVER_FAIL { 0 } else { 4 }));
        assert_eq!(r2.blocked(), None);
    }

    #[test]
    fn a_proxy_that_stops_blocks_then_comes_back() {
        let (port, _) = site();
        let proxy = Proxy::socks5().unwrap();
        let (r, rx) = relay(&format!("socks5://127.0.0.1:{}", proxy.port));
        assert_eq!(ask(r.port(), "127.0.0.1", port).1, Some(0));
        proxy.set_down(true);
        assert_eq!(ask(r.port(), "127.0.0.1", port).1, None);
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok(NOT_ANSWERING));
        proxy.set_down(false);
        assert!(r.check().is_ok());
        assert_eq!(r.blocked(), None);
        assert_eq!(ask(r.port(), "127.0.0.1", port).1, Some(0));
    }

    #[test]
    fn only_connect_is_taken() {
        let proxy = Proxy::socks5().unwrap();
        let (r, _) = relay(&format!("socks5://127.0.0.1:{}", proxy.port));
        let mut c = TcpStream::connect(("127.0.0.1", r.port())).unwrap();
        c.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        c.write_all(&[5, 1, 0]).unwrap();
        let mut b = [0u8; 2];
        c.read_exact(&mut b).unwrap();
        // UDP ASSOCIATE.
        c.write_all(&[5, 3, 0, 1, 0, 0, 0, 0, 0, 0]).unwrap();
        let mut rep = [0u8; 10];
        c.read_exact(&mut rep).unwrap();
        assert_eq!(rep[1], 7);
        // An HTTP request, as a page's fetch would send.
        let mut h = TcpStream::connect(("127.0.0.1", r.port())).unwrap();
        h.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        h.write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .unwrap();
        let mut out = Vec::new();
        let _ = h.read_to_end(&mut out);
        assert!(out.is_empty());
        assert!(proxy.onward().is_empty());
    }
}
