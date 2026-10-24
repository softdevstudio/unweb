//! The compare strip: screenshots of every stranger in a window, side by
//! side, each with its language, location, cookie count and address.
//!
//! Compare (in the bar) takes a screenshot of each stranger's page and opens
//! a window ("cmp-<hex>") showing them as one strip. Its page lays the
//! strip out on a canvas, so Save image writes exactly what it shows, as a
//! PNG in the Downloads folder. The compare window has a small store of its
//! own, deleted when it closes like a stranger's.

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
    /// For the self-test: the latest screenshots and every saved file.
    pub last_shots: Mutex<Vec<Vec<u8>>>,
    pub saved: Mutex<Vec<PathBuf>>,
}

impl Compare {
    pub fn new(dir: PathBuf) -> Compare {
        Compare {
            dir,
            pending: Mutex::new(HashMap::new()),
            last_shots: Mutex::new(Vec::new()),
            saved: Mutex::new(Vec::new()),
        }
    }
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
        let cookies = strangers
            .count(t)
            .map(|c| unweb_core::describe(&c))
            .unwrap_or_else(|| "No cookies yet".into());
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
    if let Ok(mut l) = compare.last_shots.lock() {
        *l = shots;
    }
    let label = format!("{PREFIX}{}", crate::strangers::random_hex());
    let data = serde_json::json!({ "panels": panels, "taken": utc_stamp(true) });
    if let Ok(mut p) = compare.pending.lock() {
        p.insert(label.clone(), data);
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
    match page.url() {
        Ok(u) if u.scheme() == "http" || u.scheme() == "https" => format!(
            "{}://{}{}",
            u.scheme(),
            u.host_str().unwrap_or_default(),
            u.path()
        )
        .chars()
        .take(200)
        .collect(),
        _ => "Start page".into(),
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
        let c = Compare::new(dir.clone());
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
