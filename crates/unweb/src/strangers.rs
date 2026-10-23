//! Strangers: browser windows that each get a throwaway store of their own.
//!
//! On Windows and Linux every stranger gets its own data folder under
//! `<app data>/strangers/`, which the engine uses for cookies, storage,
//! cache, service workers and everything else. Two strangers never share a
//! folder, so they never share anything. When a stranger's window closes,
//! its folder is deleted, and the deletion is checked: the folder must be
//! gone and stay gone. On macOS each stranger gets a non-persistent
//! WKWebsiteDataStore of its own, which keeps nothing on disk to delete.
//!
//! Folders left behind by a crash are deleted the next time Unweb starts.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager, Url, WebviewUrl, WebviewWindowBuilder};

/// Window labels of strangers start with this.
pub const PREFIX: &str = "stranger-";

/// Whether this platform gives each stranger a data folder of its own.
pub const USES_FOLDERS: bool = cfg!(any(target_os = "windows", target_os = "linux"));

/// How long a closed stranger's folder may take to go (the engine's
/// processes can hold files open for a moment after the window closes).
const WIPE_WITHIN: Duration = Duration::from_secs(30);
/// How long the folder must go unchanged before it is deleted. WebKitGTK
/// writes cache records a few seconds after a page closes.
const SETTLE: Duration = Duration::from_secs(4);
/// The longest to wait for the folder to settle.
const SETTLE_WITHIN: Duration = Duration::from_secs(20);
/// How long a deleted folder is watched afterwards; if anything writes it
/// again, it is deleted again.
const WATCH: Duration = Duration::from_secs(10);

/// What happened to one closed stranger's data.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Wipe {
    pub label: String,
    /// The folder, when this platform uses one.
    pub folder: Option<PathBuf>,
    /// True when the folder is gone and stayed gone, or there was none.
    pub gone: bool,
    pub seconds: f64,
    pub detail: String,
}

pub struct Strangers {
    root: PathBuf,
    next: AtomicU64,
    live: Mutex<HashMap<String, Option<PathBuf>>>,
    wipes: Mutex<Vec<Wipe>>,
    /// Wipes still running.
    pending: AtomicUsize,
}

impl Strangers {
    /// Strangers keep their folders under `root`. Anything already there is
    /// left over from a crash and is deleted now.
    pub fn new(root: PathBuf) -> Strangers {
        if root.exists() {
            let _ = std::fs::remove_dir_all(&root);
        }
        Strangers {
            root,
            next: AtomicU64::new(1),
            live: Mutex::new(HashMap::new()),
            wipes: Mutex::new(Vec::new()),
            pending: AtomicUsize::new(0),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Opens a new stranger at `url` (the start page when None). Returns
    /// its window label.
    pub fn open(&self, app: &AppHandle, url: Option<Url>) -> tauri::Result<String> {
        let n = self.next.fetch_add(1, Ordering::SeqCst);
        let label = format!("{PREFIX}{n}");
        let folder = if USES_FOLDERS {
            Some(self.root.join(format!("{label}-{}", random_hex())))
        } else {
            None
        };
        let target = match url {
            Some(u) => WebviewUrl::External(u),
            None => WebviewUrl::App("index.html".into()),
        };
        let builder = WebviewWindowBuilder::new(app, &label, target)
            .title(format!("Unweb: stranger {n}"))
            .inner_size(1100.0, 760.0);
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        let builder = builder.data_directory(folder.clone().expect("folder platforms"));
        #[cfg(target_os = "macos")]
        let builder = builder.incognito(true);
        // Remember the folder before the engine can write to it, so a
        // failed build is still cleaned up.
        if let Ok(mut live) = self.live.lock() {
            live.insert(label.clone(), folder.clone());
        }
        if let Err(e) = builder.build() {
            self.closed(&label);
            return Err(e);
        }
        Ok(label)
    }

    /// Labels of the strangers still open.
    pub fn open_labels(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .live
            .lock()
            .map(|l| l.keys().cloned().collect())
            .unwrap_or_default();
        v.sort();
        v
    }

    /// The data folder of an open stranger.
    pub fn folder(&self, label: &str) -> Option<PathBuf> {
        self.live
            .lock()
            .ok()
            .and_then(|l| l.get(label).cloned())
            .flatten()
    }

    /// Closes a stranger. On Linux the engine is first told to clear the
    /// stranger's data, so its network process has nothing left to write
    /// into the folder later. Blocks, so call it off the main thread; the
    /// folder itself is deleted when the window is gone (see `closed`).
    pub fn retire(&self, app: &AppHandle, label: &str) {
        if let Some(w) = app.get_webview_window(label) {
            #[cfg(target_os = "linux")]
            clear_engine_data(&w);
            let _ = w.destroy();
        }
    }

    /// A stranger's window has closed: delete its data and check it's gone.
    /// Blocks until done, so call it off the main thread.
    pub fn closed(&self, label: &str) -> Wipe {
        self.pending.fetch_add(1, Ordering::SeqCst);
        let folder = self
            .live
            .lock()
            .ok()
            .and_then(|mut l| l.remove(label))
            .flatten();
        let wipe = match &folder {
            None => Wipe {
                label: label.to_string(),
                folder: None,
                gone: true,
                seconds: 0.0,
                detail: "no folder: the engine kept this stranger in memory only".into(),
            },
            Some(dir) => wipe_folder(label, dir),
        };
        if let Ok(mut w) = self.wipes.lock() {
            w.push(wipe.clone());
        }
        self.pending.fetch_sub(1, Ordering::SeqCst);
        wipe
    }

    /// True while a closed stranger's data is still being deleted.
    pub fn wiping(&self) -> bool {
        self.pending.load(Ordering::SeqCst) > 0
    }

    /// Waits (up to `limit`) for every running wipe to finish.
    pub fn wait_for_wipes(&self, limit: Duration) {
        let until = Instant::now() + limit;
        while self.wiping() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    pub fn wipes(&self) -> Vec<Wipe> {
        self.wipes.lock().map(|w| w.clone()).unwrap_or_default()
    }

    /// At exit: delete whatever is still there, quickly. Anything an engine
    /// process still holds is deleted the next time Unweb starts.
    pub fn wipe_all(&self) {
        if let Ok(mut l) = self.live.lock() {
            l.clear();
        }
        for _ in 0..8 {
            let _ = std::fs::remove_dir_all(&self.root);
            if !self.root.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}

/// Asks WebKitGTK to clear everything it holds for this stranger and waits
/// (up to ten seconds) until it has.
#[cfg(target_os = "linux")]
fn clear_engine_data(w: &tauri::WebviewWindow) -> bool {
    let (tx, rx) = std::sync::mpsc::channel();
    let asked = w.with_webview(move |pv| {
        use webkit2gtk::{WebViewExt, WebsiteDataManagerExtManual, WebsiteDataTypes};
        match pv.inner().website_data_manager() {
            Some(m) => m.clear(
                WebsiteDataTypes::ALL,
                gtk::glib::TimeSpan::from_seconds(0),
                None::<&gtk::gio::Cancellable>,
                move |r| {
                    let _ = tx.send(r.is_ok());
                },
            ),
            None => {
                let _ = tx.send(false);
            }
        }
    });
    asked.is_ok() && rx.recv_timeout(Duration::from_secs(10)).unwrap_or(false)
}

/// A fingerprint of everything in `dir`: each file's path, size and time.
fn snapshot(dir: &Path) -> Vec<(PathBuf, u64, Option<std::time::SystemTime>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            match e.metadata() {
                Ok(m) if m.is_dir() => stack.push(p),
                Ok(m) => out.push((p, m.len(), m.modified().ok())),
                Err(_) => {}
            }
        }
    }
    out.sort();
    out
}

/// Deletes `dir`. First waits until the engine has stopped writing to it,
/// then deletes it (retrying while files are still held open), then
/// watches it for a while and deletes it again if anything brings it back.
fn wipe_folder(label: &str, dir: &Path) -> Wipe {
    let started = Instant::now();
    // 1. Wait for the folder to settle.
    let mut last = snapshot(dir);
    let mut quiet_since = Instant::now();
    while dir.exists() && quiet_since.elapsed() < SETTLE && started.elapsed() < SETTLE_WITHIN {
        std::thread::sleep(Duration::from_millis(250));
        let now = snapshot(dir);
        if now != last {
            last = now;
            quiet_since = Instant::now();
        }
    }
    // 2. Delete, and 3. watch.
    let mut last_error = String::new();
    let mut deletions = 0;
    let mut gone_since: Option<Instant> = None;
    loop {
        if dir.exists() {
            gone_since = None;
            match std::fs::remove_dir_all(dir) {
                Ok(()) => deletions += 1,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => last_error = e.to_string(),
            }
        }
        if !dir.exists() {
            let since = *gone_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= WATCH {
                return Wipe {
                    label: label.to_string(),
                    folder: Some(dir.to_path_buf()),
                    gone: true,
                    seconds: started.elapsed().as_secs_f64(),
                    detail: if deletions > 1 {
                        format!("deleted, written again by the engine and deleted again ({deletions} times), then gone for {}s", WATCH.as_secs())
                    } else {
                        format!(
                            "deleted once the engine stopped writing, and still gone {}s later",
                            WATCH.as_secs()
                        )
                    },
                };
            }
        }
        if started.elapsed() > SETTLE_WITHIN + WIPE_WITHIN + WATCH {
            return Wipe {
                label: label.to_string(),
                folder: Some(dir.to_path_buf()),
                gone: false,
                seconds: started.elapsed().as_secs_f64(),
                detail: format!("still there: {last_error}"),
            };
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn random_hex() -> String {
    let mut b = [0u8; 8];
    getrandom::fill(&mut b).expect("no system randomness");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let mut d = std::env::temp_dir();
        d.push(format!(
            "unweb-test-{name}-{}-{}",
            std::process::id(),
            random_hex()
        ));
        d
    }

    #[test]
    fn leftovers_are_deleted_at_start() {
        let root = temp("leftover");
        std::fs::create_dir_all(root.join("stranger-9-abc/EBWebView")).unwrap();
        std::fs::write(root.join("stranger-9-abc/cookies"), "x").unwrap();
        let s = Strangers::new(root.clone());
        assert!(!root.exists());
        assert!(s.open_labels().is_empty());
    }

    #[test]
    fn wiping_deletes_and_checks() {
        let root = temp("wipe");
        let dir = root.join("stranger-1-x");
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        std::fs::write(dir.join("a/b/c"), "x").unwrap();
        let w = wipe_folder("stranger-1", &dir);
        assert!(w.gone, "{w:?}");
        assert!(!dir.exists());
        // Wiping something already gone is fine.
        assert!(wipe_folder("stranger-1", &dir).gone);
    }

    #[test]
    fn a_closed_stranger_without_folder_is_gone() {
        let s = Strangers::new(temp("nofolder"));
        let w = s.closed("stranger-5");
        assert!(w.gone);
        assert_eq!(s.wipes().len(), 1);
    }
}
