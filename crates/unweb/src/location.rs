//! A stranger's location: a proxy the person supplies, so the stranger's
//! traffic leaves from somewhere else.
//!
//! A location is `http://host:port` (an HTTP proxy) or `socks5://host:port`
//! (a SOCKS5 proxy). Everything the stranger loads goes through it, local
//! addresses included, and when the proxy can't be reached the stranger
//! loads nothing rather than going direct. WebRTC is turned off in a
//! stranger with a location, since it can reach the network around a proxy.
//!
//! A location may carry a user name and password (`user:pass@host:port`).
//! No engine takes SOCKS5 credentials, so such a stranger, and every
//! stranger with a location on macOS, is given Unweb's own relay instead,
//! which signs in to the proxy (see relay.rs).

use tauri::Url;

/// What someone typed in a strip's location field, as a proxy address, or
/// None for "direct" (empty). `host:port` alone means an HTTP proxy.
pub fn parse_location(text: &str) -> Result<Option<Url>, String> {
    let text = text.trim();
    if text.is_empty() || text.eq_ignore_ascii_case("direct") {
        return Ok(None);
    }
    if text.len() > 1600 || text.contains(char::is_whitespace) {
        return Err(HELP.into());
    }
    let (scheme, rest) = match text.split_once("://") {
        Some((s, r)) => (s.to_ascii_lowercase(), r),
        None => ("http".to_string(), text),
    };
    if !matches!(scheme.as_str(), "http" | "socks5") {
        return Err("Unweb takes http:// and socks5:// proxies".into());
    }
    // Parsed as an http address whatever the scheme, so the host gets the
    // full check (socks5 is a scheme the parser keeps hosts raw for).
    let url = Url::parse(&format!("http://{rest}")).map_err(|_| HELP.to_string())?;
    let creds = match (url.username(), url.password()) {
        ("", None) => None,
        (user, Some(pass)) if !user.is_empty() && !pass.is_empty() => {
            let (user, pass) = (decode(user)?, decode(pass)?);
            if scheme == "http" && user.contains(':') {
                return Err("An HTTP proxy's user name can't hold a colon".into());
            }
            Some((user, pass))
        }
        _ => return Err("Give both the user name and the password, like socks5://user:password@proxy.example:1080".into()),
    };
    match url.host() {
        // A host name as DNS knows them (letters, digits, dots, hyphens,
        // after the parser turned any other script into its xn-- form).
        Some(url::Host::Domain(d))
            if !d.is_empty()
                && d.bytes().all(|b| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-'
                }) => {}
        Some(url::Host::Domain(_)) => return Err(HELP.into()),
        Some(url::Host::Ipv4(_)) => {}
        Some(url::Host::Ipv6(_)) => {
            return Err(
                "IPv6 proxy addresses aren't supported yet; use a name or an IPv4 address".into(),
            )
        }
        None => return Err(HELP.into()),
    }
    // A proxy has no default port worth guessing.
    let Some(port) = url.port() else {
        return Err("Give the proxy's port too, like proxy.example:8080".into());
    };
    if port == 0 {
        return Err(HELP.into());
    }
    if !matches!(url.path(), "" | "/") || url.query().is_some() || url.fragment().is_some() {
        return Err(HELP.into());
    }
    let host = url.host_str().unwrap_or_default();
    let mut out = Url::parse(&format!("{scheme}://{host}:{port}")).map_err(|_| HELP.to_string())?;
    if let Some((user, pass)) = creds {
        // Written back in one encoding of Unweb's own, so the same
        // credentials always give the same location.
        out.set_username(&encode(&user))
            .map_err(|_| HELP.to_string())?;
        out.set_password(Some(&encode(&pass)))
            .map_err(|_| HELP.to_string())?;
    }
    Ok(Some(out))
}

/// Characters a user name or password keeps as they are in a location.
const KEEP: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

fn encode(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, KEEP).to_string()
}

/// A user name or password as typed (percent-decoded): 1 to 255 bytes, as
/// SOCKS5 sign-in carries, and no control characters.
fn decode(s: &str) -> Result<String, String> {
    let d = percent_encoding::percent_decode_str(s)
        .decode_utf8()
        .map_err(|_| "A user name or password must be UTF-8 text".to_string())?
        .into_owned();
    if d.is_empty() || d.len() > 255 {
        return Err("A user name and a password are 1 to 255 bytes each".into());
    }
    if d.chars().any(char::is_control) {
        return Err("A user name or password can't hold control characters".into());
    }
    Ok(d)
}

/// The user name and password of a location that has them (decoded).
pub fn credentials(location: &Url) -> Option<(String, String)> {
    let pass = location.password()?;
    Some((decode(location.username()).ok()?, decode(pass).ok()?))
}

const HELP: &str = "Type a proxy address, like socks5://proxy.example:1080 or proxy.example:8080";

/// A location as captions and reports show it: scheme, host and port,
/// never the user name or password.
pub fn show(location: Option<&Url>) -> String {
    match location {
        None => String::new(),
        Some(u) => format!(
            "{}://{}:{}",
            u.scheme(),
            u.host_str().unwrap_or_default(),
            u.port().unwrap_or_default()
        ),
    }
}

/// A location as its strip shows it: with the user name, never the
/// password.
pub fn show_user(location: Option<&Url>) -> String {
    match location {
        Some(u) if !u.username().is_empty() => {
            let s = show(location);
            let (scheme, rest) = s.split_once("://").unwrap_or(("", &s));
            format!("{scheme}://{}@{rest}", u.username())
        }
        _ => show(location),
    }
}

/// A location in full, user name and password included, as a saved setup
/// keeps it.
pub fn full(location: Option<&Url>) -> String {
    match location {
        Some(u) if u.password().is_some() => {
            let s = show(location);
            let (scheme, rest) = s.split_once("://").unwrap_or(("", &s));
            format!(
                "{scheme}://{}:{}@{rest}",
                u.username(),
                u.password().unwrap_or_default()
            )
        }
        _ => show(location),
    }
}

/// Whether a stranger with this location is given Unweb's relay rather
/// than the proxy itself: on macOS always (its network layer goes direct
/// when a proxy fails, and the relay never fails), elsewhere when the
/// proxy needs a user name and password, which no engine here takes for
/// SOCKS5.
pub fn needs_relay(location: &Url) -> bool {
    cfg!(target_os = "macos") || location.password().is_some()
}

/// Whether this system can give a stranger a location, and why not.
pub fn supported() -> Result<(), &'static str> {
    #[cfg(target_os = "macos")]
    {
        // WKWebsiteDataStore takes proxies from macOS 14 on; earlier
        // versions would throw.
        if macos_major() < 14 {
            return Err("A location needs macOS 14 or later");
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn macos_major() -> isize {
    objc2_foundation::NSProcessInfo::processInfo()
        .operatingSystemVersion()
        .majorVersion
}

/// WebView2 switches for a stranger's location: the proxy for everything,
/// local addresses included (Chromium sends those direct unless told), and
/// WebRTC kept to the proxy (UDP can't go through one, so none at all).
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn webview2_switches(location: Option<&Url>) -> String {
    match location {
        None => String::new(),
        // Names are looked up by the proxy; the resolver rules make any
        // lookup the engine would still try itself (speculative ones, say)
        // fail, except the proxy's own name.
        Some(l) => format!(
            " --proxy-server={} --proxy-bypass-list=<-loopback> --force-webrtc-ip-handling-policy=disable_non_proxied_udp \"--host-resolver-rules=MAP * ~NOTFOUND , EXCLUDE {}\"",
            show(location),
            l.host_str().unwrap_or_default()
        ),
    }
}

/// Run in every frame of a stranger with a location before its own scripts:
/// takes WebRTC away, since it can reach the network around a proxy. On
/// Linux the engine's own setting turns it off too, and on Windows
/// WebView2 is told to keep it to the proxy.
pub const NO_WEBRTC: &str = r#"(function () {
  var names = ['RTCPeerConnection', 'webkitRTCPeerConnection', 'RTCDataChannel',
    'RTCRtpSender', 'RTCRtpReceiver', 'RTCRtpTransceiver', 'RTCIceCandidate',
    'RTCSessionDescription', 'RTCDtlsTransport', 'RTCIceTransport', 'RTCSctpTransport'];
  for (var i = 0; i < names.length; i++) {
    try { delete window[names[i]]; } catch (e) { }
    try { Object.defineProperty(window, names[i], { value: undefined, writable: false, configurable: false }); } catch (e) { }
  }
})();"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(s: &str) -> String {
        show(parse_location(s).unwrap().as_ref())
    }

    #[test]
    fn locations() {
        assert_eq!(parse_location("").unwrap(), None);
        assert_eq!(parse_location("  Direct ").unwrap(), None);
        assert_eq!(ok("proxy.example:8080"), "http://proxy.example:8080");
        assert_eq!(
            ok("http://proxy.example:8080/"),
            "http://proxy.example:8080"
        );
        assert_eq!(ok("socks5://10.0.0.2:1080"), "socks5://10.0.0.2:1080");
        assert_eq!(
            ok("SOCKS5://Proxy.Example:1080"),
            "socks5://proxy.example:1080"
        );
        assert_eq!(
            ok("socks5://bücher.example:1080"),
            "socks5://xn--bcher-kva.example:1080"
        );
        assert_eq!(ok("socks5://ex%41mple:1080"), "socks5://example:1080");
        // Not taken.
        for bad in [
            "proxy.example",
            "https://proxy.example:443",
            "socks4://p:1080",
            "ftp://p:21",
            "http://user@p:8080",
            "http://:pass@p:8080",
            "http://a%3Ab:pass@p:8080",
            "socks5://u:%00@p:1080",
            "socks5://u:%FF@p:1080",
            "http://[::1]:8080",
            "http://p:8080/path",
            "http://p:8080/?x=1",
            "http://p:8080/#x",
            "http://p:0",
            "two words:80",
            "http://p:8080 --remote-debugging-port=1",
            "file:///etc/passwd",
            "socks5://a_b!:1080",
            "http://p:99999",
        ] {
            assert!(parse_location(bad).is_err(), "{bad} was taken");
        }
    }

    #[test]
    fn credentials_are_kept_but_never_shown() {
        let l = parse_location("socks5://Ann:p%40ss%20w0rd!@Proxy.Example:1080")
            .unwrap()
            .unwrap();
        assert_eq!(
            credentials(&l),
            Some(("Ann".to_string(), "p@ss w0rd!".to_string()))
        );
        assert_eq!(show(Some(&l)), "socks5://proxy.example:1080");
        assert_eq!(show_user(Some(&l)), "socks5://Ann@proxy.example:1080");
        let f = full(Some(&l));
        assert!(!f.contains(' ') && f.contains("p%40ss%20w0rd%21"), "{f}");
        // The full form reads back as the same location.
        assert_eq!(parse_location(&f).unwrap().unwrap(), l);
        // Same credentials, other spelling: the same location.
        assert_eq!(
            parse_location("socks5://Ann:p%40ss%20w0rd%21@proxy.example:1080")
                .unwrap()
                .unwrap(),
            l
        );
        let h = parse_location("http://u-1:b%3Ac@p:8080").unwrap().unwrap();
        assert_eq!(credentials(&h).unwrap().1, "b:c");
        assert!(needs_relay(&h));
        let plain = parse_location("p:8080").unwrap().unwrap();
        assert_eq!(credentials(&plain), None);
        assert_eq!(needs_relay(&plain), cfg!(target_os = "macos"));
        assert_eq!(full(Some(&plain)), "http://p:8080");
        let long = "x".repeat(256);
        assert!(parse_location(&format!("socks5://{long}:p@h:1")).is_err());
        assert!(parse_location(&format!("socks5://u:{}@h:1", "y".repeat(255))).is_ok());
    }

    #[test]
    fn switches_carry_only_the_checked_address() {
        let l = parse_location("socks5://p.example:1080").unwrap();
        let s = webview2_switches(l.as_ref());
        assert!(s.contains("--proxy-server=socks5://p.example:1080 "));
        assert!(s.contains("--proxy-bypass-list=<-loopback>"));
        assert!(s.contains("disable_non_proxied_udp"));
        assert!(s.contains("\"--host-resolver-rules=MAP * ~NOTFOUND , EXCLUDE p.example\""));
        assert_eq!(webview2_switches(None), "");
    }
}
