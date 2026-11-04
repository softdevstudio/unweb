//! The compare strip: screenshots of every stranger in a window, side by
//! side, each with its language, location, cookie count and address.
//!
//! Compare (in the bar) takes a screenshot of each stranger's page and opens
//! a window ("cmp-<hex>") showing them as one strip. Its page lays the
//! strip out on a canvas, so Save image writes exactly what it shows, as a
//! PNG in the Downloads folder. Save report writes a folder there instead:
//! the strip, each stranger's screenshot, a page to open in any browser and
//! a JSON record, for handing on. The compare window has a small store of
//! its own, deleted when it closes like a stranger's.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use base64::Engine;
use tauri::webview::{NewWindowResponse, PageLoadEvent, WebviewBuilder};
use tauri::window::WindowBuilder;
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Url, WebviewUrl};

use crate::strangers::Strangers;

/// Compare windows' labels start with this; their page's with `VIEW_PREFIX`.
pub const PREFIX: &str = "cmp-";
pub const VIEW_PREFIX: &str = "cmpview-";
/// The largest image Save image accepts.
const MAX_IMAGE: usize = 64 * 1024 * 1024;

pub struct Compare {
    /// Where saved strips go.
    dir: PathBuf,
    /// Each compare window's strip, until its page has loaded.
    pending: Mutex<HashMap<String, serde_json::Value>>,
    /// Each open compare window's captions, for its report: kept here, so a
    /// report says what Unweb recorded, not what a page sent back.
    records: Mutex<HashMap<String, Record>>,
    /// The self-test keeps the latest screenshots to check them; otherwise
    /// nothing is kept once the compare window has them.
    keep_shots: bool,
    /// For the self-test: the latest screenshots and every saved file.
    pub last_shots: Mutex<Vec<Vec<u8>>>,
    pub saved: Mutex<Vec<PathBuf>>,
}

impl Compare {
    pub fn new(dir: PathBuf, keep_shots: bool) -> Compare {
        Compare {
            dir,
            keep_shots,
            pending: Mutex::new(HashMap::new()),
            records: Mutex::new(HashMap::new()),
            last_shots: Mutex::new(Vec::new()),
            saved: Mutex::new(Vec::new()),
        }
    }

    /// A compare window is gone: drop its strip if its page never took it.
    pub fn forget(&self, label: &str) {
        if let Ok(mut p) = self.pending.lock() {
            p.remove(label);
        }
        if let Ok(mut r) = self.records.lock() {
            r.remove(label);
        }
    }
}

/// What a report says about one compare: when, the address typed, and
/// each stranger's caption.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Record {
    taken: String,
    typed_address: Option<String>,
    strangers: Vec<RecordStranger>,
}

#[derive(Clone, Debug, serde::Serialize)]
struct RecordStranger {
    number: usize,
    language: String,
    language_tag: String,
    /// Scheme, host and port: never a user name or password.
    location: Option<String>,
    cookies: Option<Cookies>,
    cookie_line: String,
    address: String,
}

#[derive(Clone, Debug, serde::Serialize)]
struct Cookies {
    site: String,
    this_site: usize,
    this_site_http_only: usize,
    other_sites: usize,
    other_site_count: usize,
}

/// The window a compare page belongs to.
pub fn window_of_view(view: &str) -> Option<String> {
    view.strip_prefix(VIEW_PREFIX)
        .map(|n| format!("{PREFIX}{n}"))
}

/// The page that belongs to a compare window.
pub fn view_label(window: &str) -> String {
    format!(
        "{VIEW_PREFIX}{}",
        window.strip_prefix(PREFIX).unwrap_or(window)
    )
}

/// The only page a compare window may show.
pub fn may_open(url: &Url) -> bool {
    crate::window::is_app_origin(url) && url.path() == "/compare.html" && url.query().is_none()
}

/// Takes a screenshot of every stranger in window `win` and opens the
/// compare window with them. Blocks while the screenshots are taken, so
/// call it off the main thread.
pub fn open(app: &AppHandle, strangers: &Strangers, win: &str) -> Result<String, String> {
    let compare = app
        .try_state::<std::sync::Arc<Compare>>()
        .ok_or("not ready")?;
    let tiles = strangers.tiles(win);
    if tiles.is_empty() {
        return Err("this window has no strangers".into());
    }
    let mut panels = Vec::new();
    let mut shots = Vec::new();
    let mut record = Record {
        taken: utc_iso(),
        typed_address: strangers.typed(win).map(|u| shown_url(&u)),
        strangers: Vec::new(),
    };
    for (i, t) in tiles.iter().enumerate() {
        let page = app
            .get_webview(&crate::window::page_label(t))
            .ok_or("a stranger went away")?;
        let png = crate::capture::capture(&page)
            .map_err(|e| format!("Couldn't take stranger {}'s screenshot: {e}", i + 1))?;
        let settings = strangers.settings(t);
        let lang = crate::window::LANGUAGES
            .iter()
            .find(|(tag, _)| *tag == settings.lang)
            .map(|(_, name)| *name)
            .unwrap_or("System language");
        let location = match crate::location::show(settings.location.as_ref()) {
            l if l.is_empty() => "Direct".to_string(),
            l => format!("via {l}"),
        };
        let count = strangers.count(t);
        let cookies = count
            .as_ref()
            .map(unweb_core::describe)
            .unwrap_or_else(|| "No cookies yet".into());
        record.strangers.push(RecordStranger {
            number: i + 1,
            language: lang.to_string(),
            language_tag: settings.lang.clone(),
            location: settings
                .location
                .as_ref()
                .map(|l| crate::location::show(Some(l))),
            cookies: count.map(|c| Cookies {
                site: c.site,
                this_site: c.this_site,
                this_site_http_only: c.this_site_http_only,
                other_sites: c.other_sites,
                other_site_count: c.other_site_count,
            }),
            cookie_line: cookies.clone(),
            address: shown_address(&page),
        });
        panels.push(serde_json::json!({
            "image": format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&png)),
            "title": format!("Stranger {}", i + 1),
            "lang": lang,
            "location": location,
            "cookies": cookies,
            "address": shown_address(&page),
        }));
        shots.push(png);
    }
    if compare.keep_shots {
        if let Ok(mut l) = compare.last_shots.lock() {
            *l = shots;
        }
    }
    let label = format!("{PREFIX}{}", crate::strangers::random_hex());
    let data = serde_json::json!({ "panels": panels, "taken": utc_stamp(true) });
    if let Ok(mut p) = compare.pending.lock() {
        p.insert(label.clone(), data);
    }
    if let Ok(mut r) = compare.records.lock() {
        r.insert(label.clone(), record);
    }
    let folder = strangers.extra_folder(&label);
    if let Err(e) = build(app, &label, tiles.len(), folder.as_deref()) {
        if let Some(w) = app.get_window(&label) {
            let _ = w.destroy();
        }
        std::thread::spawn({
            let app = app.clone();
            let label = label.clone();
            move || {
                if let Some(s) = app.try_state::<std::sync::Arc<Strangers>>() {
                    s.closed(&label);
                }
            }
        });
        return Err(e.to_string());
    }
    Ok(label)
}

/// A page's address for a caption: scheme, host and path, without the
/// query and fragment a site can fill with identifiers.
fn shown_address(page: &tauri::Webview) -> String {
    match crate::window::page_url(page) {
        Some(u) => shown_url(&u),
        None => "Start page".into(),
    }
}

fn shown_url(u: &Url) -> String {
    if u.scheme() == "http" || u.scheme() == "https" {
        let port = u.port().map(|p| format!(":{p}")).unwrap_or_default();
        format!(
            "{}://{}{port}{}",
            u.scheme(),
            u.host_str().unwrap_or_default(),
            u.path()
        )
        .chars()
        .take(200)
        .collect()
    } else {
        "Start page".into()
    }
}

fn build(app: &AppHandle, label: &str, n: usize, folder: Option<&Path>) -> tauri::Result<()> {
    let size = ((n as f64 * 420.0 + 80.0).clamp(700.0, 1600.0), 680.0);
    let window = WindowBuilder::new(app, label)
        .title(if n == 1 {
            "Unweb: 1 stranger, compared".to_string()
        } else {
            format!("Unweb: {n} strangers compared")
        })
        .inner_size(size.0, size.1)
        .min_inner_size(420.0, 300.0);
    #[cfg(target_os = "macos")]
    let window = window.title_bar_style(tauri::TitleBarStyle::Transparent);
    let window = window.build()?;
    let load_app = app.clone();
    let load_label = label.to_string();
    let view = crate::window::store(
        WebviewBuilder::new(view_label(label), WebviewUrl::App("compare.html".into())),
        folder,
        &crate::strangers::Settings::default(),
    )
    .auto_resize()
    .disable_drag_drop_handler()
    .on_navigation(may_open)
    .on_new_window(|_, _| NewWindowResponse::Deny)
    .on_download(|_, _| false)
    .on_page_load(move |view, payload| {
        // Only the compare page itself (an engine can report an empty page
        // first).
        if payload.event() != PageLoadEvent::Finished || !may_open(payload.url()) {
            return;
        }
        let Some(compare) = load_app.try_state::<std::sync::Arc<Compare>>() else {
            return;
        };
        let data = compare
            .pending
            .lock()
            .ok()
            .and_then(|mut p| p.remove(&load_label));
        if let Some(data) = data {
            let _ = view.eval(format!(
                "window.unwebCompare && window.unwebCompare({data})"
            ));
        }
    });
    window.add_child(
        view,
        LogicalPosition::new(0.0, 0.0),
        LogicalSize::new(size.0, size.1),
    )?;
    Ok(())
}

/// Save image: writes the strip the compare page drew (a PNG, as base64 or
/// a data: address) to the Downloads folder. Returns the file's path.
pub fn save(compare: &Compare, image: &str) -> Result<PathBuf, String> {
    let b64 = image
        .strip_prefix("data:image/png;base64,")
        .unwrap_or(image);
    if b64.len() > MAX_IMAGE / 3 * 4 + 4 {
        return Err("the image is too large to save".into());
    }
    let png = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|_| "the image couldn't be read".to_string())?;
    if !png.starts_with(crate::capture::PNG_MAGIC) {
        return Err("the image isn't a PNG".into());
    }
    std::fs::create_dir_all(&compare.dir).map_err(|e| format!("Couldn't save: {e}"))?;
    let stem = format!("unweb-compare-{}", utc_stamp(false));
    let path = write_new(&compare.dir, &stem, &png)?;
    if let Ok(mut s) = compare.saved.lock() {
        s.push(path.clone());
    }
    Ok(path)
}

fn png_of(b64: &str) -> Result<Vec<u8>, String> {
    let b64 = b64.strip_prefix("data:image/png;base64,").unwrap_or(b64);
    if b64.len() > MAX_IMAGE / 3 * 4 + 4 {
        return Err("an image is too large to save".into());
    }
    let png = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|_| "an image couldn't be read".to_string())?;
    if !png.starts_with(crate::capture::PNG_MAGIC) || png.len() < 24 || &png[12..16] != b"IHDR" {
        return Err("an image isn't a PNG".into());
    }
    Ok(png)
}

/// A PNG's width and height, from its header.
fn png_size(png: &[u8]) -> (u32, u32) {
    let be = |i: usize| u32::from_be_bytes([png[i], png[i + 1], png[i + 2], png[i + 3]]);
    (be(16), be(20))
}

/// Save report: a new folder in Downloads with the strip (`strip`, what
/// Save image saves), each stranger's screenshot (`shots`, in order), a
/// report page and a JSON record. The captions come from the compare
/// window's record. Returns the folder's path.
pub fn save_report(
    compare: &Compare,
    window: &str,
    strip: &str,
    shots: &[String],
) -> Result<PathBuf, String> {
    let record = compare
        .records
        .lock()
        .ok()
        .and_then(|r| r.get(window).cloned())
        .ok_or("this compare window has nothing to report")?;
    if shots.len() != record.strangers.len() {
        return Err("the screenshots don't match the strangers".into());
    }
    let strip = png_of(strip)?;
    let shots = shots
        .iter()
        .map(|s| png_of(s))
        .collect::<Result<Vec<_>, _>>()?;
    std::fs::create_dir_all(&compare.dir).map_err(|e| format!("Couldn't save: {e}"))?;
    let dir = new_folder(&compare.dir, &format!("unweb-compare-{}", utc_stamp(false)))?;
    let written = write_report(&dir, &record, &strip, &shots);
    if let Err(e) = written {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(format!("Couldn't save: {e}"));
    }
    if let Ok(mut s) = compare.saved.lock() {
        s.push(dir.clone());
    }
    Ok(dir)
}

fn write_report(
    dir: &Path,
    record: &Record,
    strip: &[u8],
    shots: &[Vec<u8>],
) -> std::io::Result<()> {
    let mut strangers = Vec::new();
    for (s, png) in record.strangers.iter().zip(shots) {
        let file = format!("stranger-{}.png", s.number);
        write_file(&dir.join(&file), png)?;
        let (w, h) = png_size(png);
        let mut v = serde_json::to_value(s).map_err(std::io::Error::other)?;
        v["screenshot"] = file.into();
        v["width"] = w.into();
        v["height"] = h.into();
        strangers.push(v);
    }
    write_file(&dir.join("strip.png"), strip)?;
    let json = serde_json::json!({
        "format": "unweb-compare",
        "format_version": 1,
        "unweb": env!("CARGO_PKG_VERSION"),
        "taken": record.taken,
        "typed_address": record.typed_address,
        "strip": "strip.png",
        "strangers": strangers,
    });
    let text = serde_json::to_vec_pretty(&json).map_err(std::io::Error::other)?;
    write_file(&dir.join("report.json"), &text)?;
    write_file(&dir.join("report.html"), report_html(record).as_bytes())?;
    Ok(())
}

/// Text made safe for HTML, in an element or an attribute.
fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            c if c.is_control() => o.push(' '),
            c => o.push(c),
        }
    }
    o
}

/// The report page: static, no scripts, everything it shows escaped.
fn report_html(r: &Record) -> String {
    let mut figs = String::new();
    for s in &r.strangers {
        figs.push_str(&format!(
            "<figure>\n<figcaption><b>Stranger {n}</b><br>{lang} &middot; {loc}<br>{cookies}<br><span class=\"addr\">{addr}</span></figcaption>\n<a href=\"stranger-{n}.png\"><img src=\"stranger-{n}.png\" alt=\"Stranger {n}'s screenshot\"></a>\n</figure>\n",
            n = s.number,
            lang = esc(&s.language),
            loc = esc(&s.location.as_ref().map(|l| format!("via {l}")).unwrap_or_else(|| "Direct".into())),
            cookies = esc(&s.cookie_line),
            addr = esc(&s.address),
        ));
    }
    let typed = r
        .typed_address
        .as_ref()
        .map(|t| format!(" &middot; address typed: {}", esc(t)))
        .unwrap_or_default();
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src 'self' file: data:; style-src 'unsafe-inline'\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>Unweb compare, {taken}</title>\n<style>\nbody {{ font: 15px/1.45 system-ui, sans-serif; margin: 24px; color: #111827; background: #fff; }}\nh1 {{ font-size: 20px; margin: 0 0 4px; }}\np.meta {{ color: #4b5563; margin: 0 0 20px; }}\nmain {{ display: flex; flex-wrap: wrap; gap: 20px; align-items: flex-start; }}\nfigure {{ margin: 0; flex: 1 1 320px; max-width: 640px; }}\nfigcaption {{ font-size: 14px; color: #374151; margin-bottom: 8px; overflow-wrap: anywhere; }}\n.addr {{ color: #6b7280; }}\nimg {{ width: 100%; border: 1px solid #d1d5db; }}\nfooter {{ margin-top: 24px; color: #6b7280; font-size: 13px; }}\n</style>\n</head>\n<body>\n<h1>Unweb compare</h1>\n<p class=\"meta\">{n} {strangers}, {taken}{typed}</p>\n<main>\n{figs}</main>\n<footer>Made by Unweb {v}. The strip as one image: <a href=\"strip.png\">strip.png</a>. The same record as data: <a href=\"report.json\">report.json</a>.</footer>\n</body>\n</html>\n",
        taken = esc(&r.taken),
        n = r.strangers.len(),
        strangers = if r.strangers.len() == 1 { "stranger" } else { "strangers" },
        v = env!("CARGO_PKG_VERSION"),
    )
}

/// Creates `<dir>/<stem>`, or `<stem>-2` and so on when that name is
/// taken; never into an existing folder.
fn new_folder(dir: &Path, stem: &str) -> Result<PathBuf, String> {
    for n in 1..1000 {
        let name = if n == 1 {
            stem.to_string()
        } else {
            format!("{stem}-{n}")
        };
        let path = dir.join(name);
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("Couldn't save: {e}")),
        }
    }
    Err("Couldn't find a free folder name".into())
}

fn write_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}

/// Writes `bytes` to `<dir>/<stem>.png`, or `<stem>-2.png` and so on when
/// that name is taken; never over an existing file.
fn write_new(dir: &Path, stem: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    use std::io::Write;
    for n in 1..1000 {
        let name = if n == 1 {
            format!("{stem}.png")
        } else {
            format!("{stem}-{n}.png")
        };
        let path = dir.join(name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut f) => {
                let written = f.write_all(bytes).and_then(|()| f.sync_all());
                return match written {
                    Ok(()) => Ok(path),
                    Err(e) => {
                        drop(f);
                        let _ = std::fs::remove_file(&path);
                        Err(format!("Couldn't save: {e}"))
                    }
                };
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("Couldn't save: {e}")),
        }
    }
    Err("Couldn't find a free file name".into())
}

/// The time now in UTC as ISO 8601: "2026-10-24T08:35:12Z".
fn utc_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    iso_of(secs)
}

fn iso_of(secs: u64) -> String {
    let (y, m, d) = civil_from_days((secs / 86_400) as i64);
    let s = secs % 86_400;
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        s / 3600,
        s % 3600 / 60,
        s % 60
    )
}

/// The time now in UTC: "20261024-083512", or "2026-10-24 08:35 UTC" for
/// people to read.
pub fn utc_stamp(readable: bool) -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    stamp_of(secs, readable)
}

fn stamp_of(secs: u64, readable: bool) -> String {
    let (y, m, d) = civil_from_days((secs / 86_400) as i64);
    let s = secs % 86_400;
    let (hh, mm, ss) = (s / 3600, s % 3600 / 60, s % 60);
    if readable {
        format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02} UTC")
    } else {
        format!("{y:04}{m:02}{d:02}-{hh:02}{mm:02}{ss:02}")
    }
}

/// Days since 1970-01-01 to a calendar date (Howard Hinnant's method).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamps() {
        assert_eq!(stamp_of(0, false), "19700101-000000");
        // 2026-10-24 08:35:12 UTC
        assert_eq!(stamp_of(1_792_830_912, false), "20261024-083512");
        assert_eq!(stamp_of(1_792_830_912, true), "2026-10-24 08:35 UTC");
        // A leap day.
        assert_eq!(stamp_of(1_709_164_800, false), "20240229-000000");
    }

    fn tiny_png(w: u32, h: u32) -> String {
        let mut p = crate::capture::PNG_MAGIC.to_vec();
        p.extend_from_slice(&13u32.to_be_bytes());
        p.extend_from_slice(b"IHDR");
        p.extend_from_slice(&w.to_be_bytes());
        p.extend_from_slice(&h.to_be_bytes());
        p.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        base64::engine::general_purpose::STANDARD.encode(p)
    }

    #[test]
    fn a_report_is_a_new_folder_with_everything_escaped() {
        let mut b = [0u8; 8];
        getrandom::fill(&mut b).unwrap();
        let dir = std::env::temp_dir().join(format!(
            "unweb-report-{}-{}",
            std::process::id(),
            b.iter().map(|x| format!("{x:02x}")).collect::<String>()
        ));
        let c = Compare::new(dir.clone(), false);
        let loc = crate::location::parse_location("socks5://ann:s3cret@p.example:1080")
            .unwrap()
            .unwrap();
        let rec = Record {
            taken: iso_of(1_792_830_912),
            typed_address: Some("https://shop.example/x".into()),
            strangers: vec![
                RecordStranger {
                    number: 1,
                    language: "Deutsch".into(),
                    language_tag: "de-DE".into(),
                    location: Some(crate::location::show(Some(&loc))),
                    cookies: Some(Cookies {
                        site: "shop.example".into(),
                        this_site: 3,
                        this_site_http_only: 1,
                        other_sites: 2,
                        other_site_count: 1,
                    }),
                    cookie_line: "3 cookies from shop.example".into(),
                    address: "https://shop.example/<script>alert(1)</script>".into(),
                },
                RecordStranger {
                    number: 2,
                    language: "System language".into(),
                    language_tag: String::new(),
                    location: None,
                    cookies: None,
                    cookie_line: "No cookies yet".into(),
                    address: "Start page".into(),
                },
            ],
        };
        c.records.lock().unwrap().insert("cmp-1".into(), rec);
        let shots = vec![tiny_png(800, 600), tiny_png(640, 480)];
        // The wrong number of screenshots, or one that isn't a PNG.
        assert!(save_report(&c, "cmp-1", &tiny_png(10, 10), &shots[..1]).is_err());
        let gif = base64::engine::general_purpose::STANDARD.encode(b"GIF89a....");
        assert!(save_report(&c, "cmp-1", &tiny_png(10, 10), &[shots[0].clone(), gif]).is_err());
        assert!(save_report(&c, "cmp-9", &tiny_png(10, 10), &shots).is_err());
        let a = save_report(&c, "cmp-1", &tiny_png(10, 10), &shots).unwrap();
        let b2 = save_report(&c, "cmp-1", &tiny_png(10, 10), &shots).unwrap();
        assert_ne!(a, b2);
        let mut names: Vec<String> = std::fs::read_dir(&a)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "report.html",
                "report.json",
                "stranger-1.png",
                "stranger-2.png",
                "strip.png"
            ]
        );
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(a.join("report.json")).unwrap()).unwrap();
        assert_eq!(json["format"], "unweb-compare");
        assert_eq!(json["taken"], "2026-10-24T08:35:12Z");
        assert_eq!(json["strangers"][0]["width"], 800);
        assert_eq!(json["strangers"][1]["height"], 480);
        assert_eq!(json["strangers"][0]["location"], "socks5://p.example:1080");
        assert!(json["strangers"][1]["location"].is_null());
        assert_eq!(json["strangers"][0]["cookies"]["this_site"], 3);
        let html = std::fs::read_to_string(a.join("report.html")).unwrap();
        assert!(!html.contains("<script"), "{html}");
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("src=\"stranger-2.png\""));
        for f in &names {
            let bytes = std::fs::read(a.join(f)).unwrap();
            let text = String::from_utf8_lossy(&bytes);
            assert!(!text.contains("s3cret") && !text.contains("ann@"), "{f}");
        }
        // Nothing is left behind when the record is gone.
        c.forget("cmp-1");
        assert!(save_report(&c, "cmp-1", &tiny_png(10, 10), &shots).is_err());
        assert_eq!(window_of_view("cmpview-0a1b").as_deref(), Some("cmp-0a1b"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_the_compare_page() {
        let app = crate::window::app_url("compare.html");
        assert!(may_open(&app));
        assert!(!may_open(&crate::window::app_url("bar.html")));
        assert!(!may_open(
            &Url::parse("http://127.0.0.1:1/compare.html").unwrap()
        ));
        assert!(!may_open(&Url::parse(&format!("{app}?x=1")).unwrap()));
        assert_eq!(view_label("cmp-0a1b"), "cmpview-0a1b");
    }

    #[test]
    fn saving_checks_and_never_overwrites() {
        let mut b = [0u8; 8];
        getrandom::fill(&mut b).unwrap();
        let dir = std::env::temp_dir().join(format!(
            "unweb-compare-{}-{}",
            std::process::id(),
            b.iter().map(|x| format!("{x:02x}")).collect::<String>()
        ));
        let c = Compare::new(dir.clone(), false);
        let png = base64::engine::general_purpose::STANDARD.encode(b"\x89PNG\r\n\x1a\nrest");
        let a = save(&c, &format!("data:image/png;base64,{png}")).unwrap();
        let b2 = save(&c, &png).unwrap();
        assert_ne!(a, b2);
        assert!(b2.to_string_lossy().ends_with("-2.png") || a.file_name() != b2.file_name());
        assert!(save(&c, "not base64 !!").is_err());
        let gif = base64::engine::general_purpose::STANDARD.encode(b"GIF89a....");
        assert!(save(&c, &gif).is_err());
        assert_eq!(c.saved.lock().unwrap().len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
