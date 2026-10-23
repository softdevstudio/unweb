//! End-to-end self-test: `unweb --selftest <dir>`.
//!
//! Runs the real app on this system's web engine against two small local
//! servers: a site at 127.0.0.1 and a third party at localhost, which
//! counts as another site. Both log every request with its Cookie header
//! and whether it asked to revalidate a cached copy. The logs and the pages'
//! own reports decide each check.
//!
//! 1. Stranger A opens /set, which leaves everything a site can leave: a
//!    server cookie (HttpOnly), a script cookie, localStorage, IndexedDB, a
//!    Cache API entry, a service worker, a file in the HTTP cache, a cookie
//!    and localStorage key from a frame on the other site, and a WebGL
//!    shader no one has compiled before.
//! 2. A opens /check: it must find all of it (the control, so the checks
//!    that follow can't pass by accident).
//! 3. Stranger B, open at the same time, opens /check: it must find none of
//!    it, send none of A's cookies, and fetch the cached file afresh. The
//!    frame's traces are judged only where A's control could see them.
//! 4. A closes: its data folder must be deleted and stay deleted.
//! 5. Stranger C, opened after A closed, opens /check: nothing.
//! 6. Every stranger closes: no folder may be left, and no file may appear
//!    anywhere else in Unweb's data, local data or cache folders.
//! 7. On Linux, the GPU shader caches every program shares (Mesa's and
//!    NVIDIA's, in the user's cache folder) must not have changed.
//!
//! Results go to `<dir>/selftest.json`; the exit code is 0 only if every
//! check passed.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Manager, Url};

use crate::strangers::{Strangers, USES_FOLDERS};

const OVERALL: Duration = Duration::from_secs(240);

/// The folder given after `--selftest`, if any.
pub fn requested_dir() -> Option<PathBuf> {
    let args: Vec<String> = std::env::args().collect();
    let i = args.iter().position(|a| a == "--selftest")?;
    Some(PathBuf::from(
        args.get(i + 1)
            .cloned()
            .unwrap_or_else(|| "selftest-out".into()),
    ))
}

#[derive(Debug, Clone, Serialize)]
struct Hit {
    path: String,
    query: String,
    cookie: String,
    revalidate: bool,
    /// The If-None-Match header, if any (an ETag handed out earlier).
    etag: String,
    /// When the request arrived.
    #[serde(skip)]
    at: Option<Instant>,
}

#[derive(Clone)]
struct Server {
    port: u16,
    hits: Arc<Mutex<Vec<Hit>>>,
}

impl Server {
    fn start(site: bool, other: Arc<Mutex<String>>) -> std::io::Result<Server> {
        // Both listen on 127.0.0.1; the other one is addressed as localhost,
        // which counts as a different site.
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        let hits = Arc::new(Mutex::new(Vec::new()));
        let h = hits.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let h = h.clone();
                let other = other.clone();
                std::thread::spawn(move || {
                    let _ = serve(stream, site, &h, &other);
                });
            }
        });
        Ok(Server { port, hits })
    }

    fn hits(&self) -> Vec<Hit> {
        self.hits.lock().map(|h| h.clone()).unwrap_or_default()
    }

    /// The last report a page sent for `step` and `who`, as key-value pairs.
    fn report(&self, step: &str, who: &str) -> Option<BTreeMap<String, String>> {
        self.hits()
            .iter()
            .rev()
            .filter(|h| h.path == "/report")
            .map(|h| parse_query(&h.query))
            .find(|q| {
                q.get("step").map(String::as_str) == Some(step)
                    && q.get("who").map(String::as_str) == Some(who)
            })
    }
}

fn parse_query(q: &str) -> BTreeMap<String, String> {
    q.split('&')
        .filter_map(|kv| {
            let mut it = kv.splitn(2, '=');
            Some((it.next()?.to_string(), it.next().unwrap_or("").to_string()))
        })
        .collect()
}

fn serve(
    mut stream: TcpStream,
    site: bool,
    hits: &Mutex<Vec<Hit>>,
    other: &Mutex<String>,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let target = line.split_whitespace().nth(1).unwrap_or("/").to_string();
    let mut cookie = String::new();
    let mut revalidate = false;
    let mut etag = String::new();
    let mut length = 0usize;
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h)? == 0 || h == "\r\n" || h == "\n" {
            break;
        }
        let lower = h.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("cookie:") {
            cookie = v.trim().to_string();
        }
        if lower.starts_with("if-none-match:") || lower.starts_with("if-modified-since:") {
            revalidate = true;
        }
        if let Some(v) = h
            .get("if-none-match:".len()..)
            .filter(|_| lower.starts_with("if-none-match:"))
        {
            etag = v.trim().to_string();
        }
        if let Some(v) = lower.strip_prefix("content-length:") {
            length = v.trim().parse().unwrap_or(0);
        }
    }
    if length > 0 {
        let mut body = vec![0u8; length.min(1 << 20)];
        let _ = std::io::Read::read_exact(&mut reader, &mut body);
    }
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (target.clone(), String::new()),
    };
    if let Ok(mut h) = hits.lock() {
        h.push(Hit {
            path: path.clone(),
            query: query.clone(),
            cookie,
            revalidate,
            etag: etag.clone(),
            at: Some(Instant::now()),
        });
    }
    let other = other.lock().map(|o| o.clone()).unwrap_or_default();
    let who = parse_query(&query).get("who").cloned().unwrap_or_default();
    let mut extra = String::new();
    let (status, kind, body) = match (site, path.as_str()) {
        (true, "/set") => {
            extra.push_str("Set-Cookie: unweb_http=1; Max-Age=3600; Path=/; HttpOnly\r\n");
            ("200 OK", "text/html", set_page(&who, &other))
        }
        (true, "/check") => ("200 OK", "text/html", check_page(&who, &other)),
        (true, "/cached.js") => {
            extra.push_str("Cache-Control: public, max-age=3600, immutable\r\nETag: \"unweb-1\"\r\nLast-Modified: Thu, 01 Oct 2026 00:00:00 GMT\r\n");
            (
                "200 OK",
                "text/javascript",
                "window.unwebCached = 1;".to_string(),
            )
        }
        (true, "/sw.js") => ("200 OK", "text/javascript", SW.to_string()),
        // An ETag used as an ID: the file must be checked every time
        // (no-cache), and a browser that kept it sends the ID back.
        (true, "/etag.js") if !etag.is_empty() => {
            extra.push_str("Cache-Control: no-cache\r\n");
            extra.push_str(&format!("ETag: {etag}\r\n"));
            ("304 Not Modified", "text/javascript", String::new())
        }
        (true, "/etag.js") => {
            extra.push_str("Cache-Control: no-cache\r\n");
            extra.push_str(&format!("ETag: \"id-{}\"\r\n", random_tag()));
            (
                "200 OK",
                "text/javascript",
                "window.unwebTagged = 1;".to_string(),
            )
        }
        (false, "/frame-set") => {
            extra.push_str("Set-Cookie: unweb_frame=1; Max-Age=3600; Path=/; SameSite=None\r\n");
            ("200 OK", "text/html", FRAME_SET.to_string())
        }
        (false, "/frame-check") => ("200 OK", "text/html", FRAME_CHECK.to_string()),
        (true, "/nav1") | (true, "/popup") | (true, "/barnav") => (
            "200 OK",
            "text/html",
            format!("<!doctype html><title>{path}</title><p>{path}</p>"),
        ),
        // A page that takes over the page's own Back and Forward.
        (true, "/trap") => ("200 OK", "text/html", TRAP.to_string()),
        _ => ("204 No Content", "text/plain", String::new()),
    };
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}; charset=utf-8\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body.as_bytes())?;
    stream.flush()
}

// The frame on the other site leaves its own cookie and localStorage key,
// and reports what it can see when asked.
const FRAME_SET: &str = r#"<!doctype html><script>
try { document.cookie = 'unweb_frame_js=1; max-age=3600; path=/; SameSite=None'; } catch (e) { }
try { localStorage.setItem('unweb_frame_ls', '1'); } catch (e) { }
parent.postMessage('frame-set', '*');
</script>"#;
// A service worker that calls home twice a second for as long as it runs.
const SW: &str = r#"var who = new URL(self.location).searchParams.get('who') || '';
self.addEventListener('install', function () { self.skipWaiting(); });
self.addEventListener('activate', function (e) { e.waitUntil(self.clients.claim()); });
self.addEventListener('message', function () { });
setInterval(function () { fetch('/sw-ping?who=' + who).catch(function () { }); }, 500);"#;

fn random_tag() -> String {
    let mut b = [0u8; 6];
    let _ = getrandom::fill(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

const TRAP: &str = r#"<!doctype html><title>trap</title><script>
history.back = function () { fetch('/trapped?how=back'); };
history.forward = function () { fetch('/trapped?how=forward'); };
history.go = function () { fetch('/trapped?how=go'); };
window.addEventListener('popstate', function () { fetch('/trapped?how=popstate'); });
</script><p>trap</p>"#;
const FRAME_CHECK: &str = r#"<!doctype html><script>
var ls = ''; try { ls = localStorage.getItem('unweb_frame_ls') || ''; } catch (e) { ls = 'error'; }
parent.postMessage('frame:' + (document.cookie.indexOf('unweb_frame') >= 0 ? 1 : 0) + ':' + (ls ? 1 : 0), '*');
</script>"#;

fn set_page(who: &str, other: &str) -> String {
    format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><title>set {who}</title>
<script src="/cached.js"></script><script src="/etag.js"></script></head><body><h1>Leaving traces as {who}</h1>
<iframe id="f" src="{other}/frame-set"></iframe>
<script>
(async function () {{
  var r = {{ step: 'set', who: '{who}' }};
  function done() {{
    var q = Object.keys(r).map(function (k) {{ return k + '=' + encodeURIComponent(r[k]); }}).join('&');
    fetch('/report?' + q);
  }}
  try {{ document.cookie = 'unweb_js=1; max-age=3600; path=/'; r.cookie = 1; }} catch (e) {{ r.cookie = 'error'; }}
  try {{ localStorage.setItem('unweb_ls', '1'); r.ls = 1; }} catch (e) {{ r.ls = 'error'; }}
  try {{
    await new Promise(function (ok, bad) {{
      var o = indexedDB.open('unweb_db', 1);
      o.onupgradeneeded = function () {{ o.result.createObjectStore('s'); }};
      o.onsuccess = function () {{
        var t = o.result.transaction('s', 'readwrite');
        t.objectStore('s').put('1', 'k');
        t.oncomplete = function () {{ o.result.close(); ok(); }};
        t.onerror = bad;
      }};
      o.onerror = bad;
    }});
    r.idb = 1;
  }} catch (e) {{ r.idb = 'error'; }}
  try {{ var c = await caches.open('unweb_cache'); await c.put('/cache-api-item', new Response('1')); r.cacheapi = 1; }} catch (e) {{ r.cacheapi = 'error'; }}
  try {{ await navigator.serviceWorker.register('/sw.js?who={who}'); await navigator.serviceWorker.ready; r.sw = 1; }} catch (e) {{ r.sw = 'error'; }}
  r.cachedjs = window.unwebCached ? 1 : 0;
  // Compile a shader no one has compiled before, so a shared GPU shader
  // cache would gain an entry.
  try {{
    var gl = document.createElement('canvas').getContext('webgl');
    var src = 'precision mediump float; void main() {{ gl_FragColor = vec4(' + Math.random().toFixed(8) + ', 0.5, 0.25, 1.0); }}';
    var sh = gl.createShader(gl.FRAGMENT_SHADER);
    gl.shaderSource(sh, src);
    gl.compileShader(sh);
    var vs = gl.createShader(gl.VERTEX_SHADER);
    gl.shaderSource(vs, 'attribute vec4 p; void main() {{ gl_Position = p; }}');
    gl.compileShader(vs);
    var pr = gl.createProgram();
    gl.attachShader(pr, vs); gl.attachShader(pr, sh); gl.linkProgram(pr);
    r.webgl = gl.getProgramParameter(pr, gl.LINK_STATUS) ? 1 : 0;
  }} catch (e) {{ r.webgl = 'none'; }}
  var framed = new Promise(function (ok) {{
    window.addEventListener('message', function (m) {{ if (m.data === 'frame-set') ok(1); }});
    setTimeout(function () {{ ok(0); }}, 3000);
  }});
  r.frame = await framed;
  done();
}})();
</script></body></html>"#
    )
}

fn check_page(who: &str, other: &str) -> String {
    format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><title>check {who}</title>
<script src="/cached.js"></script><script src="/etag.js"></script></head><body><h1>Looking for traces as {who}</h1>
<script>
(async function () {{
  var r = {{ step: 'check', who: '{who}' }};
  function done() {{
    var q = Object.keys(r).map(function (k) {{ return k + '=' + encodeURIComponent(r[k]); }}).join('&');
    fetch('/report?' + q);
  }}
  // Keep a running service worker awake while this page is open.
  setInterval(function () {{ var c = navigator.serviceWorker && navigator.serviceWorker.controller; if (c) c.postMessage(1); }}, 500);
  r.cookie = document.cookie.indexOf('unweb_js') >= 0 ? 1 : 0;
  try {{ r.ls = localStorage.getItem('unweb_ls') ? 1 : 0; }} catch (e) {{ r.ls = 'error'; }}
  try {{
    r.idb = await new Promise(function (ok) {{
      var found = 1;
      var o = indexedDB.open('unweb_db');
      o.onupgradeneeded = function (e) {{ if (e.oldVersion === 0) found = 0; }};
      o.onsuccess = function () {{
        var db = o.result;
        var has = found && db.objectStoreNames.contains('s');
        db.close();
        if (!found) indexedDB.deleteDatabase('unweb_db');
        ok(has ? 1 : 0);
      }};
      o.onerror = function () {{ ok('error'); }};
    }});
  }} catch (e) {{ r.idb = 'error'; }}
  try {{
    var keys = await caches.keys();
    r.cacheapi = keys.indexOf('unweb_cache') >= 0 ? 1 : 0;
  }} catch (e) {{ r.cacheapi = 'error'; }}
  try {{ r.sw = (await navigator.serviceWorker.getRegistrations()).length > 0 ? 1 : 0; }} catch (e) {{ r.sw = 'error'; }}
  var f = document.createElement('iframe');
  f.src = '{other}/frame-check';
  var framed = new Promise(function (ok) {{
    window.addEventListener('message', function (m) {{ if (typeof m.data === 'string' && m.data.indexOf('frame:') === 0) ok(m.data.slice(6)); }});
    setTimeout(function () {{ ok('none'); }}, 3000);
  }});
  document.body.appendChild(f);
  var fr = (await framed).split(':');
  r.frame_cookie = fr[0];
  r.frame_ls = fr[1] || '';
  done();
}})();
</script></body></html>"#
    )
}

#[derive(Debug, Serialize)]
struct Check {
    name: &'static str,
    pass: bool,
    /// Not run here (it needs something only CI sets up).
    skipped: bool,
    detail: String,
}

fn check(name: &'static str, pass: bool, detail: String) -> Check {
    Check {
        name,
        pass,
        skipped: false,
        detail,
    }
}

#[derive(Serialize)]
struct Outcome {
    os: &'static str,
    arch: &'static str,
    user_agent: String,
    folders: bool,
    passed: bool,
    checks: Vec<Check>,
    site_hits: Vec<Hit>,
    other_hits: Vec<Hit>,
    hsts_seen: Vec<crate::hsts::Seen>,
    wipes: Vec<crate::strangers::Wipe>,
    seconds: f64,
}

/// Starts the servers and the checker thread. Returns where the first
/// stranger should open.
pub fn start(
    app: &AppHandle,
    strangers: Arc<Strangers>,
    dir: PathBuf,
) -> Result<Url, Box<dyn std::error::Error>> {
    let site_base = Arc::new(Mutex::new(String::new()));
    let other_base = Arc::new(Mutex::new(String::new()));
    let site = Server::start(true, other_base.clone())?;
    let other = Server::start(false, site_base.clone())?;
    let site_url = format!("http://127.0.0.1:{}", site.port);
    let other_url = format!("http://localhost:{}", other.port);
    *site_base.lock().unwrap() = site_url.clone();
    *other_base.lock().unwrap() = other_url;
    let first = Url::parse(&format!("{site_url}/set?who=A"))?;
    let handle = app.clone();
    let shaders_before = shader_cache_snapshot();
    let normal_before = normal_browser_snapshot();
    let hsts = std::env::var_os("UNWEB_TEST_TLS").map(|d| crate::hsts::Hsts::start(Path::new(&d)));
    std::thread::spawn(move || {
        let started = Instant::now();
        let outcome = run(
            &handle,
            &strangers,
            Servers {
                site: &site,
                other: &other,
                hsts: hsts.as_ref(),
            },
            &site_url,
            started,
            (shaders_before, normal_before),
        );
        let passed = outcome.passed;
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(
            dir.join("selftest.json"),
            serde_json::to_string_pretty(&outcome).unwrap_or_default(),
        );
        for c in &outcome.checks {
            println!(
                "{} {}: {}",
                match (c.skipped, c.pass) {
                    (true, true) => "SKIP",
                    (_, true) => "PASS",
                    (_, false) => "FAIL",
                },
                c.name,
                c.detail
            );
        }
        println!(
            "selftest {}{} in {:.1}s on {} ({})",
            if passed { "passed" } else { "FAILED" },
            match outcome.checks.iter().filter(|c| c.skipped).count() {
                0 => String::new(),
                n => format!(" with {n} check(s) not run"),
            },
            outcome.seconds,
            outcome.os,
            outcome.user_agent
        );
        handle.exit(if passed { 0 } else { 1 });
    });
    Ok(first)
}

fn wait_until(deadline: Instant, mut f: impl FnMut() -> bool) -> bool {
    while Instant::now() < deadline {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    f()
}

/// Runs `js` in the webview `label` (a page or a bar) and returns the
/// string it evaluates to.
fn eval_string(app: &AppHandle, label: &str, js: &str) -> String {
    let Some(w) = app.get_webview(label) else {
        return String::new();
    };
    let (tx, rx) = std::sync::mpsc::channel();
    if w.eval_with_callback(js, move |out| {
        let _ = tx.send(out);
    })
    .is_err()
    {
        return String::new();
    }
    rx.recv_timeout(Duration::from_secs(5))
        .ok()
        .and_then(|s| serde_json::from_str::<String>(&s).ok())
        .unwrap_or_default()
}

fn navigate(app: &AppHandle, label: &str, url: &str) {
    if let (Some(w), Ok(u)) = (
        app.get_webview(&crate::window::page_label(label)),
        Url::parse(url),
    ) {
        let _ = w.navigate(u);
    }
}

fn close(app: &AppHandle, strangers: &Strangers, label: &str) {
    strangers.retire(app, label);
}

/// The page's address, as the page itself sees it.
fn page_address(app: &AppHandle, window: &str) -> String {
    eval_string(
        app,
        &crate::window::page_label(window),
        "String(location.href)",
    )
}

/// Asks the bar of `window` to run one of its commands, the way its
/// buttons do.
fn bar_call(app: &AppHandle, window: &str, name: &str, args: &str) {
    let _ = eval_string(
        app,
        &crate::window::bar_label(window),
        &format!("window.__TAURI_INTERNALS__.invoke('{name}', {args}); 'sent'"),
    );
}

fn hits_on(site: &Server, path: &str) -> usize {
    site.hits().iter().filter(|h| h.path == path).count()
}

/// Width and height of a webview's content, as it reports them.
fn inner_size_of(app: &AppHandle, label: &str) -> (f64, f64) {
    let v = eval_string(app, label, "innerWidth + 'x' + innerHeight");
    let mut it = v.split('x').map(|n| n.parse::<f64>().unwrap_or(-1.0));
    (it.next().unwrap_or(-1.0), it.next().unwrap_or(-1.0))
}

/// The window's content size in logical pixels.
fn window_size(app: &AppHandle, window: &str) -> (f64, f64) {
    app.get_window(window)
        .and_then(|w| {
            let s = w.inner_size().ok()?;
            let f = w.scale_factor().ok()?;
            Some((s.width as f64 / f, s.height as f64 / f))
        })
        .unwrap_or((-1.0, -1.0))
}

/// Checks the bar and the window of an open stranger (C).
fn bar_checks(
    app: &AppHandle,
    strangers: &Strangers,
    site: &Server,
    site_url: &str,
    w: &str,
) -> Vec<Check> {
    use crate::window::{bar_label, page_label, BAR_HEIGHT};
    let mut out = Vec::new();

    // The bar shows the page's address.
    let page_url = page_address(app, w);
    let shows = wait_until(Instant::now() + Duration::from_secs(5), || {
        eval_string(
            app,
            &bar_label(w),
            "document.getElementById('address').value",
        ) == page_url
    });
    out.push(check(
        "the bar shows the page's address",
        shows && page_url.starts_with("http"),
        format!(
            "page at {page_url}; bar shows {:?}",
            eval_string(
                app,
                &bar_label(w),
                "document.getElementById('address').value"
            )
        ),
    ));

    // Typing an address, then Back and Forward past a page that took over
    // history.back, history.go and popstate.
    let before = hits_on(site, "/nav1");
    bar_call(app, w, "go", &format!("{{ text: '{site_url}/nav1' }}"));
    let went = wait_until(Instant::now() + Duration::from_secs(10), || {
        page_address(app, w).ends_with("/nav1") && hits_on(site, "/nav1") > before
    });
    bar_call(app, w, "go", &format!("{{ text: '{site_url}/trap' }}"));
    let trapped_page = wait_until(Instant::now() + Duration::from_secs(10), || {
        page_address(app, w).ends_with("/trap")
    });
    std::thread::sleep(Duration::from_millis(500));
    bar_call(app, w, "back", "{}");
    let backed = wait_until(Instant::now() + Duration::from_secs(10), || {
        page_address(app, w).ends_with("/nav1")
    });
    let bar_after_back = wait_until(Instant::now() + Duration::from_secs(5), || {
        eval_string(
            app,
            &bar_label(w),
            "document.getElementById('address').value",
        )
        .ends_with("/nav1")
    });
    bar_call(app, w, "forward", "{}");
    let forwarded = wait_until(Instant::now() + Duration::from_secs(10), || {
        page_address(app, w).ends_with("/trap")
    });
    std::thread::sleep(Duration::from_millis(500));
    let trapped = site
        .hits()
        .iter()
        .filter(|h| h.path == "/trapped")
        .map(|h| h.query.clone())
        .collect::<Vec<_>>();
    let reload_before = site.hits().iter().filter(|h| h.path == "/trap").count();
    bar_call(app, w, "reload", "{}");
    let reloaded = wait_until(Instant::now() + Duration::from_secs(10), || {
        site.hits().iter().filter(|h| h.path == "/trap").count() > reload_before
    });
    out.push(check(
        "the bar's address, back, forward and reload work, and a page can't trap them",
        went && trapped_page && backed && bar_after_back && forwarded && reloaded
            && trapped.iter().all(|q| q.contains("popstate")),
        format!(
            "typed address opened: {went}; back reached /nav1: {backed} (bar followed: {bar_after_back}); forward reached /trap: {forwarded}; reload fetched it again: {reloaded}; the page's own history functions called: {:?}",
            trapped
                .iter()
                .filter(|q| !q.contains("popstate"))
                .collect::<Vec<_>>()
        ),
    ));

    // Nothing can load a page in the bar (a link dropped on it, say).
    let barnav_before = hits_on(site, "/barnav");
    // A web address, and the app's own host name on the test server's
    // port (engines send *.localhost to this machine).
    let port = site_url.rsplit(':').next().unwrap_or("0");
    let mut bar_urls = Vec::new();
    for target in [
        format!("{site_url}/barnav"),
        format!("http://tauri.localhost:{port}/barnav"),
    ] {
        let _ = eval_string(
            app,
            &bar_label(w),
            &format!("location.href = '{target}'; 'sent'"),
        );
        std::thread::sleep(Duration::from_secs(2));
        bar_urls.push(eval_string(app, &bar_label(w), "String(location.href)"));
    }
    let bar_url = bar_urls.join(", then ");
    // WebView2 can send the request before the refusal cancels it; what
    // counts is that the bar never shows anything but itself.
    let bar_kept = bar_urls.iter().all(|u| u.ends_with("/bar.html"))
        && bar_urls.iter().all(|u| !u.contains(&format!(":{port}/")));
    out.push(check(
        "nothing can load another page in the bar",
        bar_kept,
        format!(
            "after the bar was sent to another address it shows {bar_url}; requests the engine sent for that address before refusing it: {}",
            hits_on(site, "/barnav") - barnav_before
        ),
    ));

    // A page can't call the app, and its popups stay in the stranger.
    let windows_before = app.windows().len();
    let _ = eval_string(
        app,
        &page_label(w),
        "(function(){var i=window.__TAURI_INTERNALS__;var tell=function(n,r){fetch('/cmd?name='+n+'&r='+r)};\
         ['new_stranger','go','reload'].forEach(function(n){try{i.invoke(n,{text:'example.com'}).then(function(){tell(n,'ran')},function(){tell(n,'refused')})}catch(e){tell(n,'refused')}});\
         return 'sent'})()",
    );
    let answered = wait_until(Instant::now() + Duration::from_secs(10), || {
        site.hits().iter().filter(|h| h.path == "/cmd").count() >= 3
    });
    let cmds: Vec<String> = site
        .hits()
        .iter()
        .filter(|h| h.path == "/cmd")
        .map(|h| h.query.clone())
        .collect();
    // What a page can learn about its stranger: Tauri puts the window's
    // label in the page, so it must be random.
    let seen_label = eval_string(
        app,
        &page_label(w),
        "(function(){try{return String(window.__TAURI_INTERNALS__.metadata.currentWindow.label)}catch(e){return ''}})()",
    );
    let label_random = seen_label
        .strip_prefix(crate::strangers::PREFIX)
        .is_some_and(|r| r.len() == 16 && r.bytes().all(|b| b.is_ascii_hexdigit()));
    let popup_before = hits_on(site, "/popup");
    let _ = eval_string(
        app,
        &page_label(w),
        &format!("window.open('{site_url}/popup', '_blank'); 'sent'"),
    );
    let popup_here = wait_until(Instant::now() + Duration::from_secs(10), || {
        page_address(app, w).ends_with("/popup") && hits_on(site, "/popup") > popup_before
    });
    std::thread::sleep(Duration::from_millis(800));
    let windows_after = app.windows().len();
    out.push(check(
        "a page can't call the app, and its popups stay in the same stranger",
        answered
            && cmds.iter().all(|q| q.contains("r=refused"))
            && popup_here
            && windows_after == windows_before
            && label_random,
        format!(
            "the page's calls: {cmds:?}; window.open opened in this stranger's page: {popup_here}; windows before {windows_before}, after {windows_after}; the window name the page can read is random: {label_random} ({seen_label})"
        ),
    ));

    // The bar keeps its height and the page fills the rest, before and
    // after a resize.
    let mut sizes = Vec::new();
    let mut fits = true;
    for size in [None, Some((900.0, 600.0))] {
        if let (Some((sw, sh)), Some(win)) = (size, app.get_window(w)) {
            let _ = win.set_size(tauri::LogicalSize::new(sw, sh));
            std::thread::sleep(Duration::from_millis(1200));
        }
        let (ww, wh) = window_size(app, w);
        let (bw, bh) = inner_size_of(app, &bar_label(w));
        let (pw, ph) = inner_size_of(app, &page_label(w));
        let ok = (bh - BAR_HEIGHT).abs() <= 2.0
            && (ph - (wh - BAR_HEIGHT)).abs() <= 3.0
            && (bw - ww).abs() <= 3.0
            && (pw - ww).abs() <= 3.0;
        fits &= ok;
        sizes.push(format!("window {ww}x{wh}: bar {bw}x{bh}, page {pw}x{ph}"));
    }
    out.push(check(
        "the bar keeps its height and the page fills the rest",
        fits,
        sizes.join("; "),
    ));

    // New stranger from the bar: a separate window with a store of its own.
    let before: Vec<String> = strangers.open_labels();
    bar_call(app, w, "new_stranger", "{}");
    let appeared = wait_until(Instant::now() + Duration::from_secs(10), || {
        strangers.open_labels().len() > before.len()
    });
    let new_label = strangers
        .open_labels()
        .into_iter()
        .find(|l| !before.contains(l));
    let own_store = match (&new_label, USES_FOLDERS) {
        (Some(l), true) => {
            strangers.folder(l).is_some() && strangers.folder(l) != strangers.folder(w)
        }
        // macOS: every webview gets a fresh in-memory store; the clean
        // stranger checks are what show it.
        (Some(_), false) => true,
        (None, _) => false,
    };
    let start_page = new_label
        .as_ref()
        .map(|l| {
            wait_until(Instant::now() + Duration::from_secs(10), || {
                eval_string(app, &page_label(l), "document.title") == "Unweb"
            })
        })
        .unwrap_or(false);
    out.push(check(
        "New stranger opens another stranger with a store of its own",
        appeared && own_store && start_page,
        format!(
            "opened: {}; its own store: {}; it shows the start page: {start_page}",
            new_label.as_deref().unwrap_or("nothing"),
            if USES_FOLDERS {
                own_store.to_string()
            } else {
                "not judged here (each webview gets its own in-memory store on this system)"
                    .to_string()
            }
        ),
    ));
    out
}

/// Every trace a /check report can find, by key.
const TRACES: [&str; 5] = ["cookie", "ls", "idb", "cacheapi", "sw"];

fn describe(r: &BTreeMap<String, String>) -> String {
    let mut v: Vec<String> = TRACES
        .iter()
        .chain(["frame_cookie", "frame_ls"].iter())
        .map(|k| format!("{k}={}", r.get(*k).map(String::as_str).unwrap_or("?")))
        .collect();
    v.sort();
    v.join(" ")
}

/// The /check hits made by `who`'s page and the cached file it loaded.
fn requests_of(server: &Server, who: &str) -> (Vec<Hit>, Vec<Hit>) {
    let hits = server.hits();
    // Requests between this stranger's /check and its report belong to it.
    let start = hits.iter().rposition(|h| {
        h.path == "/check" && parse_query(&h.query).get("who").map(String::as_str) == Some(who)
    });
    let Some(start) = start else {
        return (Vec::new(), Vec::new());
    };
    let end = hits[start..]
        .iter()
        .position(|h| {
            h.path == "/report" && parse_query(&h.query).get("who").map(String::as_str) == Some(who)
        })
        .map(|e| start + e + 1)
        .unwrap_or(hits.len());
    let mine: Vec<Hit> = hits[start..end].to_vec();
    let cached: Vec<Hit> = mine
        .iter()
        .filter(|h| h.path == "/cached.js")
        .cloned()
        .collect();
    (mine, cached)
}

/// The frame's traces that stranger A's control found: only those can show
/// anything carrying over. (Engines that block third-party cookies never
/// let the frame's cookie be seen at all, not even by A.)
fn frame_keys_seen(control: &BTreeMap<String, String>) -> Vec<&'static str> {
    ["frame_cookie", "frame_ls"]
        .into_iter()
        .filter(|k| control.get(*k).map(String::as_str) == Some("1"))
        .collect()
}

/// A stranger that must have found nothing.
fn clean_stranger(
    name: &'static str,
    site: &Server,
    who: &str,
    control: &BTreeMap<String, String>,
) -> Check {
    let report = site.report("check", who);
    let (mine, cached) = requests_of(site, who);
    // Requests that are certainly this stranger's carry its name; another
    // stranger's background requests (a service worker's update check, say)
    // can fall in the same window of time.
    let carried_cookie = mine.iter().any(|h| {
        h.cookie.contains("unweb_")
            && parse_query(&h.query).get("who").map(String::as_str) == Some(who)
    });
    let seen: Vec<String> = mine
        .iter()
        .map(|h| {
            format!(
                "{}{}{}",
                h.path,
                if h.cookie.contains("unweb_") {
                    "+cookie"
                } else {
                    ""
                },
                if h.revalidate { "+revalidate" } else { "" }
            )
        })
        .collect();
    let fresh_fetch = cached.len() == 1 && !cached[0].revalidate;
    // An ETag handed to another stranger, sent back here, would be an ID.
    let sent_etag: Vec<String> = mine
        .iter()
        .filter(|h| !h.etag.is_empty())
        .map(|h| format!("{} {}", h.path, h.etag))
        .collect();
    let found: Vec<&str> = report
        .as_ref()
        .map(|r| {
            TRACES
                .iter()
                .copied()
                .filter(|k| r.get(*k).map(String::as_str) != Some("0"))
                .collect()
        })
        .unwrap_or_else(|| TRACES.to_vec());
    let testable = frame_keys_seen(control);
    let frame_found = report
        .as_ref()
        .map(|r| {
            testable
                .iter()
                .any(|k| r.get(*k).map(String::as_str) != Some("0"))
        })
        .unwrap_or(true);
    let untestable: Vec<&str> = ["frame_cookie", "frame_ls"]
        .into_iter()
        .filter(|k| !testable.contains(k))
        .collect();
    check(
        name,
        report.is_some()
            && found.is_empty()
            && !frame_found
            && !carried_cookie
            && fresh_fetch
            && sent_etag.is_empty(),
        format!(
            "page found [{}] (none expected): {}{}; its requests carried an earlier cookie: {carried_cookie}; the cached file was fetched afresh: {fresh_fetch}; ETags sent back: {sent_etag:?}; requests meanwhile: {}",
            found.join(" "),
            report.as_ref().map(describe).unwrap_or_else(|| "no report".into()),
            if untestable.is_empty() {
                String::new()
            } else {
                format!(
                    " ({} not judged: A's own control couldn't see it on this engine)",
                    untestable.join(", ")
                )
            },
            seen.join(" ")
        ),
    )
}

fn files_under(dir: &Path, skip: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.starts_with(skip) {
            continue;
        }
        if p.is_dir() {
            files_under(&p, skip, out);
        } else {
            out.push(p);
        }
    }
}

type FileMark = (PathBuf, u64, Option<std::time::SystemTime>);

/// Every file in the per-user GPU shader caches (Linux: Mesa's and
/// NVIDIA's), with its size and time. Empty elsewhere.
fn shader_cache_snapshot() -> Vec<FileMark> {
    let mut out = Vec::new();
    if !cfg!(target_os = "linux") {
        return out;
    }
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")));
    let Some(cache) = cache else {
        return out;
    };
    let mut files = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&cache) {
        for e in rd.flatten() {
            if e.file_name()
                .to_string_lossy()
                .starts_with("mesa_shader_cache")
            {
                files_under(&e.path(), Path::new("/nonexistent-skip"), &mut files);
            }
        }
    }
    files_under(
        &cache.join("nvidia"),
        Path::new("/nonexistent-skip"),
        &mut files,
    );
    for f in files {
        if let Ok(m) = std::fs::metadata(&f) {
            out.push((f, m.len(), m.modified().ok()));
        }
    }
    out
}

/// Where the folders a normal browser (and the engine, when an app uses its
/// default store) keeps web data are, for this system. Unweb's own folders
/// are checked separately.
fn normal_browser_roots() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default();
    let mut roots = Vec::new();
    let named_unweb = |dir: PathBuf, roots: &mut Vec<PathBuf>| {
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_ascii_lowercase();
                if n.contains("unweb") && n != "org.unweb.app" {
                    roots.push(e.path());
                }
            }
        }
    };
    if cfg!(target_os = "linux") {
        for d in [
            ".local/share/webkitgtk",
            ".cache/webkitgtk",
            ".local/share/epiphany",
            ".cache/epiphany",
            ".config/epiphany",
            ".mozilla",
            ".config/chromium",
            ".config/google-chrome",
        ] {
            roots.push(home.join(d));
        }
        named_unweb(home.join(".local/share"), &mut roots);
        named_unweb(home.join(".cache"), &mut roots);
    } else if cfg!(target_os = "macos") {
        // Safari's web data. (Library/Safari itself holds files that system
        // services open while Safari isn't running, so it isn't watched.)
        for d in [
            "Library/Cookies",
            "Library/WebKit/com.apple.Safari",
            "Library/Containers/com.apple.Safari/Data/Library/Cookies",
        ] {
            roots.push(home.join(d));
        }
        for d in [
            "Library/WebKit",
            "Library/Caches",
            "Library/HTTPStorages",
            "Library/Cookies",
            "Library/Saved Application State",
            "Library/Application Support",
        ] {
            named_unweb(home.join(d), &mut roots);
        }
    } else if cfg!(windows) {
        let local = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_default();
        let roaming = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_default();
        roots.push(local.join("Microsoft/Edge/User Data"));
        // Where WebView2 puts its data when an app names no folder.
        if let Ok(exe) = std::env::current_exe() {
            roots.push(PathBuf::from(format!("{}.WebView2", exe.display())));
        }
        named_unweb(local, &mut roots);
        named_unweb(roaming, &mut roots);
    }
    roots
}

fn normal_browser_snapshot() -> Vec<FileMark> {
    let mut out = Vec::new();
    for root in normal_browser_roots() {
        let mut files = Vec::new();
        if root.is_file() {
            files.push(root);
        } else {
            files_under(&root, Path::new("/nonexistent-skip"), &mut files);
        }
        for f in files {
            if let Ok(m) = std::fs::metadata(&f) {
                out.push((f, m.len(), m.modified().ok()));
            }
        }
    }
    out
}

/// Folders a web engine names after the app (its bundle identifier, or
/// the program's name when it runs unbundled) and could write to.
pub fn engine_app_folders(identifier: &str) -> Vec<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default();
    let mut out = Vec::new();
    for name in [identifier, "unweb"] {
        if cfg!(target_os = "macos") {
            for d in [
                "Library/WebKit",
                "Library/HTTPStorages",
                "Library/Caches",
                "Library/Cookies",
                "Library/Saved Application State",
            ] {
                out.push(home.join(d).join(name));
            }
            out.push(home.join(format!("Library/Saved Application State/{name}.savedState")));
            out.push(home.join(format!("Library/Cookies/{name}.binarycookies")));
        } else if cfg!(target_os = "linux") {
            out.push(home.join(".local/share").join(name));
            out.push(home.join(".cache").join(name));
        } else if cfg!(windows) {
            for var in ["LOCALAPPDATA", "APPDATA"] {
                if let Some(v) = std::env::var_os(var) {
                    out.push(PathBuf::from(v).join(name));
                }
            }
        }
    }
    out
}

/// The test's local servers.
struct Servers<'a> {
    site: &'a Server,
    other: &'a Server,
    /// The HSTS servers, when the test certificate is set up.
    hsts: Option<&'a Result<crate::hsts::Hsts, String>>,
}

/// Sends `window`'s page to the plain HSTS server and returns what that
/// server saw from then on.
fn hsts_probe(
    app: &AppHandle,
    h: &crate::hsts::Hsts,
    window: &str,
    who: &str,
) -> Vec<crate::hsts::Seen> {
    use crate::hsts::Seen;
    let n = h.seen().len();
    navigate(app, window, &h.plain_url(&format!("/probe?who={who}")));
    let tag = format!("who={who}");
    wait_until(Instant::now() + Duration::from_secs(10), || {
        h.seen()[n..]
            .iter()
            .any(|x| *x == Seen::Upgraded || matches!(x, Seen::Plain(p) if p.contains(&tag)))
    });
    std::thread::sleep(Duration::from_millis(1500));
    h.seen()[n..].to_vec()
}

fn sw_pings(site: &Server, who: &str) -> usize {
    sw_pings_after(site, who, None)
}

/// Service worker calls from `who` that arrived after `since`.
fn sw_pings_after(site: &Server, who: &str, since: Option<Instant>) -> usize {
    let tag = format!("who={who}");
    site.hits()
        .iter()
        .filter(|h| h.path == "/sw-ping" && h.query == tag)
        .filter(|h| since.is_none_or(|s| h.at.is_some_and(|a| a > s)))
        .count()
}

fn run(
    app: &AppHandle,
    strangers: &Strangers,
    servers: Servers,
    site_url: &str,
    started: Instant,
    (shaders_before, normal_before): (Vec<FileMark>, Vec<FileMark>),
) -> Outcome {
    use crate::hsts::Seen;
    let (site, other) = (servers.site, servers.other);
    let hsts = servers.hsts.and_then(|h| h.as_ref().ok());
    let deadline = started + OVERALL;
    let mut checks = Vec::new();

    // 1. A leaves traces.
    let set = wait_until(deadline, || site.report("set", "A").is_some());
    std::thread::sleep(Duration::from_millis(1500));
    // A is the stranger opened at start.
    let la = strangers.opened().first().cloned().unwrap_or_default();
    let la = la.as_str();
    let user_agent = eval_string(app, &crate::window::page_label(la), "navigator.userAgent");
    let set_report = site.report("set", "A").unwrap_or_default();
    let frame_cookie_sent = other.hits().iter().any(|h| h.path == "/frame-set");
    checks.push(check(
        "stranger A leaves every kind of trace",
        set && ["cookie", "ls", "idb", "cacheapi", "sw", "cachedjs"]
            .iter()
            .all(|k| set_report.get(*k).map(String::as_str) == Some("1")),
        format!(
            "A's page reported {:?}; the other site's frame loaded: {frame_cookie_sent}",
            set_report
        ),
    ));
    let folder_a = strangers.folder(la);
    let a_files = folder_a
        .as_ref()
        .map(|d| {
            let mut v = Vec::new();
            files_under(d, Path::new("/nonexistent-skip"), &mut v);
            v.len()
        })
        .unwrap_or(0);

    // 2. Control: A finds its own traces.
    navigate(app, la, &format!("{site_url}/check?who=A"));
    let a_checked = wait_until(Instant::now() + Duration::from_secs(20), || {
        site.report("check", "A").is_some()
    });
    let a_report = site.report("check", "A").unwrap_or_default();
    let (a_mine, a_cached) = requests_of(site, "A");
    let a_cookie = a_mine
        .iter()
        .any(|h| h.path == "/check" && h.cookie.contains("unweb_http"));
    let a_cache_used = a_cached.is_empty() || a_cached.iter().all(|h| h.revalidate);
    let a_etag = a_mine
        .iter()
        .any(|h| h.path == "/etag.js" && h.etag.starts_with("\"id-"));
    let a_seen: Vec<String> = a_mine
        .iter()
        .map(|h| {
            format!(
                "{}{}{}",
                h.path,
                if h.cookie.contains("unweb_") {
                    "+cookie"
                } else {
                    ""
                },
                if h.revalidate { "+revalidate" } else { "" }
            )
        })
        .collect();
    let a_found: Vec<&str> = TRACES
        .iter()
        .copied()
        .filter(|k| a_report.get(*k).map(String::as_str) == Some("1"))
        .collect();
    checks.push(check(
        "stranger A finds its own traces (control)",
        // WKWebView's non-persistent store serves no cached copy from one
        // page to the next, so on macOS the HTTP cache can't carry anything
        // over and the control can't show a hit; the checks below then
        // can't fail on the cache, and the detail says so.
        a_checked
            && a_found.len() == TRACES.len()
            && a_cookie
            && ((a_cache_used && a_etag) || !USES_FOLDERS),
        format!(
            "A found [{}]; A sent its server cookie: {a_cookie}; the cached file came from A's cache: {}; A sent back its ETag ID: {a_etag}; frame: cookie {} storage {}{}; requests: {}",
            a_found.join(" "),
            if a_cache_used {
                "true".to_string()
            } else {
                "false (this engine keeps no HTTP cache between a stranger's pages, so the cache can't carry anything over here)".to_string()
            },
            a_report.get("frame_cookie").cloned().unwrap_or_default(),
            a_report.get("frame_ls").cloned().unwrap_or_default(),
            folder_a
                .as_ref()
                .map(|_| format!("; {a_files} files in A's own folder"))
                .unwrap_or_default(),
            a_seen.join(" ")
        ),
    ));

    // HSTS: A learns it from the HTTPS server, and a plain visit from A is
    // then upgraded (the control); B and C must visit plainly.
    let mut hsts_a: Option<(bool, Vec<Seen>)> = None;
    if let Some(h) = hsts {
        navigate(app, la, &h.https_url("/learn?who=A"));
        let learned = wait_until(Instant::now() + Duration::from_secs(10), || {
            h.seen()
                .iter()
                .any(|x| matches!(x, Seen::Https(p) if p.contains("who=A")))
        });
        std::thread::sleep(Duration::from_millis(500));
        hsts_a = Some((learned, hsts_probe(app, h, la, "A")));
        // Back to a page that keeps A's service worker awake.
        navigate(app, la, &format!("{site_url}/check?who=A2"));
        wait_until(Instant::now() + Duration::from_secs(10), || {
            site.report("check", "A2").is_some()
        });
    }

    // 3. B, open at the same time.
    let b = strangers.open(app, Url::parse(&format!("{site_url}/check?who=B")).ok());
    let lb = b.as_ref().cloned().unwrap_or_default();
    let b_ok = b.is_ok()
        && wait_until(Instant::now() + Duration::from_secs(25), || {
            site.report("check", "B").is_some()
        });
    let mut c_b = clean_stranger(
        "a stranger open at the same time finds nothing",
        site,
        "B",
        &a_report,
    );
    if !b_ok {
        c_b.pass = false;
    }
    checks.push(c_b);
    let hsts_b = hsts.map(|h| hsts_probe(app, h, &lb, "B"));

    // 4. A closes and its data goes. Its service worker must stop with it.
    let pings_open = sw_pings(site, "A");
    std::thread::sleep(Duration::from_millis(1500));
    let pings_at_close = sw_pings(site, "A");
    // Closing clears the engine's data first (on Linux), then removes the
    // window; count from the moment the window is gone.
    close(app, strangers, la);
    let pings_gone = sw_pings(site, "A");
    let gone_at = Instant::now();
    let wiped = wait_until(Instant::now() + Duration::from_secs(40), || {
        strangers.wipes().iter().any(|w| w.label == la)
    });
    let wipe_a = strangers.wipes().into_iter().find(|w| w.label == la);
    checks.push(check(
        "a closed stranger's data is deleted",
        wiped
            && wipe_a
                .as_ref()
                .is_some_and(|w| w.gone && (!USES_FOLDERS || w.folder.is_some())),
        match &wipe_a {
            Some(w) => format!(
                "{}{} after {:.1}s",
                w.detail,
                w.folder
                    .as_ref()
                    .map(|f| format!(" ({})", f.display()))
                    .unwrap_or_default(),
                w.seconds
            ),
            None => "no record of A being wiped".into(),
        },
    ));

    // Watch for at least 7 s after the window went (the wipe may already
    // have taken longer).
    if let Some(left) = Duration::from_secs(7).checked_sub(gone_at.elapsed()) {
        std::thread::sleep(left);
    }
    let after_window = sw_pings_after(site, "A", Some(gone_at));
    let after_2s = sw_pings_after(site, "A", Some(gone_at + Duration::from_secs(2)));
    let sw_was_running = pings_at_close > pings_open;
    checks.push(check(
        "a closed stranger's service worker stops",
        // WKWebView lets a worker finish what it is doing for a moment, so
        // calls in the first 2 s after the window goes are counted, not
        // failed. After that there must be none.
        sw_was_running && after_2s == 0,
        format!(
            "A's service worker called home {} times in the 1.5 s before A closed (it was running: {sw_was_running}); {} times while A was closing; {} times in the 2 s after the window went; {after_2s} times from then until {:.0} s after",
            pings_at_close - pings_open,
            pings_gone - pings_at_close,
            after_window - after_2s,
            gone_at.elapsed().as_secs_f64()
        ),
    ));

    // 5. C, opened after A closed.
    let c = strangers.open(app, Url::parse(&format!("{site_url}/check?who=C")).ok());
    let lc = c.as_ref().cloned().unwrap_or_default();
    let c_ok = c.is_ok()
        && wait_until(Instant::now() + Duration::from_secs(25), || {
            site.report("check", "C").is_some()
        });
    let mut c_c = clean_stranger(
        "a new stranger after one closed finds nothing",
        site,
        "C",
        &a_report,
    );
    if !c_ok {
        c_c.pass = false;
    }
    checks.push(c_c);

    // The bar and the window, on stranger C.
    checks.extend(bar_checks(app, strangers, site, site_url, &lc));
    let hsts_c = hsts.map(|h| hsts_probe(app, h, &lc, "C"));
    checks.push(match (servers.hsts, &hsts_a) {
        (Some(Ok(_)), Some((learned, a))) => {
            let a_up = a.contains(&Seen::Upgraded);
            let plain = |v: &Option<Vec<Seen>>, who: &str| {
                v.as_ref().is_some_and(|v| {
                    !v.contains(&Seen::Upgraded)
                        && v.iter().any(|x| matches!(x, Seen::Plain(p) if p.contains(&format!("who={who}"))))
                })
            };
            let (b_plain, c_plain) = (plain(&hsts_b, "B"), plain(&hsts_c, "C"));
            check(
                "HSTS learned by one stranger doesn't reach another",
                *learned && a_up && b_plain && c_plain,
                format!(
                    "A got the HSTS header over HTTPS: {learned}; A's next plain visit was upgraded (control): {a_up}; B visited plainly: {b_plain}; C (opened after A closed) visited plainly: {c_plain}; seen: A {a:?}, B {:?}, C {:?}",
                    hsts_b.unwrap_or_default(),
                    hsts_c.unwrap_or_default()
                ),
            )
        }
        (Some(Err(e)), _) => check(
            "HSTS learned by one stranger doesn't reach another",
            false,
            format!("the HTTPS server didn't start: {e}"),
        ),
        _ => Check {
            skipped: true,
            ..check(
                "HSTS learned by one stranger doesn't reach another",
                std::env::var_os("CI").is_none(),
                "not run: no trusted test certificate (scripts/test-ca.sh, then UNWEB_TEST_TLS); required in CI".into(),
            )
        },
    });

    // 6. Everything closes; nothing may be left.
    let opened = strangers.open_labels();
    for l in &opened {
        close(app, strangers, l);
    }
    let all = wait_until(Instant::now() + Duration::from_secs(60), || {
        let w = strangers.wipes();
        opened.iter().all(|l| w.iter().any(|x| &x.label == l))
    });
    // Engines can write late (a deferred cache record, a cookie flush), so
    // look again after a while.
    std::thread::sleep(Duration::from_secs(8));
    let wipes = strangers.wipes();
    // Everything under the strangers folder must be gone except this run's
    // own lock, which goes when the app exits.
    let root = strangers.base().to_path_buf();
    let mut left_in_root = Vec::new();
    files_under(&root, Path::new("/nonexistent-skip"), &mut left_in_root);
    left_in_root.retain(|p| p != strangers.lock_path());
    // Another copy of Unweb running on this machine keeps its own run.
    let other_runs = left_in_root
        .iter()
        .filter(|p| strangers.belongs_to_other_live_run(p))
        .count();
    left_in_root.retain(|p| !strangers.belongs_to_other_live_run(p));
    let mut elsewhere = Vec::new();
    let paths = app.path();
    let mut ours: Vec<PathBuf> = [
        paths.app_data_dir(),
        paths.app_local_data_dir(),
        paths.app_cache_dir(),
        paths.app_config_dir(),
    ]
    .into_iter()
    .flatten()
    .collect();
    ours.extend(engine_app_folders(&app.config().identifier));
    for d in ours {
        if d.is_file() {
            elsewhere.push(d.clone());
        }
        files_under(&d, &root, &mut elsewhere);
    }
    elsewhere.sort();
    elsewhere.dedup();
    checks.push(check(
        "nothing is left on disk",
        all && wipes.iter().all(|w| w.gone) && left_in_root.is_empty() && elsewhere.is_empty(),
        format!(
            "{} strangers wiped ({}); files left in the strangers folder: {:?}{}; files anywhere else in Unweb's folders: {}",
            wipes.len(),
            wipes
                .iter()
                .map(|w| format!("{} {}", w.label, if w.gone { "gone" } else { "NOT gone" }))
                .collect::<Vec<_>>()
                .join(", "),
            left_in_root.iter().take(10).map(|p| p.strip_prefix(&root).unwrap_or(p).display().to_string()).collect::<Vec<_>>(),
            if other_runs > 0 {
                format!(" (not counting {other_runs} files of another copy of Unweb running now)")
            } else {
                String::new()
            },
            if elsewhere.is_empty() {
                "none".to_string()
            } else {
                elsewhere.iter().take(10).map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
            }
        ),
    ));

    // 7. The GPU shader cache shared by every program kept nothing.
    let shaders_after = shader_cache_snapshot();
    let changed: Vec<String> = shaders_after
        .iter()
        .filter(|f| !shaders_before.contains(f))
        .map(|f| f.0.display().to_string())
        .collect();
    let webgl = set_report.get("webgl").cloned().unwrap_or_default();
    checks.push(check(
        "the shared GPU shader cache gains nothing",
        changed.is_empty(),
        if cfg!(target_os = "linux") {
            format!(
                "A compiled a new WebGL shader: {}; files added or changed in the user's Mesa and NVIDIA shader caches: {}",
                webgl == "1",
                if changed.is_empty() {
                    "none".to_string()
                } else {
                    changed.iter().take(10).cloned().collect::<Vec<_>>().join(", ")
                }
            )
        } else {
            format!(
                "A compiled a new WebGL shader: {}; on this system the driver's own shader cache is outside Unweb's folders and isn't checked here{}",
                webgl == "1",
                if cfg!(windows) {
                    " (WebView2's GPU shader disk cache is turned off)"
                } else {
                    ""
                }
            )
        },
    ));

    // 8. The normal browser's data, and the engine's default store, were
    // left alone.
    let normal_after = normal_browser_snapshot();
    let touched: Vec<String> = normal_after
        .iter()
        .filter(|f| !normal_before.contains(f))
        .map(|f| f.0.display().to_string())
        .collect();
    let looked: Vec<String> = normal_browser_roots()
        .iter()
        .filter(|r| r.exists())
        .map(|r| r.display().to_string())
        .collect();
    checks.push(check(
        "the normal browser's data is left alone",
        touched.is_empty(),
        format!(
            "files added or changed: {}; folders that exist and were watched: {}",
            if touched.is_empty() {
                "none".to_string()
            } else {
                touched
                    .iter()
                    .take(10)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            },
            if looked.is_empty() {
                "none (no normal browser data on this machine)".to_string()
            } else {
                looked.join(", ")
            }
        ),
    ));

    let passed = checks.iter().all(|c| c.pass);
    Outcome {
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        user_agent,
        folders: USES_FOLDERS,
        passed,
        checks,
        site_hits: site.hits(),
        hsts_seen: hsts.map(|h| h.seen()).unwrap_or_default(),
        other_hits: other.hits(),
        wipes,
        seconds: started.elapsed().as_secs_f64(),
    }
}
