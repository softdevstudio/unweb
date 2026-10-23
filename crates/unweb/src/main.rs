#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

//! Unweb: a tiny browser where every visit is a first visit.
//!
//! Every window is a stranger: it starts with no history, no logins and no
//! cookies, keeps everything it collects in a throwaway store of its own,
//! and when it closes that store is deleted. See strangers.rs.

#[cfg(target_os = "linux")]
mod linux_layout;
mod nav;
mod selftest;
mod strangers;
mod window;

use std::sync::Arc;

use tauri::{Manager, RunEvent, Url, WindowEvent};

use strangers::{Strangers, PREFIX};

/// Turns what someone typed into an address: `example.com` becomes
/// `https://example.com/`. Only http and https open in a stranger.
pub fn normalize_url(input: &str) -> Option<Url> {
    let text = input.trim();
    if text.is_empty() || text.contains(char::is_whitespace) {
        return None;
    }
    let candidate = if text.contains("://") {
        text.to_string()
    } else if text.contains('.') || text.starts_with("localhost") {
        format!("https://{text}")
    } else {
        return None;
    };
    let url = Url::parse(&candidate).ok()?;
    match url.scheme() {
        "http" | "https" if url.host_str().is_some() => Some(url),
        _ => None,
    }
}

/// The page webview that belongs to the bar `bar` (a bar can only steer its
/// own stranger's page).
fn page_for(bar: &tauri::Webview) -> Result<tauri::Webview, String> {
    let window = window::window_of(bar.label()).ok_or("not a stranger's bar")?;
    bar.app_handle()
        .get_webview(&window::page_label(&window))
        .ok_or_else(|| "this stranger is gone".to_string())
}

/// The bar's address field: opens what was typed in this stranger.
#[tauri::command]
async fn go(webview: tauri::Webview, text: String) -> Result<(), String> {
    let url = normalize_url(&text).ok_or("Type a web address, like example.com")?;
    page_for(&webview)?.navigate(url).map_err(|e| e.to_string())
}

#[tauri::command]
async fn back(webview: tauri::Webview) -> Result<(), String> {
    nav::step(&page_for(&webview)?, nav::Step::Back)
}

#[tauri::command]
async fn forward(webview: tauri::Webview) -> Result<(), String> {
    nav::step(&page_for(&webview)?, nav::Step::Forward)
}

#[tauri::command]
async fn reload(webview: tauri::Webview) -> Result<(), String> {
    page_for(&webview)?.reload().map_err(|e| e.to_string())
}

/// Opens another stranger, with a store of its own.
#[tauri::command]
async fn new_stranger(webview: tauri::Webview) -> Result<(), String> {
    window::window_of(webview.label()).ok_or("not a stranger's bar")?;
    let app = webview.app_handle().clone();
    let strangers = app
        .try_state::<Arc<Strangers>>()
        .ok_or("not ready")?
        .inner()
        .clone();
    strangers
        .open(&app, None)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// The first command-line argument that is a web address.
fn start_url_from_args() -> Option<Url> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    args.iter()
        .enumerate()
        .filter(|(i, a)| !a.starts_with('-') && (*i == 0 || args[i - 1] != "--selftest"))
        .find_map(|(_, a)| normalize_url(a))
}

/// Mesa (and NVIDIA's Linux driver) keep compiled GPU shaders in a disk
/// cache in the user's home folder, shared by every program. A page could
/// fill it in one stranger and find its entries again, by timing, in the
/// next. Turned off before any engine process starts, so they inherit it.
fn no_shared_shader_cache() {
    #[cfg(target_os = "linux")]
    for (k, v) in [
        ("MESA_SHADER_CACHE_DISABLE", "true"),
        ("MESA_GLSL_CACHE_DISABLE", "true"),
        ("__GL_SHADER_DISK_CACHE", "0"),
    ] {
        std::env::set_var(k, v);
    }
}

fn main() {
    no_shared_shader_cache();
    let selftest_dir = selftest::requested_dir();
    let selftest_running = selftest_dir.is_some();
    let holder: Arc<std::sync::OnceLock<Arc<Strangers>>> = Arc::new(std::sync::OnceLock::new());
    let setup_holder = holder.clone();

    let built = tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            go,
            back,
            forward,
            reload,
            new_stranger
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            let root = handle.path().app_data_dir()?.join("strangers");
            let strangers = Arc::new(Strangers::new(root));
            let _ = setup_holder.set(strangers.clone());
            app.manage(strangers.clone());
            let start = match &selftest_dir {
                Some(dir) => Some(selftest::start(&handle, strangers.clone(), dir.clone())?),
                None => start_url_from_args(),
            };
            strangers.open(&handle, start)?;
            Ok(())
        })
        .build(tauri::generate_context!());
    let app = match built {
        Ok(app) => app,
        Err(e) => {
            eprintln!("unweb: {e}");
            std::process::exit(1);
        }
    };
    app.run(move |_app, event| match event {
        RunEvent::WindowEvent {
            label,
            event: WindowEvent::Destroyed,
            ..
        } if label.starts_with(PREFIX) => {
            if let Some(s) = holder.get().cloned() {
                std::thread::spawn(move || {
                    let _ = s.closed(&label);
                });
            }
        }
        // Closing a stranger goes through `retire`, which clears the
        // engine's data first.
        RunEvent::WindowEvent {
            label,
            event: WindowEvent::CloseRequested { api, .. },
            ..
        } if label.starts_with(PREFIX) => {
            api.prevent_close();
            if let Some(s) = holder.get().cloned() {
                let handle = _app.clone();
                std::thread::spawn(move || s.retire(&handle, &label));
            }
        }
        // The self-test closes every stranger and still has checks to make;
        // it ends the app itself.
        RunEvent::ExitRequested {
            code: None, api, ..
        } if selftest_running => {
            api.prevent_exit();
        }
        // The last stranger closed: finish deleting the strangers' data
        // before the app goes, then exit.
        RunEvent::ExitRequested {
            code: None, api, ..
        } => {
            if let Some(s) = holder.get().cloned() {
                api.prevent_exit();
                let handle = _app.clone();
                std::thread::spawn(move || {
                    // Give the closed window's wipe a moment to start.
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    s.wait_for_wipes(std::time::Duration::from_secs(90));
                    handle.exit(0);
                });
            }
        }
        RunEvent::Exit => {
            if let Some(s) = holder.get() {
                s.wipe_all();
            }
        }
        _ => {}
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        assert_eq!(
            normalize_url("example.com").unwrap().as_str(),
            "https://example.com/"
        );
        assert_eq!(
            normalize_url("http://localhost:8080/x").unwrap().as_str(),
            "http://localhost:8080/x"
        );
        assert!(normalize_url("file:///etc/passwd").is_none());
        assert!(normalize_url("javascript:alert(1)").is_none());
        assert!(normalize_url("two words").is_none());
        assert!(normalize_url("").is_none());
    }
}
