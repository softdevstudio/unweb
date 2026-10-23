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
//!    Cache API entry, a service worker, a file in the HTTP cache, and a
//!    cookie and localStorage key from a frame on the other site.
//! 2. A opens /check: it must find all of it (the control, so the checks
//!    that follow can't pass by accident).
//! 3. Stranger B, open at the same time, opens /check: it must find none of
//!    it, send none of A's cookies, and fetch the cached file afresh.
//! 4. A closes: its data folder must be deleted and stay deleted.
//! 5. Stranger C, opened after A closed, opens /check: nothing.
//! 6. Every stranger closes: no folder may be left, and no file may appear
//!    anywhere else in Unweb's data, local data or cache folders.
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

const OVERALL: Duration = Duration::from_secs(150);

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
        (true, "/sw.js") => (
            "200 OK",
            "text/javascript",
            "self.addEventListener('install', function () { self.skipWaiting(); });".to_string(),
        ),
        (false, "/frame-set") => {
            extra.push_str("Set-Cookie: unweb_frame=1; Max-Age=3600; Path=/; SameSite=None\r\n");
            ("200 OK", "text/html", FRAME_SET.to_string())
        }
        (false, "/frame-check") => ("200 OK", "text/html", FRAME_CHECK.to_string()),
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
const FRAME_CHECK: &str = r#"<!doctype html><script>
var ls = ''; try { ls = localStorage.getItem('unweb_frame_ls') || ''; } catch (e) { ls = 'error'; }
parent.postMessage('frame:' + (document.cookie.indexOf('unweb_frame') >= 0 ? 1 : 0) + ':' + (ls ? 1 : 0), '*');
</script>"#;

fn set_page(who: &str, other: &str) -> String {
    format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><title>set {who}</title>
<script src="/cached.js"></script></head><body><h1>Leaving traces as {who}</h1>
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
  try {{ await navigator.serviceWorker.register('/sw.js'); await navigator.serviceWorker.ready; r.sw = 1; }} catch (e) {{ r.sw = 'error'; }}
  r.cachedjs = window.unwebCached ? 1 : 0;
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
<script src="/cached.js"></script></head><body><h1>Looking for traces as {who}</h1>
<script>
(async function () {{
  var r = {{ step: 'check', who: '{who}' }};
  function done() {{
    var q = Object.keys(r).map(function (k) {{ return k + '=' + encodeURIComponent(r[k]); }}).join('&');
    fetch('/report?' + q);
  }}
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
    detail: String,
}

fn check(name: &'static str, pass: bool, detail: String) -> Check {
    Check { name, pass, detail }
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
    std::thread::spawn(move || {
        let started = Instant::now();
        let outcome = run(&handle, &strangers, &site, &other, &site_url, started);
        let passed = outcome.passed;
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(
            dir.join("selftest.json"),
            serde_json::to_string_pretty(&outcome).unwrap_or_default(),
        );
        for c in &outcome.checks {
            println!(
                "{} {}: {}",
                if c.pass { "PASS" } else { "FAIL" },
                c.name,
                c.detail
            );
        }
        println!(
            "selftest {} in {:.1}s on {} ({})",
            if passed { "passed" } else { "FAILED" },
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

fn eval_string(app: &AppHandle, label: &str, js: &str) -> String {
    let Some(w) = app.get_webview_window(label) else {
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
    if let (Some(w), Ok(u)) = (app.get_webview_window(label), Url::parse(url)) {
        let _ = w.navigate(u);
    }
}

fn close(app: &AppHandle, strangers: &Strangers, label: &str) {
    strangers.retire(app, label);
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

/// A stranger that must have found nothing.
fn clean_stranger(name: &'static str, site: &Server, who: &str) -> Check {
    let report = site.report("check", who);
    let (mine, cached) = requests_of(site, who);
    let carried_cookie = mine.iter().any(|h| h.cookie.contains("unweb_"));
    let fresh_fetch = cached.len() == 1 && !cached[0].revalidate;
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
    let frame_found = report
        .as_ref()
        .map(|r| {
            r.get("frame_cookie").map(String::as_str) == Some("1")
                || r.get("frame_ls").map(String::as_str) == Some("1")
        })
        .unwrap_or(true);
    check(
        name,
        report.is_some() && found.is_empty() && !frame_found && !carried_cookie && fresh_fetch,
        format!(
            "page found [{}] (none expected): {}; its requests carried an earlier cookie: {carried_cookie}; the cached file was fetched afresh: {fresh_fetch}",
            found.join(" "),
            report.as_ref().map(describe).unwrap_or_else(|| "no report".into())
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

fn run(
    app: &AppHandle,
    strangers: &Strangers,
    site: &Server,
    other: &Server,
    site_url: &str,
    started: Instant,
) -> Outcome {
    let deadline = started + OVERALL;
    let mut checks = Vec::new();

    // 1. A leaves traces.
    let set = wait_until(deadline, || site.report("set", "A").is_some());
    std::thread::sleep(Duration::from_millis(1500));
    let user_agent = eval_string(app, "stranger-1", "navigator.userAgent");
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
    let folder_a = strangers.folder("stranger-1");
    let a_files = folder_a
        .as_ref()
        .map(|d| {
            let mut v = Vec::new();
            files_under(d, Path::new("/nonexistent-skip"), &mut v);
            v.len()
        })
        .unwrap_or(0);

    // 2. Control: A finds its own traces.
    navigate(app, "stranger-1", &format!("{site_url}/check?who=A"));
    let a_checked = wait_until(Instant::now() + Duration::from_secs(20), || {
        site.report("check", "A").is_some()
    });
    let a_report = site.report("check", "A").unwrap_or_default();
    let (a_mine, a_cached) = requests_of(site, "A");
    let a_cookie = a_mine
        .iter()
        .any(|h| h.path == "/check" && h.cookie.contains("unweb_http"));
    let a_cache_used = a_cached.is_empty() || a_cached.iter().all(|h| h.revalidate);
    let a_found: Vec<&str> = TRACES
        .iter()
        .copied()
        .filter(|k| a_report.get(*k).map(String::as_str) == Some("1"))
        .collect();
    checks.push(check(
        "stranger A finds its own traces (control)",
        a_checked && a_found.len() == TRACES.len() && a_cookie && a_cache_used,
        format!(
            "A found [{}]; A sent its server cookie: {a_cookie}; the cached file came from A's cache: {a_cache_used}; frame: cookie {} storage {}{}",
            a_found.join(" "),
            a_report.get("frame_cookie").cloned().unwrap_or_default(),
            a_report.get("frame_ls").cloned().unwrap_or_default(),
            folder_a
                .as_ref()
                .map(|_| format!("; {a_files} files in A's own folder"))
                .unwrap_or_default()
        ),
    ));

    // 3. B, open at the same time.
    let b = strangers.open(app, Url::parse(&format!("{site_url}/check?who=B")).ok());
    let b_ok = b.is_ok()
        && wait_until(Instant::now() + Duration::from_secs(25), || {
            site.report("check", "B").is_some()
        });
    let mut c_b = clean_stranger("a stranger open at the same time finds nothing", site, "B");
    if !b_ok {
        c_b.pass = false;
    }
    checks.push(c_b);

    // 4. A closes and its data goes.
    close(app, strangers, "stranger-1");
    let wiped = wait_until(Instant::now() + Duration::from_secs(40), || {
        strangers.wipes().iter().any(|w| w.label == "stranger-1")
    });
    let wipe_a = strangers
        .wipes()
        .into_iter()
        .find(|w| w.label == "stranger-1");
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

    // 5. C, opened after A closed.
    let c = strangers.open(app, Url::parse(&format!("{site_url}/check?who=C")).ok());
    let c_ok = c.is_ok()
        && wait_until(Instant::now() + Duration::from_secs(25), || {
            site.report("check", "C").is_some()
        });
    let mut c_c = clean_stranger("a new stranger after one closed finds nothing", site, "C");
    if !c_ok {
        c_c.pass = false;
    }
    checks.push(c_c);

    // 6. Everything closes; nothing may be left.
    for l in strangers.open_labels() {
        close(app, strangers, &l);
    }
    let all = wait_until(Instant::now() + Duration::from_secs(40), || {
        let w = strangers.wipes();
        ["stranger-2", "stranger-3"]
            .iter()
            .all(|l| w.iter().any(|x| x.label == *l))
    });
    // Engines can write late (a deferred cache record, a cookie flush), so
    // look again after a while.
    std::thread::sleep(Duration::from_secs(8));
    let wipes = strangers.wipes();
    let root = strangers.root().to_path_buf();
    let mut left_in_root = Vec::new();
    files_under(&root, Path::new("/nonexistent-skip"), &mut left_in_root);
    let mut elsewhere = Vec::new();
    let paths = app.path();
    for d in [
        paths.app_data_dir(),
        paths.app_local_data_dir(),
        paths.app_cache_dir(),
        paths.app_config_dir(),
    ]
    .into_iter()
    .flatten()
    {
        files_under(&d, &root, &mut elsewhere);
    }
    elsewhere.sort();
    elsewhere.dedup();
    checks.push(check(
        "nothing is left on disk",
        all && wipes.iter().all(|w| w.gone) && left_in_root.is_empty() && elsewhere.is_empty(),
        format!(
            "{} strangers wiped ({}); files left in the strangers folder: {:?}; files anywhere else in Unweb's folders: {}",
            wipes.len(),
            wipes
                .iter()
                .map(|w| format!("{} {}", w.label, if w.gone { "gone" } else { "NOT gone" }))
                .collect::<Vec<_>>()
                .join(", "),
            left_in_root.iter().take(10).map(|p| p.strip_prefix(&root).unwrap_or(p).display().to_string()).collect::<Vec<_>>(),
            if elsewhere.is_empty() {
                "none".to_string()
            } else {
                elsewhere.iter().take(10).map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
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
        other_hits: other.hits(),
        wipes,
        seconds: started.elapsed().as_secs_f64(),
    }
}
