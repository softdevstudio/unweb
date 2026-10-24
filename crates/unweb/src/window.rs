//! Unweb's windows: one shared bar on top and one to four strangers below.
//!
//! A window ("win-<hex>") holds the bar ("bar-<hex>", Unweb's own page with
//! the address field and buttons) and its strangers. Each stranger
//! ("stranger-<hex>") is a tile of two webviews: a thin strip
//! ("strip-<hex>") with its language, cookie count and close button, and
//! the page ("page-<hex>") that shows the web. A tile's strip and page use
//! that stranger's own store; the bar uses a small store of the window's
//! own. Only the bar and the strips are granted app commands
//! (capabilities/), each steering only its own window or tile; pages get
//! none.

use std::path::Path;

use tauri::webview::{DownloadEvent, NewWindowResponse, PageLoadEvent, WebviewBuilder};
use tauri::window::WindowBuilder;
#[cfg(not(target_os = "linux"))]
use tauri::WindowEvent;
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Url, WebviewUrl};

use crate::strangers::{Settings, PREFIX};

/// Height of the bar in logical pixels.
pub const BAR_HEIGHT: f64 = 44.0;
/// Height of a stranger's strip.
pub const STRIP_HEIGHT: f64 = 28.0;
/// Window labels start with this.
pub const WINDOW_PREFIX: &str = "win-";
/// The most strangers in one window.
pub const MAX_TILES: usize = 4;

/// Languages a stranger can be given, as (tag, name). The first is "the
/// system's", which leaves the engine's default alone.
pub const LANGUAGES: [(&str, &str); 15] = [
    ("", "System language"),
    ("en-US", "English (US)"),
    ("en-GB", "English (UK)"),
    ("de-DE", "Deutsch"),
    ("fr-FR", "Français"),
    ("es-ES", "Español"),
    ("it-IT", "Italiano"),
    ("pt-BR", "Português (Brasil)"),
    ("nl-NL", "Nederlands"),
    ("pl-PL", "Polski"),
    ("tr-TR", "Türkçe"),
    ("ja-JP", "日本語"),
    ("zh-CN", "中文 (简体)"),
    ("he-IL", "עברית"),
    ("ar", "العربية"),
];

/// Whether this engine lets each stranger have a language of its own.
/// WebView2 takes it per environment, and every stranger has its own.
/// WKWebView takes it from the whole app, and WebKitGTK's preferred
/// languages apply to every web context in the process (the self-test
/// showed one stranger's choice reaching the others), so on macOS and Linux
/// every stranger has the system's.
pub const LANGUAGE_PER_STRANGER: bool = cfg!(target_os = "windows");

/// Why there is no language per stranger here, for the strip's menu.
pub const NO_LANGUAGE_REASON: &str = if cfg!(target_os = "macos") {
    "On macOS every stranger uses the system language: WebKit takes one language for the whole app."
} else {
    "On Linux every stranger uses the system language: WebKitGTK takes one language for all of them."
};

/// A language tag Unweb offers, or None.
pub fn known_language(tag: &str) -> Option<&'static str> {
    LANGUAGES.iter().find(|(t, _)| *t == tag).map(|(t, _)| *t)
}

fn suffix(label: &str) -> &str {
    label
        .strip_prefix(PREFIX)
        .or_else(|| label.strip_prefix(WINDOW_PREFIX))
        .unwrap_or(label)
}

/// The page webview's label for a stranger.
pub fn page_label(stranger: &str) -> String {
    format!("page-{}", suffix(stranger))
}

/// The strip webview's label for a stranger.
pub fn strip_label(stranger: &str) -> String {
    format!("strip-{}", suffix(stranger))
}

/// The bar webview's label for a window.
pub fn bar_label(window: &str) -> String {
    format!("bar-{}", suffix(window))
}

fn hex_suffix<'a>(label: &'a str, prefix: &str) -> Option<&'a str> {
    let n = label.strip_prefix(prefix)?;
    (!n.is_empty() && n.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))).then_some(n)
}

/// The window a bar belongs to.
pub fn window_of_bar(webview: &str) -> Option<String> {
    hex_suffix(webview, "bar-").map(|n| format!("{WINDOW_PREFIX}{n}"))
}

/// The stranger a strip or page belongs to.
pub fn stranger_of(webview: &str) -> Option<String> {
    hex_suffix(webview, "strip-")
        .or_else(|| hex_suffix(webview, "page-"))
        .map(|n| format!("{PREFIX}{n}"))
}

/// True for Unweb's own pages (the start page, the bar and the strips),
/// which are served from the app's origin: exactly `tauri://localhost` on
/// Linux and macOS, `http://tauri.localhost` (no port) on Windows.
/// Anything else, such as `http://tauri.localhost:8765` on Linux (which the
/// engine sends to whatever listens on that port), is an ordinary web
/// address.
pub fn is_app_origin(url: &Url) -> bool {
    let (scheme, host) = if cfg!(windows) {
        ("http", "tauri.localhost")
    } else {
        ("tauri", "localhost")
    };
    url.scheme() == scheme && url.host_str() == Some(host) && url.port().is_none()
}

/// The address of one of Unweb's own pages.
pub fn app_url(path: &str) -> Url {
    let base = if cfg!(windows) {
        "http://tauri.localhost/"
    } else {
        "tauri://localhost/"
    };
    Url::parse(base)
        .and_then(|b| b.join(path))
        .expect("app URL")
}

/// What the page may open. Web pages, plus the start page Unweb itself
/// shows; never the bar, a strip or anything else of the app's.
pub fn page_may_open(url: &Url) -> bool {
    if is_app_origin(url) {
        return matches!(url.path(), "" | "/" | "/index.html")
            && url.query().is_none()
            && url.fragment().is_none();
    }
    match url.scheme() {
        "http" | "https" => !matches!(url.host_str(), Some("ipc.localhost")),
        "about" | "blob" | "data" => true,
        _ => false,
    }
}

/// What the bar may open: only itself. A link dropped on the bar, or
/// anything else that tries to load a page there, is refused, so a site
/// can never take the bar's place.
pub fn bar_may_open(url: &Url) -> bool {
    is_app_origin(url) && url.path() == "/bar.html" && url.query().is_none()
}

/// The same for a stranger's strip.
pub fn strip_may_open(url: &Url) -> bool {
    is_app_origin(url) && url.path() == "/strip.html" && url.query().is_none()
}

/// Columns and rows for `n` strangers: side by side up to three, two by
/// two for four.
pub fn grid(n: usize) -> (usize, usize) {
    match n {
        0 | 1 => (1, 1),
        2 => (2, 1),
        3 => (3, 1),
        _ => (2, 2),
    }
}

/// Where tile `i` of `n` goes in a content area of `w` x `h` (below the
/// bar): its strip and its page, as (x, y, width, height).
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub fn tile_rects(n: usize, i: usize, w: f64, h: f64) -> ([f64; 4], [f64; 4]) {
    let (cols, rows) = grid(n);
    let (col, row) = (i % cols, i / cols);
    let tw = (w / cols as f64).floor();
    let th = (h / rows as f64).floor();
    let x = col as f64 * tw;
    let y = BAR_HEIGHT + row as f64 * th;
    // The last column and row take what rounding left over.
    let tw = if col + 1 == cols { w - x } else { tw };
    let th = if row + 1 == rows {
        h - row as f64 * th
    } else {
        th
    };
    (
        [x, y, tw, STRIP_HEIGHT.min(th)],
        [x, y + STRIP_HEIGHT, tw, (th - STRIP_HEIGHT).max(0.0)],
    )
}

/// Builds an empty window with its bar. `folder` is the window's own small
/// store on Windows and Linux. Strangers are added with `add_tile`.
pub fn build_window(
    app: &AppHandle,
    label: &str,
    title: &str,
    size: (f64, f64),
    folder: Option<&Path>,
) -> tauri::Result<tauri::Window> {
    let window = WindowBuilder::new(app, label)
        .title(title)
        .inner_size(size.0, size.1)
        .min_inner_size(480.0, 300.0);
    // Tauri's default macOS style lets the content run under the title
    // bar, which would hide most of the bar. This one keeps the content
    // below it.
    #[cfg(target_os = "macos")]
    let window = window.title_bar_style(tauri::TitleBarStyle::Transparent);
    let window = window.build()?;

    let bar_app = app.clone();
    let bar_window = label.to_string();
    let bar = store(
        WebviewBuilder::new(bar_label(label), WebviewUrl::App("bar.html".into())),
        folder,
        &Settings::default(),
    )
    .disable_drag_drop_handler()
    .on_navigation(bar_may_open)
    .on_new_window(|_, _| NewWindowResponse::Deny)
    .on_download(|_, _| false)
    // A page can finish loading before the bar is ready to show it.
    .on_page_load(move |_, payload| {
        if payload.event() == PageLoadEvent::Finished {
            show_address(&bar_app, &bar_window);
        }
    });
    window.add_child(
        bar,
        LogicalPosition::new(0.0, 0.0),
        LogicalSize::new(size.0, BAR_HEIGHT),
    )?;

    #[cfg(not(target_os = "linux"))]
    {
        let app = app.clone();
        let l = label.to_string();
        window.on_window_event(move |e| {
            if let WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } = e {
                layout(&app, &l);
            }
        });
    }
    Ok(window)
}

/// Adds a stranger's strip and page to `window`, with its language and
/// location (already checked). The page starts empty; `start_page` sends it
/// to its first address.
pub fn add_tile(
    app: &AppHandle,
    window: &tauri::Window,
    stranger: &str,
    folder: Option<&Path>,
    settings: &Settings,
) -> tauri::Result<()> {
    let win = window.label().to_string();
    let strip_app = app.clone();
    let strip_stranger = stranger.to_string();
    let strip = store(
        WebviewBuilder::new(strip_label(stranger), WebviewUrl::App("strip.html".into())),
        folder,
        settings,
    )
    .disable_drag_drop_handler()
    .on_navigation(strip_may_open)
    .on_new_window(|_, _| NewWindowResponse::Deny)
    .on_download(|_, _| false)
    .on_page_load(move |_, payload| {
        if payload.event() == PageLoadEvent::Finished {
            crate::counter::refresh_one(&strip_app, &strip_stranger);
        }
    });

    let load_app = app.clone();
    let load_win = win.clone();
    let load_stranger = stranger.to_string();
    let popup_app = app.clone();
    let popup_stranger = stranger.to_string();
    let title_app = app.clone();
    let title_win = win.clone();
    let blank = WebviewUrl::External(Url::parse("about:blank").expect("about:blank"));
    let page = store(
        WebviewBuilder::new(page_label(stranger), blank),
        folder,
        settings,
    );
    // WebRTC can reach the network around a proxy: with a location it is
    // taken away in every frame (and turned off in the engine below on
    // Linux, kept to the proxy by WebView2's switches on Windows).
    let page = if settings.location.is_some() {
        page.initialization_script_for_all_frames(crate::location::NO_WEBRTC)
    } else {
        page
    };
    let page = page
        .disable_drag_drop_handler()
        .on_navigation(page_may_open)
        // A new window would need a store of its own; open it here instead,
        // so it stays with this stranger.
        .on_new_window(move |url, _| {
            if page_may_open(&url) && !is_app_origin(&url) {
                if let Some(p) = popup_app.get_webview(&page_label(&popup_stranger)) {
                    let _ = p.navigate(url);
                }
            }
            NewWindowResponse::Deny
        })
        // Downloads would land outside the stranger's folder.
        .on_download(|_, event| !matches!(event, DownloadEvent::Requested { .. }))
        .on_page_load(move |_, payload| {
            if payload.event() == PageLoadEvent::Finished
                || payload.event() == PageLoadEvent::Started
            {
                show_address(&load_app, &load_win);
                crate::counter::refresh_one(&load_app, &load_stranger);
            }
        })
        .on_document_title_changed(move |_, _| show_address(&title_app, &title_win));

    // Placed properly by `layout` once the window knows all its tiles.
    let size = LogicalSize::new(100.0, 100.0);
    window.add_child(strip, LogicalPosition::new(0.0, BAR_HEIGHT), size)?;
    let page_view = window.add_child(
        page,
        LogicalPosition::new(0.0, BAR_HEIGHT + STRIP_HEIGHT),
        size,
    )?;

    // WebKitGTK can offer to save HTTP sign-in passwords in the system
    // keyring, which would outlive the stranger.
    #[cfg(target_os = "linux")]
    {
        let close_app = app.clone();
        let close_stranger = stranger.to_string();
        let no_webrtc = settings.location.is_some();
        let _ = page_view.with_webview(move |pv| {
            use webkit2gtk::{WebViewExt, WebsiteDataManagerExt};
            if let Some(m) = pv.inner().website_data_manager() {
                m.set_persistent_credential_storage_enabled(false);
            }
            if no_webrtc {
                use gtk::glib::prelude::ObjectExt;
                if let Some(s) = WebViewExt::settings(&pv.inner()) {
                    if s.find_property("enable-webrtc").is_some() {
                        s.set_property("enable-webrtc", false);
                    }
                }
            }
            // A page that calls window.close() takes its webview away (wry
            // destroys it, and with it any other close handler). Close the
            // stranger with it, as its strip's close button would, so its
            // data goes and the window lays itself out again instead of
            // keeping an empty tile. Unweb's own closes mark the stranger
            // busy first, so this only acts when the page went by itself.
            use gtk::prelude::WidgetExt;
            pv.inner().connect_destroy(move |_| {
                let app = close_app.clone();
                let stranger = close_stranger.clone();
                std::thread::spawn(move || {
                    if let Some(s) = app.try_state::<std::sync::Arc<crate::strangers::Strangers>>()
                    {
                        s.retire(&app, &stranger);
                    }
                });
            });
        });
    }
    // WebView2: a page's window.close() makes wry destroy the page's own
    // window. Close the stranger with it, as on Linux.
    #[cfg(target_os = "windows")]
    {
        let close_app = app.clone();
        let close_stranger = stranger.to_string();
        let _ = page_view.with_webview(move |pv| unsafe {
            use webview2_com::WindowCloseRequestedEventHandler;
            if let Ok(core) = pv.controller().CoreWebView2() {
                let mut token = 0i64;
                let _ = core.add_WindowCloseRequested(
                    &WindowCloseRequestedEventHandler::create(Box::new(move |_, _| {
                        let app = close_app.clone();
                        let stranger = close_stranger.clone();
                        std::thread::spawn(move || {
                            if let Some(s) =
                                app.try_state::<std::sync::Arc<crate::strangers::Strangers>>()
                            {
                                s.retire(&app, &stranger);
                            }
                        });
                        Ok(())
                    })),
                    &mut token,
                );
            }
        });
    }
    // macOS: the proxy again, with fallback to going direct turned off,
    // before the page's first request.
    #[cfg(target_os = "macos")]
    if let Some(l) = &settings.location {
        crate::location::macos_no_failover(&page_view, l)
            .map_err(|e| tauri::Error::Io(std::io::Error::other(e)))?;
    }
    #[cfg(target_os = "macos")]
    let _ = page_view;
    Ok(())
}

/// Sends a new stranger's page to `target`. Its language, where the
/// engine takes one per stranger (Windows), is already a switch of its
/// WebView2 environment (see `store`), so it applies from the first request.
pub fn start_page(page: &tauri::Webview, target: Url) {
    let _ = page.navigate(target);
}

/// Gives a webview its store (the folder on Windows and Linux, a fresh
/// in-memory store on macOS), its language where the engine takes one per
/// stranger, and its location. A stranger's strip and page get the same,
/// since they share one store.
pub(crate) fn store<R: tauri::Runtime>(
    b: WebviewBuilder<R>,
    folder: Option<&Path>,
    settings: &Settings,
) -> WebviewBuilder<R> {
    // WebView2 keeps compiled GPU shaders in a disk cache a page can fill
    // and later probe by timing. Turn it off (and keep wry's own default
    // switches, which this replaces). The language goes here too, since
    // WebView2 takes it per environment. Every webview in one folder must
    // ask for the same switches, since they share one environment.
    #[cfg(target_os = "windows")]
    let b = {
        let mut args = String::from(
            "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --disable-gpu-shader-disk-cache",
        );
        if let Some(tag) = known_language(&settings.lang).filter(|t| !t.is_empty()) {
            args.push_str(&format!(" --lang={tag} --accept-lang={tag}"));
        }
        // wry adds its own proxy switch only when the app gives none, so
        // the location goes here too.
        args.push_str(&crate::location::webview2_switches(
            settings.location.as_ref(),
        ));
        b.additional_browser_args(&args)
    };
    // WebKitGTK takes the proxy per web context (each stranger has its
    // own), WKWebView per data store (each stranger has its own).
    #[cfg(not(target_os = "windows"))]
    let b = match &settings.location {
        Some(l) => b.proxy_url(l.clone()),
        None => b,
    };
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    let b = match folder {
        Some(f) => b.data_directory(f.to_path_buf()),
        None => b,
    };
    #[cfg(target_os = "macos")]
    let b = {
        let _ = folder;
        b.incognito(true)
    };
    b
}

/// Places the bar and every stranger of `window`. On Linux GTK does the
/// placing, after the widgets are packed (linux_layout.rs).
pub fn layout(app: &AppHandle, window: &str) {
    let Some(strangers) = app.try_state::<std::sync::Arc<crate::strangers::Strangers>>() else {
        return;
    };
    let tiles = strangers.tiles(window);
    #[cfg(target_os = "linux")]
    if let Some(w) = app.get_window(window) {
        if let Err(e) = crate::linux_layout::arrange(&w, tiles) {
            eprintln!("unweb: window layout: {e}");
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let Some(w) = app.get_window(window) else {
            return;
        };
        let Ok(size) = w.inner_size() else {
            return;
        };
        let scale = w.scale_factor().unwrap_or(1.0);
        let (ww, wh) = (size.width as f64 / scale, size.height as f64 / scale);
        if let Some(bar) = app.get_webview(&bar_label(window)) {
            let _ = bar.set_position(LogicalPosition::new(0.0, 0.0));
            let _ = bar.set_size(LogicalSize::new(ww, BAR_HEIGHT));
        }
        let n = tiles.len();
        for (i, t) in tiles.iter().enumerate() {
            let (s, p) = tile_rects(n, i, ww, (wh - BAR_HEIGHT).max(0.0));
            for (label, r) in [(strip_label(t), s), (page_label(t), p)] {
                if let Some(v) = app.get_webview(&label) {
                    let _ = v.set_position(LogicalPosition::new(r[0], r[1]));
                    let _ = v.set_size(LogicalSize::new(r[2], r[3]));
                }
            }
        }
    }
}

/// The address a page shows, if it has one. On macOS wry's own call
/// panics when WKWebView has no address yet (a page that hasn't loaded, or
/// couldn't, such as one whose proxy is down), so the address is read
/// directly there. Call it off the main thread.
pub fn page_url(page: &tauri::Webview) -> Option<Url> {
    #[cfg(target_os = "macos")]
    {
        let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();
        page.with_webview(move |pv| unsafe {
            let ptr = pv.inner() as *const objc2_web_kit::WKWebView;
            let s = ptr
                .as_ref()
                .and_then(|wv| wv.URL())
                .and_then(|u| u.absoluteString())
                .map(|s| s.to_string());
            let _ = tx.send(s);
        })
        .ok()?;
        rx.recv_timeout(std::time::Duration::from_secs(5))
            .ok()
            .flatten()
            .and_then(|s| Url::parse(&s).ok())
    }
    #[cfg(not(target_os = "macos"))]
    {
        page.url().ok()
    }
}

/// Tells the bar what the window's first stranger is showing and whether
/// any of its strangers can go back or forward. Runs off the calling
/// thread, since asking the engine about its history needs the main
/// thread.
pub fn show_address(app: &AppHandle, window: &str) {
    let app = app.clone();
    let window = window.to_string();
    std::thread::spawn(move || {
        let Some(strangers) = app.try_state::<std::sync::Arc<crate::strangers::Strangers>>() else {
            return;
        };
        let tiles = strangers.tiles(&window);
        let Some(bar) = app.get_webview(&bar_label(&window)) else {
            return;
        };
        let pages: Vec<tauri::Webview> = tiles
            .iter()
            .filter_map(|t| app.get_webview(&page_label(t)))
            .collect();
        let Some(first) = pages.first() else {
            return;
        };
        let url = page_url(first).map(|u| u.to_string()).unwrap_or_default();
        // The start page is Unweb's own; the bar shows it empty.
        let shown = match Url::parse(&url) {
            Ok(u) if is_app_origin(&u) || u.scheme() == "about" => String::new(),
            _ => url,
        };
        let (mut back, mut forward) = (false, false);
        for p in &pages {
            let (b, f) = crate::nav::can_step(p).unwrap_or((true, true));
            back |= b;
            forward |= f;
        }
        let setups = app
            .try_state::<std::sync::Arc<crate::setups::Setups>>()
            .map(|s| s.names())
            .unwrap_or_default();
        let state = serde_json::json!({
            "setups": setups,
            "url": shown,
            "back": back,
            "forward": forward,
            "tiles": tiles.len(),
            "max": MAX_TILES,
        });
        let _ = bar.eval(format!("window.unwebShow && window.unwebShow({state})"));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        assert_eq!(page_label("stranger-3a"), "page-3a");
        assert_eq!(strip_label("stranger-3a"), "strip-3a");
        assert_eq!(bar_label("win-3a"), "bar-3a");
        assert_eq!(window_of_bar("bar-12ab").as_deref(), Some("win-12ab"));
        assert_eq!(
            stranger_of("page-0a1b2c3d4e5f6a7b").as_deref(),
            Some("stranger-0a1b2c3d4e5f6a7b")
        );
        assert_eq!(stranger_of("strip-4f").as_deref(), Some("stranger-4f"));
        assert_eq!(window_of_bar("bar-0A1B"), None);
        assert_eq!(window_of_bar("page-12"), None);
        assert_eq!(stranger_of("bar-12"), None);
        assert_eq!(window_of_bar("bar-"), None);
        assert_eq!(stranger_of("strip-1x"), None);
        assert_eq!(stranger_of("page-../x"), None);
        assert_eq!(window_of_bar("main"), None);
    }

    #[test]
    fn what_the_page_may_open() {
        let ok = |s: &str| page_may_open(&Url::parse(s).unwrap());
        let app = if cfg!(windows) {
            "http://tauri.localhost"
        } else {
            "tauri://localhost"
        };
        assert!(ok("https://example.com/x"));
        assert!(ok("http://127.0.0.1:8080/"));
        assert!(ok("about:blank"));
        assert!(ok("data:text/html,hi"));
        assert!(ok(&format!("{app}/index.html")));
        assert!(ok(&format!("{app}/")));
        assert!(ok(app_url("index.html").as_str()));
        assert!(!ok(&format!("{app}/bar.html")));
        assert!(!ok(&format!("{app}/strip.html")));
        assert!(!ok(&format!("{app}/index.html?x=1")));
        assert!(!ok(&format!("{app}/#x")));
        assert!(!ok("http://ipc.localhost/go"));
        assert!(!ok("file:///etc/passwd"));
        assert!(!ok("javascript:alert(1)"));
    }

    #[test]
    fn the_bar_and_strips_open_only_themselves() {
        let bar = |s: &str| bar_may_open(&Url::parse(s).unwrap());
        let strip = |s: &str| strip_may_open(&Url::parse(s).unwrap());
        let app = if cfg!(windows) {
            "http://tauri.localhost"
        } else {
            "tauri://localhost"
        };
        assert!(bar(&format!("{app}/bar.html")));
        assert!(strip(&format!("{app}/strip.html")));
        assert!(!bar(&format!("{app}/strip.html")));
        assert!(!strip(&format!("{app}/bar.html")));
        assert!(!bar(&format!("{app}/index.html")));
        assert!(!bar(&format!("{app}/bar.html?x")));
        assert!(!strip(&format!("{app}/strip.html?x")));
        // The app's host on another port is someone else's server.
        assert!(!bar("http://tauri.localhost:8765/bar.html"));
        assert!(!strip("http://tauri.localhost:8765/strip.html"));
        assert!(!bar("https://tauri.localhost/bar.html"));
        assert!(!bar("tauri://localhost:1/bar.html"));
        assert!(!bar("http://127.0.0.1:8765/fakebar.html"));
        assert!(!bar("data:text/html,x"));
        assert!(!strip("about:blank"));
    }

    #[test]
    fn grids() {
        assert_eq!(grid(1), (1, 1));
        assert_eq!(grid(2), (2, 1));
        assert_eq!(grid(3), (3, 1));
        assert_eq!(grid(4), (2, 2));
        // Four tiles fill the area exactly, without overlap.
        let (w, h) = (1001.0, 701.0);
        let mut area = 0.0;
        for i in 0..4 {
            let (s, p) = tile_rects(4, i, w, h);
            assert_eq!(s[0], p[0]);
            assert_eq!(s[2], p[2]);
            assert_eq!(p[1], s[1] + STRIP_HEIGHT);
            area += s[2] * (s[3] + p[3]);
        }
        assert_eq!(area, w * h);
        let (s0, _) = tile_rects(3, 0, 900.0, 500.0);
        let (s2, p2) = tile_rects(3, 2, 900.0, 500.0);
        assert_eq!((s0[0], s0[1], s0[2]), (0.0, BAR_HEIGHT, 300.0));
        assert_eq!(s2[0] + s2[2], 900.0);
        assert_eq!(p2[1] + p2[3], BAR_HEIGHT + 500.0);
    }

    #[test]
    fn languages() {
        assert_eq!(known_language("de-DE"), Some("de-DE"));
        assert_eq!(known_language(""), Some(""));
        assert_eq!(known_language("xx-YY"), None);
        assert_eq!(known_language("de-DE --remote-debugging-port=1"), None);
        assert!(LANGUAGES
            .iter()
            .all(|(t, _)| t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')));
    }
}
