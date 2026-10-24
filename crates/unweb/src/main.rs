#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

//! Unweb: a tiny browser where every visit is a first visit.
//!
//! Every stranger starts with no history, no logins and no cookies, keeps
//! everything it collects in a throwaway store of its own, and when it
//! closes that store is deleted. A window shows one to four strangers side
//! by side under one bar. See strangers.rs and window.rs.

mod capture;
mod compare;
mod counter;
mod hsts;
#[cfg(target_os = "linux")]
mod linux_layout;
mod location;
mod nav;
mod relay;
mod selftest;
mod setups;
mod strangers;
mod testproxy;
mod window;

use std::sync::Arc;

use tauri::{Manager, RunEvent, Url, WindowEvent};

use strangers::{Settings, Strangers};
use window::WINDOW_PREFIX;

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

fn strangers_of(webview: &tauri::Webview) -> Result<Arc<Strangers>, String> {
    Ok(webview
        .app_handle()
        .try_state::<Arc<Strangers>>()
        .ok_or("not ready")?
        .inner()
        .clone())
}

/// The pages of the window whose bar is `bar` (a bar can only steer its own
/// window's strangers).
fn pages_for(bar: &tauri::Webview) -> Result<Vec<tauri::Webview>, String> {
    let win = window::window_of_bar(bar.label()).ok_or("not a window's bar")?;
    let strangers = strangers_of(bar)?;
    let pages: Vec<tauri::Webview> = strangers
        .tiles(&win)
        .iter()
        .filter_map(|t| bar.app_handle().get_webview(&window::page_label(t)))
        .collect();
    if pages.is_empty() {
        return Err("this window has no strangers".into());
    }
    Ok(pages)
}

/// The stranger whose strip is `strip` (a strip can only change its own
/// stranger).
fn stranger_for(strip: &tauri::Webview) -> Result<String, String> {
    match window::stranger_of(strip.label()) {
        Some(s) if strip.label().starts_with("strip-") => Ok(s),
        _ => Err("not a stranger's strip".into()),
    }
}

/// The bar's address field: opens what was typed in every stranger of
/// this window.
#[tauri::command]
async fn go(webview: tauri::Webview, text: String) -> Result<(), String> {
    let url = normalize_url(&text).ok_or("Type a web address, like example.com")?;
    let pages = pages_for(&webview)?;
    if let Some(win) = window::window_of_bar(webview.label()) {
        strangers_of(&webview)?.set_typed(&win, url.clone());
    }
    for p in pages {
        p.navigate(url.clone()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn back(webview: tauri::Webview) -> Result<(), String> {
    for p in pages_for(&webview)? {
        nav::step(&p, nav::Step::Back)?;
    }
    Ok(())
}

#[tauri::command]
async fn forward(webview: tauri::Webview) -> Result<(), String> {
    for p in pages_for(&webview)? {
        nav::step(&p, nav::Step::Forward)?;
    }
    Ok(())
}

#[tauri::command]
async fn reload(webview: tauri::Webview) -> Result<(), String> {
    for p in pages_for(&webview)? {
        p.reload().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Opens another window with a new stranger in it.
#[tauri::command]
async fn new_stranger(webview: tauri::Webview) -> Result<(), String> {
    window::window_of_bar(webview.label()).ok_or("not a window's bar")?;
    let app = webview.app_handle().clone();
    strangers_of(&webview)?
        .open(&app, None)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Adds a stranger to this window, beside the others, at the address last
/// typed in the bar. Not at the address the others are on now: a site can
/// put an identifier in its own address, and the new stranger would carry
/// it.
#[tauri::command]
async fn side_by_side(webview: tauri::Webview) -> Result<(), String> {
    let win = window::window_of_bar(webview.label()).ok_or("not a window's bar")?;
    let strangers = strangers_of(&webview)?;
    let app = webview.app_handle().clone();
    let url = strangers.typed(&win);
    strangers
        .add_tile(&app, &win, url, &Settings::default(), None)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// A strip's language menu: gives its stranger another language, which
/// means a new stranger in its place (see Strangers::change_settings).
#[tauri::command]
async fn set_language(webview: tauri::Webview, lang: String) -> Result<(), String> {
    let stranger = stranger_for(&webview)?;
    if !window::LANGUAGE_PER_STRANGER {
        return Err(window::NO_LANGUAGE_REASON.into());
    }
    let lang = window::known_language(&lang).ok_or("not a language Unweb offers")?;
    let app = webview.app_handle().clone();
    let strangers = strangers_of(&webview)?;
    let settings = Settings {
        lang: lang.to_string(),
        ..strangers.settings(&stranger)
    };
    strangers
        .change_settings(&app, &stranger, &settings)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// A strip's location field: gives its stranger a proxy (or none), which
/// means a new stranger in its place.
#[tauri::command]
async fn set_location(webview: tauri::Webview, text: String) -> Result<(), String> {
    let stranger = stranger_for(&webview)?;
    let location = location::parse_location(&text)?;
    if location.is_some() {
        location::supported()?;
    }
    let app = webview.app_handle().clone();
    let strangers = strangers_of(&webview)?;
    let settings = Settings {
        location,
        ..strangers.settings(&stranger)
    };
    if settings == strangers.settings(&stranger) {
        return Ok(());
    }
    strangers
        .change_settings(&app, &stranger, &settings)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn setups_of(webview: &tauri::Webview) -> Result<Arc<setups::Setups>, String> {
    Ok(webview
        .app_handle()
        .try_state::<Arc<setups::Setups>>()
        .ok_or("not ready")?
        .inner()
        .clone())
}

/// Tells every window's bar the setups there are now.
fn show_setups_everywhere(app: &tauri::AppHandle) {
    if let Some(s) = app.try_state::<Arc<Strangers>>() {
        for w in s.open_windows() {
            window::show_address(app, &w);
        }
    }
}

/// Saves this window's strangers (their languages and locations, nothing
/// they saw) as a setup called `name`.
#[tauri::command]
async fn save_setup(webview: tauri::Webview, name: String) -> Result<(), String> {
    let win = window::window_of_bar(webview.label()).ok_or("not a window's bar")?;
    let strangers = strangers_of(&webview)?;
    let settings: Vec<Settings> = strangers
        .tiles(&win)
        .iter()
        .map(|t| strangers.settings(t))
        .collect();
    setups_of(&webview)?.save(&name, &settings)?;
    show_setups_everywhere(webview.app_handle());
    Ok(())
}

/// Opens a new window with the strangers of setup `name`, each with a new
/// store of its own, at the start page.
#[tauri::command]
async fn open_setup(webview: tauri::Webview, name: String) -> Result<(), String> {
    window::window_of_bar(webview.label()).ok_or("not a window's bar")?;
    let settings = setups_of(&webview)?.get(&name)?;
    let app = webview.app_handle().clone();
    strangers_of(&webview)?
        .open_window(&app, settings.into_iter().map(|s| (None, s)).collect())
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn delete_setup(webview: tauri::Webview, name: String) -> Result<(), String> {
    window::window_of_bar(webview.label()).ok_or("not a window's bar")?;
    setups_of(&webview)?.delete(&name)?;
    show_setups_everywhere(webview.app_handle());
    Ok(())
}

/// Compare: screenshots of every stranger in this window, side by side, in
/// a window of their own.
#[tauri::command]
async fn compare(webview: tauri::Webview) -> Result<(), String> {
    let win = window::window_of_bar(webview.label()).ok_or("not a window's bar")?;
    let strangers = strangers_of(&webview)?;
    let app = webview.app_handle().clone();
    compare::open(&app, &strangers, &win).map(|_| ())
}

/// The compare window's Save image: writes the strip it drew as a PNG in
/// the Downloads folder and says where.
#[tauri::command]
async fn save_compare(webview: tauri::Webview, image: String) -> Result<String, String> {
    if !webview.label().starts_with(compare::VIEW_PREFIX) {
        return Err("not a compare window".into());
    }
    let c = webview
        .app_handle()
        .try_state::<Arc<compare::Compare>>()
        .ok_or("not ready")?
        .inner()
        .clone();
    compare::save(&c, &image).map(|p| p.display().to_string())
}

/// A strip's close button: closes its stranger (its whole window when it is
/// the only one).
#[tauri::command]
async fn close_stranger(webview: tauri::Webview) -> Result<(), String> {
    let stranger = stranger_for(&webview)?;
    let app = webview.app_handle().clone();
    let strangers = strangers_of(&webview)?;
    std::thread::spawn(move || strangers.retire(&app, &stranger));
    Ok(())
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
            new_stranger,
            side_by_side,
            set_language,
            set_location,
            close_stranger,
            save_setup,
            open_setup,
            delete_setup,
            compare,
            save_compare
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            let root = handle.path().app_data_dir()?.join("strangers");
            let strangers = Arc::new(Strangers::new(root));
            // The self-test keeps its setups in its own folder, never with
            // the person's.
            let setups_path = match &selftest_dir {
                Some(dir) => dir.join("setups.json"),
                None => handle.path().app_config_dir()?.join("setups.json"),
            };
            app.manage(Arc::new(setups::Setups::new(setups_path)));
            // Saved strips go to Downloads (the self-test's to its folder).
            let compare_dir = match &selftest_dir {
                Some(dir) => dir.join("compare"),
                None => handle
                    .path()
                    .download_dir()
                    .or_else(|_| handle.path().home_dir())?,
            };
            app.manage(Arc::new(compare::Compare::new(
                compare_dir,
                selftest_dir.is_some(),
            )));
            let _ = setup_holder.set(strangers.clone());
            app.manage(strangers.clone());
            let start = match &selftest_dir {
                Some(dir) => Some(selftest::start(&handle, strangers.clone(), dir.clone())?),
                None => start_url_from_args(),
            };
            counter::start(handle.clone());
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
        } if label.starts_with(WINDOW_PREFIX) => {
            if let Some(s) = holder.get().cloned() {
                std::thread::spawn(move || s.window_closed(&label));
            }
        }
        // A compare window is gone: delete its small store.
        RunEvent::WindowEvent {
            label,
            event: WindowEvent::Destroyed,
            ..
        } if label.starts_with(compare::PREFIX) => {
            if let Some(c) = _app.try_state::<Arc<compare::Compare>>() {
                c.forget(&label);
            }
            if let Some(s) = holder.get().cloned() {
                std::thread::spawn(move || {
                    s.closed(&label);
                });
            }
        }
        // Closing a window goes through `retire`, which clears the engine's
        // data first.
        RunEvent::WindowEvent {
            label,
            event: WindowEvent::CloseRequested { api, .. },
            ..
        } if label.starts_with(WINDOW_PREFIX) => {
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
