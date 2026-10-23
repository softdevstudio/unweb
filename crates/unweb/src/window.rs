//! A stranger's window: the bar on top and the page below.
//!
//! Each window holds two webviews. The bar ("bar-N") is Unweb's own page
//! with the address field and buttons; it is the only webview granted app
//! commands (capabilities/bar.json). The page ("page-N") shows the web and
//! is granted nothing. Both use the stranger's own store, so the bar adds
//! nothing that outlives the stranger.

use std::path::Path;

use tauri::webview::{DownloadEvent, NewWindowResponse, PageLoadEvent, WebviewBuilder};
use tauri::window::WindowBuilder;
#[cfg(not(target_os = "linux"))]
use tauri::WindowEvent;
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Url, WebviewUrl};

use crate::strangers::PREFIX;

/// Height of the bar in logical pixels.
pub const BAR_HEIGHT: f64 = 44.0;
const WIDTH: f64 = 1100.0;
const HEIGHT: f64 = 760.0;

/// The page webview's label for a stranger's window label.
pub fn page_label(window: &str) -> String {
    format!("page-{}", window.strip_prefix(PREFIX).unwrap_or(window))
}

/// The bar webview's label for a stranger's window label.
pub fn bar_label(window: &str) -> String {
    format!("bar-{}", window.strip_prefix(PREFIX).unwrap_or(window))
}

/// The stranger's window label for a bar or page label.
pub fn window_of(webview: &str) -> Option<String> {
    let n = webview
        .strip_prefix("bar-")
        .or_else(|| webview.strip_prefix("page-"))?;
    (!n.is_empty() && n.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
        .then(|| format!("{PREFIX}{n}"))
}

/// True for Unweb's own pages (the start page and the bar), which are
/// served from the app's origin.
fn is_app_origin(url: &Url) -> bool {
    url.scheme() == "tauri"
        || matches!(url.host_str(), Some("tauri.localhost"))
            && matches!(url.scheme(), "http" | "https")
}

/// What the page may open. Web pages, plus the start page Unweb itself
/// shows; never the bar or anything else of the app's.
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

/// Builds the window, its bar and its page. `folder` is the stranger's
/// data folder on Windows and Linux.
pub fn build(
    app: &AppHandle,
    label: &str,
    n: u64,
    folder: Option<&Path>,
    target: WebviewUrl,
) -> tauri::Result<()> {
    let window = WindowBuilder::new(app, label)
        .title(format!("Unweb: stranger {n}"))
        .inner_size(WIDTH, HEIGHT)
        .min_inner_size(420.0, 240.0);
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
    )
    .disable_drag_drop_handler()
    .on_navigation(bar_may_open)
    .on_new_window(|_, _| NewWindowResponse::Deny)
    .on_download(|_, _| false)
    // The page can finish loading before the bar is ready to show it.
    .on_page_load(move |_, payload| {
        if payload.event() == PageLoadEvent::Finished {
            show_address(&bar_app, &bar_window);
        }
    });

    let page_app = app.clone();
    let window_label = label.to_string();
    let popup_app = app.clone();
    let popup_label = label.to_string();
    let title_app = app.clone();
    let title_label = label.to_string();
    let page = store(WebviewBuilder::new(page_label(label), target), folder)
        .disable_drag_drop_handler()
        .on_navigation(page_may_open)
        // A new window would need a store of its own; open it here instead,
        // so it stays with this stranger.
        .on_new_window(move |url, _| {
            if page_may_open(&url) && !is_app_origin(&url) {
                if let Some(p) = popup_app.get_webview(&page_label(&popup_label)) {
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
                show_address(&page_app, &window_label);
            }
        })
        .on_document_title_changed(move |_, _| show_address(&title_app, &title_label));

    let width = WIDTH;
    let height = HEIGHT;
    window.add_child(
        bar,
        LogicalPosition::new(0.0, 0.0),
        LogicalSize::new(width, BAR_HEIGHT),
    )?;
    let page_view = window.add_child(
        page,
        LogicalPosition::new(0.0, BAR_HEIGHT),
        LogicalSize::new(width, height - BAR_HEIGHT),
    )?;

    #[cfg(target_os = "linux")]
    if let Err(e) = crate::linux_layout::arrange(&window) {
        eprintln!("unweb: window layout: {e}");
    }
    #[cfg(not(target_os = "linux"))]
    {
        let w = window.clone();
        let l = label.to_string();
        window.on_window_event(move |e| {
            if let WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } = e {
                fit(&w, &l);
            }
        });
        fit(&window, label);
    }

    // WebKitGTK can offer to save HTTP sign-in passwords in the system
    // keyring, which would outlive the stranger.
    #[cfg(target_os = "linux")]
    let _ = page_view.with_webview(|pv| {
        use webkit2gtk::{WebViewExt, WebsiteDataManagerExt};
        if let Some(m) = pv.inner().website_data_manager() {
            m.set_persistent_credential_storage_enabled(false);
        }
    });
    #[cfg(not(target_os = "linux"))]
    let _ = page_view;
    Ok(())
}

/// Gives a webview the stranger's store.
fn store<R: tauri::Runtime>(b: WebviewBuilder<R>, folder: Option<&Path>) -> WebviewBuilder<R> {
    // WebView2 keeps compiled GPU shaders in a disk cache a page can fill
    // and later probe by timing. Turn it off (and keep wry's own default
    // switches, which this replaces). Both webviews must ask for the same
    // switches, since they share one WebView2 environment.
    #[cfg(target_os = "windows")]
    let b = b.additional_browser_args(
        "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --disable-gpu-shader-disk-cache",
    );
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

/// Places the bar and the page in the window (Windows and macOS; on Linux
/// GTK does it, see linux_layout.rs).
#[cfg(not(target_os = "linux"))]
fn fit(window: &tauri::Window, label: &str) {
    let Ok(size) = window.inner_size() else {
        return;
    };
    let scale = window.scale_factor().unwrap_or(1.0);
    let (w, h) = (size.width as f64 / scale, size.height as f64 / scale);
    let app = window.app_handle();
    if let Some(bar) = app.get_webview(&bar_label(label)) {
        let _ = bar.set_position(LogicalPosition::new(0.0, 0.0));
        let _ = bar.set_size(LogicalSize::new(w, BAR_HEIGHT));
    }
    if let Some(page) = app.get_webview(&page_label(label)) {
        let _ = page.set_position(LogicalPosition::new(0.0, BAR_HEIGHT));
        let _ = page.set_size(LogicalSize::new(w, (h - BAR_HEIGHT).max(0.0)));
    }
}

/// Tells the bar what the page is showing and whether it can go back or
/// forward. Runs off the calling thread, since asking the engine about
/// its history needs the main thread.
pub fn show_address(app: &AppHandle, window: &str) {
    let app = app.clone();
    let window = window.to_string();
    std::thread::spawn(move || {
        let (Some(page), Some(bar)) = (
            app.get_webview(&page_label(&window)),
            app.get_webview(&bar_label(&window)),
        ) else {
            return;
        };
        let url = page.url().map(|u| u.to_string()).unwrap_or_default();
        // The start page is Unweb's own; the bar shows it empty.
        let shown = match Url::parse(&url) {
            Ok(u) if is_app_origin(&u) => String::new(),
            _ => url,
        };
        let (back, forward) = crate::nav::can_step(&page).unwrap_or((true, true));
        let state = serde_json::json!({ "url": shown, "back": back, "forward": forward });
        let _ = bar.eval(format!("window.unwebShow && window.unwebShow({state})"));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        assert_eq!(page_label("stranger-3"), "page-3");
        assert_eq!(bar_label("stranger-3"), "bar-3");
        assert_eq!(window_of("bar-12").as_deref(), Some("stranger-12"));
        assert_eq!(
            window_of("page-0a1b2c3d4e5f6a7b").as_deref(),
            Some("stranger-0a1b2c3d4e5f6a7b")
        );
        assert_eq!(window_of("bar-0A1B"), None);
        assert_eq!(window_of("page-4").as_deref(), Some("stranger-4"));
        assert_eq!(window_of("bar-"), None);
        assert_eq!(window_of("bar-1x"), None);
        assert_eq!(window_of("bar-../x"), None);
        assert_eq!(window_of("main"), None);
    }

    #[test]
    fn what_the_page_may_open() {
        let ok = |s: &str| page_may_open(&Url::parse(s).unwrap());
        assert!(ok("https://example.com/x"));
        assert!(ok("http://127.0.0.1:8080/"));
        assert!(ok("about:blank"));
        assert!(ok("data:text/html,hi"));
        assert!(ok("tauri://localhost/index.html"));
        assert!(ok("tauri://localhost"));
        assert!(ok("http://tauri.localhost/"));
        assert!(!ok("tauri://localhost/bar.html"));
        assert!(!ok("https://tauri.localhost/bar.html"));
        assert!(!ok("http://ipc.localhost/go"));
        assert!(!ok("file:///etc/passwd"));
        assert!(!ok("javascript:alert(1)"));
        assert!(!ok("tauri://localhost/index.html?x=1"));
        assert!(!ok("tauri://localhost/#x"));
        let bar = |s: &str| bar_may_open(&Url::parse(s).unwrap());
        assert!(bar("tauri://localhost/bar.html"));
        assert!(bar("http://tauri.localhost/bar.html"));
        assert!(!bar("tauri://localhost/index.html"));
        assert!(!bar("tauri://localhost/bar.html?x"));
        assert!(!bar("http://127.0.0.1:8765/fakebar.html"));
        assert!(!bar("data:text/html,x"));
        assert!(!bar("about:blank"));
    }
}
