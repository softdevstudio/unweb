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
    let url = page.url().map(|u| u.to_string()).unwrap_or_default();
    let web = url.starts_with("http://") || url.starts_with("https://");
    let cookies: Vec<StoredCookie> = page
        .cookies()
        .unwrap_or_default()
        .into_iter()
        .map(|c| StoredCookie {
            name: c.name().to_string(),
            domain: c.domain().unwrap_or_default().to_string(),
            http_only: c.http_only().unwrap_or(false),
        })
        .collect();
    let count = count_cookies(if web { &url } else { "" }, &cookies);
    let text = if web {
        describe(&count)
    } else if cookies.is_empty() {
        "No cookies yet".to_string()
    } else {
        describe(&count)
    };
    strangers.set_count(stranger, count.clone());
    let lang = strangers.lang(stranger);
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
    });
    let _ = strip.eval(format!("window.unwebStrip && window.unwebStrip({state})"));
}
