//! Back and forward through each engine's own calls.
//!
//! Running `history.back()` inside the page would call whatever the page
//! put there, so a site could trap the Back button. These native calls
//! can't be intercepted by page code.

use std::sync::mpsc;
use std::time::Duration;

/// Which way to move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Back,
    Forward,
}

/// Moves the page one step through its history, if it can.
pub fn step(view: &tauri::Webview, step: Step) -> Result<(), String> {
    view.with_webview(move |platform| {
        #[cfg(target_os = "linux")]
        {
            use webkit2gtk::WebViewExt;
            let wv = platform.inner();
            match step {
                Step::Back if wv.can_go_back() => wv.go_back(),
                Step::Forward if wv.can_go_forward() => wv.go_forward(),
                _ => {}
            }
        }
        #[cfg(target_os = "windows")]
        unsafe {
            if let Ok(core) = platform.controller().CoreWebView2() {
                let _ = match step {
                    Step::Back => core.GoBack(),
                    Step::Forward => core.GoForward(),
                };
            }
        }
        #[cfg(target_os = "macos")]
        unsafe {
            let ptr = platform.inner() as *const objc2_web_kit::WKWebView;
            if let Some(wv) = ptr.as_ref() {
                let _ = match step {
                    Step::Back => wv.goBack(),
                    Step::Forward => wv.goForward(),
                };
            }
        }
    })
    .map_err(|e| e.to_string())
}

/// Whether the page can go back and forward. Waits for the main thread,
/// so never call it from there.
pub fn can_step(view: &tauri::Webview) -> Option<(bool, bool)> {
    let (tx, rx) = mpsc::channel();
    view.with_webview(move |platform| {
        #[cfg(target_os = "linux")]
        let answer = {
            use webkit2gtk::WebViewExt;
            let wv = platform.inner();
            (wv.can_go_back(), wv.can_go_forward())
        };
        #[cfg(target_os = "windows")]
        let answer = unsafe {
            let mut back = windows_core_bool();
            let mut fwd = windows_core_bool();
            if let Ok(core) = platform.controller().CoreWebView2() {
                let _ = core.CanGoBack(&mut back);
                let _ = core.CanGoForward(&mut fwd);
            }
            (back.as_bool(), fwd.as_bool())
        };
        #[cfg(target_os = "macos")]
        let answer = unsafe {
            let ptr = platform.inner() as *const objc2_web_kit::WKWebView;
            match ptr.as_ref() {
                Some(wv) => (wv.canGoBack(), wv.canGoForward()),
                None => (false, false),
            }
        };
        let _ = tx.send(answer);
    })
    .ok()?;
    rx.recv_timeout(Duration::from_secs(2)).ok()
}

#[cfg(target_os = "windows")]
fn windows_core_bool() -> windows_core::BOOL {
    Default::default()
}
