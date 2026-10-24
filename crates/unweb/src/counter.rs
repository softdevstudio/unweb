//! The cookie counter: what each stranger has been given.
//!
//! Every two seconds, and whenever a page loads, Unweb reads the stranger's
//! cookie store from the engine (names and domains only, never values),
//! counts it against the site the stranger is showing (unweb-core) and
//! shows the result in the stranger's strip. A stranger's store starts
//! empty, so everything counted was handed to this stranger since it
//! opened.

use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Manager};
use unweb_core::{count_cookies, describe, StoredCookie};

use crate::strangers::Strangers;
use crate::window::{
    page_label, strip_label, LANGUAGES, LANGUAGE_PER_STRANGER, NO_LANGUAGE_REASON,
};

/// Starts the loop that refreshes every stranger's count.
pub fn start(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(2));
        let Some(strangers) = app.try_state::<Arc<Strangers>>() else {
            continue;
        };
        for s in strangers.open_labels() {
            refresh(&app, &strangers, &s);
        }
    });
}

/// Refreshes one stranger's count, off the calling thread (reading the
/// cookie store waits for the main thread).
pub fn refresh_one(app: &AppHandle, stranger: &str) {
    let app = app.clone();
    let stranger = stranger.to_string();
    std::thread::spawn(move || {
        if let Some(strangers) = app.try_state::<Arc<Strangers>>() {
            refresh(&app, &strangers, &stranger);
        }
    });
}

fn refresh(app: &AppHandle, strangers: &Strangers, stranger: &str) {
    let (Some(page), Some(strip)) = (
        app.get_webview(&page_label(stranger)),
        app.get_webview(&strip_label(stranger)),
    ) else {
        return;
    };
    let url = crate::window::page_url(&page)
        .map(|u| u.to_string())
        .unwrap_or_default();
    let web = url.starts_with("http://") || url.starts_with("https://");
    let cookies = read_cookies(&page);
    let count = count_cookies(if web { &url } else { "" }, &cookies);
    let text = if web {
        describe(&count)
    } else if cookies.is_empty() {
        "No cookies yet".to_string()
    } else {
        describe(&count)
    };
    strangers.set_count(stranger, count.clone());
    let settings = strangers.settings(stranger);
    let lang = settings.lang.clone();
    let tiles = strangers
        .window_of(stranger)
        .map(|w| strangers.tiles(&w).len())
        .unwrap_or(1);
    let state = serde_json::json!({
        "cookies": text,
        "thisSite": count.this_site,
        "otherSites": count.other_sites,
        "site": count.site,
        "lang": lang,
        "langs": LANGUAGES.iter().map(|(t, n)| [*t, *n]).collect::<Vec<_>>(),
        "langPerStranger": LANGUAGE_PER_STRANGER,
        "noLangReason": NO_LANGUAGE_REASON,
        "tiles": tiles,
        "location": crate::location::show(settings.location.as_ref()),
        "locationNote": crate::location::supported().err().unwrap_or(""),
    });
    let _ = strip.eval(format!("window.unwebStrip && window.unwebStrip({state})"));
}

/// The cookies in a page's store: names, domains and whether scripts can
/// see them, never values.
#[cfg(not(target_os = "macos"))]
fn read_cookies(page: &tauri::Webview) -> Vec<StoredCookie> {
    page.cookies()
        .unwrap_or_default()
        .into_iter()
        .map(|c| StoredCookie {
            name: c.name().to_string(),
            domain: c.domain().unwrap_or_default().to_string(),
            http_only: c.http_only().unwrap_or(false),
        })
        .collect()
}

/// On macOS wry's cookies() waits by running the main thread's event loop
/// from inside the main thread, which can stall or break whatever else is
/// queued there (the self-test saw windows fail to open). So the store is
/// asked directly, and this thread waits for the answer instead.
#[cfg(target_os = "macos")]
fn read_cookies(page: &tauri::Webview) -> Vec<StoredCookie> {
    let (tx, rx) = std::sync::mpsc::channel::<Vec<StoredCookie>>();
    let asked = page.with_webview(move |pv| unsafe {
        let ptr = pv.inner() as *const objc2_web_kit::WKWebView;
        let Some(wv) = ptr.as_ref() else {
            let _ = tx.send(Vec::new());
            return;
        };
        let store = wv.configuration().websiteDataStore().httpCookieStore();
        let done = block2::RcBlock::new(
            move |cookies: std::ptr::NonNull<
                objc2_foundation::NSArray<objc2_foundation::NSHTTPCookie>,
            >| {
                let list = cookies
                    .as_ref()
                    .to_vec()
                    .into_iter()
                    .map(|c| StoredCookie {
                        name: c.name().to_string(),
                        domain: c.domain().to_string(),
                        http_only: c.isHTTPOnly(),
                    })
                    .collect();
                let _ = tx.send(list);
            },
        );
        store.getAllCookies(&done);
    });
    if asked.is_err() {
        return Vec::new();
    }
    rx.recv_timeout(Duration::from_secs(5)).unwrap_or_default()
}
