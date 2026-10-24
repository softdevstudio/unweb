//! A stranger's location: a proxy the person supplies, so the stranger's
//! traffic leaves from somewhere else.
//!
//! A location is `http://host:port` (an HTTP proxy) or `socks5://host:port`
//! (a SOCKS5 proxy). Everything the stranger loads goes through it, local
//! addresses included, and when the proxy can't be reached the stranger
//! loads nothing rather than going direct. WebRTC is turned off in a
//! stranger with a location, since it can reach the network around a proxy.
//! Proxies that need a user name and password aren't supported yet.

use tauri::Url;

/// What someone typed in a strip's location field, as a proxy address, or
/// None for "direct" (empty). `host:port` alone means an HTTP proxy.
pub fn parse_location(text: &str) -> Result<Option<Url>, String> {
    let text = text.trim();
    if text.is_empty() || text.eq_ignore_ascii_case("direct") {
        return Ok(None);
    }
    if text.len() > 300 || text.contains(char::is_whitespace) {
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
    if !url.username().is_empty() || url.password().is_some() {
        return Err("Proxies with a user name and password aren't supported yet".into());
    }
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
    Url::parse(&format!("{scheme}://{host}:{port}"))
        .map(Some)
        .map_err(|_| HELP.to_string())
}

const HELP: &str = "Type a proxy address, like socks5://proxy.example:1080 or proxy.example:8080";

/// How a location is written in a strip and a saved setup.
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
            "http://user:pass@p:8080",
            "http://user@p:8080",
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

/// macOS: gives a stranger's page its proxy with fallback to a direct
/// connection turned off. Apple's proxy configuration lets a connection go
/// direct when the proxy can't be reached unless told otherwise, and wry
/// doesn't tell it (the self-test caught a stranger reaching the site
/// directly with its proxy down). This replaces wry's configuration on the
/// page's own data store before its first request. An error means the
/// stranger must not be used.
#[cfg(target_os = "macos")]
pub fn macos_no_failover(page: &tauri::Webview, location: &Url) -> Result<(), String> {
    use objc2_foundation::NSObjectNSKeyValueCoding;
    use std::ffi::{c_char, CString};
    type Obj = *mut objc2_foundation::NSObject;
    #[link(name = "Network", kind = "framework")]
    unsafe extern "C" {
        fn nw_endpoint_create_host(host: *const c_char, port: *const c_char) -> Obj;
        fn nw_proxy_config_create_socksv5(endpoint: Obj) -> Obj;
        fn nw_proxy_config_create_http_connect(endpoint: Obj, tls: Obj) -> Obj;
        fn nw_proxy_config_set_failover_allowed(config: Obj, allowed: bool);
    }
    let fail = || "Couldn't give this stranger its proxy".to_string();
    let host = CString::new(location.host_str().unwrap_or_default()).map_err(|_| fail())?;
    let port = CString::new(location.port().unwrap_or_default().to_string()).map_err(|_| fail())?;
    let socks = location.scheme() == "socks5";
    let (tx, rx) = std::sync::mpsc::channel::<bool>();
    page.with_webview(move |pv| unsafe {
        let done = (|| {
            let wv = (pv.inner() as *const objc2_web_kit::WKWebView).as_ref()?;
            let endpoint = nw_endpoint_create_host(host.as_ptr(), port.as_ptr());
            if endpoint.is_null() {
                return None;
            }
            let config = if socks {
                nw_proxy_config_create_socksv5(endpoint)
            } else {
                nw_proxy_config_create_http_connect(endpoint, std::ptr::null_mut())
            };
            if config.is_null() {
                return None;
            }
            nw_proxy_config_set_failover_allowed(config, false);
            let proxies: objc2::rc::Retained<
                objc2_foundation::NSArray<objc2_foundation::NSObject>,
            > = objc2_foundation::NSArray::arrayWithObject(&*config);
            wv.configuration().websiteDataStore().setValue_forKey(
                Some(&proxies),
                objc2_foundation::ns_string!("proxyConfigurations"),
            );
            Some(())
        })();
        let _ = tx.send(done.is_some());
    })
    .map_err(|_| fail())?;
    match rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(true) => Ok(()),
        _ => Err(fail()),
    }
}
