//! Strangers: browser windows that each get a throwaway store of their own.
//!
//! On Windows and Linux every stranger gets its own data folder under
//! `<app data>/strangers/run-<random>/`, which the engine uses for cookies, storage,
//! cache, service workers and everything else. Two strangers never share a
//! folder, so they never share anything. When a stranger's window closes,
//! its folder is deleted, and the deletion is checked: the folder must be
//! gone and stay gone. On macOS each stranger gets a non-persistent
//! WKWebsiteDataStore of its own, which keeps nothing on disk to delete.
//!
//! Each running copy of Unweb has a run folder of its own and holds a lock
//! on `run-<random>.lock` next to it while it runs. At start, Unweb deletes
//! every run folder whose lock nobody holds (left behind by a crash or a
//! kill) and leaves the folders of copies still running alone.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager, Url, WebviewUrl};

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
    /// `<app data>/strangers`: every run's folder and lock file.
    base: PathBuf,
    /// This run's folder.
    root: PathBuf,
    lock_path: PathBuf,
    /// Held while this copy of Unweb runs.
    lock: Mutex<Option<std::fs::File>>,
    next: AtomicU64,
    live: Mutex<HashMap<String, Option<PathBuf>>>,
    wipes: Mutex<Vec<Wipe>>,
    /// Wipes still running.
    pending: AtomicUsize,
}

impl Strangers {
    /// Strangers keep their folders in a new run folder under `base`. Run
    /// folders nobody holds a lock on are left over from a crash and are
    /// deleted now.
    pub fn new(base: PathBuf) -> Strangers {
        purge_abandoned(&base);
        let (root, lock_path, lock) = take_run(&base);
        Strangers {
            base,
            root,
            lock_path,
            lock: Mutex::new(lock),
            next: AtomicU64::new(1),
            live: Mutex::new(HashMap::new()),
            wipes: Mutex::new(Vec::new()),
            pending: AtomicUsize::new(0),
        }
    }

    /// The folder that holds every run's folder and lock file.
    pub fn base(&self) -> &Path {
        &self.base
    }

    /// True when `path` (anything under the strangers folder) belongs to
    /// another copy of Unweb that is running now, whose lock is held.
    pub fn belongs_to_other_live_run(&self, path: &Path) -> bool {
        let Ok(rel) = path.strip_prefix(&self.base) else {
            return false;
        };
        let Some(first) = rel.components().next() else {
            return false;
        };
        let name = first.as_os_str().to_string_lossy();
        let run = name
            .strip_suffix(".lock.new")
            .or_else(|| name.strip_suffix(".lock"))
            .unwrap_or(&name);
        let lock = self.base.join(format!("{run}.lock"));
        run.starts_with("run-") && lock != self.lock_path && lock.exists() && !lock_is_free(&lock)
    }

    /// This run's lock file.
    pub fn lock_path(&self) -> &Path {
        &self.lock_path
    }

    /// This run's folder.
    #[cfg(test)]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Opens a new stranger at `url` (the start page when None). Returns
    /// its window label. The window holds two webviews: the bar (address,
    /// back, forward, reload, new stranger) and the page. Both use the
    /// stranger's own store; only the bar is granted app commands.
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
        // Remember the folder before the engine can write to it, so a
        // failed build is still cleaned up.
        if let Ok(mut live) = self.live.lock() {
            live.insert(label.clone(), folder.clone());
        }
        match crate::window::build(app, &label, n, folder.as_deref(), target) {
            Ok(()) => Ok(label),
            Err(e) => {
                if let Some(w) = app.get_window(&label) {
                    let _ = w.destroy();
                }
                self.closed(&label);
                Err(e)
            }
        }
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
        #[cfg(target_os = "linux")]
        if let Some(page) = app.get_webview(&crate::window::page_label(label)) {
            clear_engine_data(&page);
        }
        // WKWebView keeps a stranger's service worker running for a few
        // seconds after its window is gone. Clearing the store removes the
        // worker; leaving the page first stops what it is doing.
        #[cfg(target_os = "macos")]
        if let Some(page) = app.get_webview(&crate::window::page_label(label)) {
            if let Ok(blank) = Url::parse("about:blank") {
                let _ = page.navigate(blank);
            }
            clear_store(&page);
        }
        if let Some(w) = app.get_window(label) {
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

    /// At exit: delete this run's folder, quickly, then let go of the lock.
    /// Anything an engine process still holds is deleted the next time
    /// Unweb starts, because the lock is free by then.
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
        let held = self.lock.lock().ok().and_then(|mut l| l.take());
        drop(held);
        if !self.root.exists() {
            let _ = std::fs::remove_file(&self.lock_path);
        }
        // The strangers folder itself goes when no other run is using it.
        let _ = std::fs::remove_dir(&self.base);
    }
}

/// Makes this run's lock: the file is created and locked under a name no
/// purge touches while it's young, then renamed into place, so another
/// copy starting at the same moment never sees an unlocked run lock.
fn take_run(base: &Path) -> (PathBuf, PathBuf, Option<std::fs::File>) {
    let _ = std::fs::create_dir_all(base);
    let mut last = None;
    for _ in 0..5 {
        let id = random_hex();
        let root = base.join(format!("run-{id}"));
        let lock_path = base.join(format!("run-{id}.lock"));
        let fresh = base.join(format!("run-{id}.lock.new"));
        let Ok(f) = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&fresh)
        else {
            continue;
        };
        // Another copy's probe can hold the lock for an instant.
        let mut locked = false;
        for _ in 0..20 {
            if f.try_lock().is_ok() {
                locked = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if locked && std::fs::rename(&fresh, &lock_path).is_ok() {
            return (root, lock_path, Some(f));
        }
        drop(f);
        let _ = std::fs::remove_file(&fresh);
        last = Some((root, lock_path));
    }
    let (root, lock_path) = last.unwrap_or_else(|| {
        let id = random_hex();
        (
            base.join(format!("run-{id}")),
            base.join(format!("run-{id}.lock")),
        )
    });
    (root, lock_path, None)
}

/// A lock file made in the last few seconds may belong to a copy that is
/// still starting; purges leave it alone.
fn young(path: &Path) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age < Duration::from_secs(10))
}

/// Deletes every run folder under `base` whose lock nobody holds, and
/// anything else there that isn't a run folder or a lock in use.
fn purge_abandoned(base: &Path) {
    let Ok(rd) = std::fs::read_dir(base) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if name.ends_with(".lock.new") {
            // A lock being made: only an old, free one is left over.
            if !young(&p) && lock_is_free(&p) {
                let _ = std::fs::remove_file(&p);
            }
            continue;
        }
        if let Some(run) = name.strip_suffix(".lock") {
            // A lock whose folder is gone: remove it if it's free.
            if !base.join(run).exists() && !young(&p) && lock_is_free(&p) {
                let _ = std::fs::remove_file(&p);
            }
            continue;
        }
        let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let lock = base.join(format!("{name}.lock"));
        if name.starts_with("run-")
            && is_dir
            && lock.exists()
            && (young(&lock) || !lock_is_free(&lock))
        {
            continue; // another copy of Unweb is using it
        }
        if is_dir {
            let _ = std::fs::remove_dir_all(&p);
        } else {
            let _ = std::fs::remove_file(&p);
        }
        if lock.exists() && !p.exists() && lock_is_free(&lock) {
            let _ = std::fs::remove_file(&lock);
        }
    }
}

/// True when nobody holds the lock on `path`.
fn lock_is_free(path: &Path) -> bool {
    match std::fs::OpenOptions::new().write(true).open(path) {
        Ok(f) => f.try_lock().is_ok(),
        Err(_) => false,
    }
}

/// Asks WebKitGTK to clear everything it holds for this stranger and waits
/// (up to ten seconds) until it has.
#[cfg(target_os = "linux")]
fn clear_engine_data(w: &tauri::Webview) -> bool {
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

/// Asks WKWebView to remove everything in this stranger's store (which
/// also ends its service workers) and waits (up to ten seconds) until
/// WebKit says it is done.
#[cfg(target_os = "macos")]
fn clear_store(page: &tauri::Webview) -> bool {
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let asked = page.with_webview(move |pv| unsafe {
        let Some(mtm) = objc2::MainThreadMarker::new() else {
            return;
        };
        let ptr = pv.inner() as *const objc2_web_kit::WKWebView;
        let Some(wv) = ptr.as_ref() else {
            return;
        };
        let store = wv.configuration().websiteDataStore();
        let types = objc2_web_kit::WKWebsiteDataStore::allWebsiteDataTypes(mtm);
        let since = objc2_foundation::NSDate::dateWithTimeIntervalSince1970(0.0);
        let done = block2::RcBlock::new(move || {
            let _ = tx.send(());
        });
        store.removeDataOfTypes_modifiedSince_completionHandler(&types, &since, &done);
    });
    asked.is_ok() && rx.recv_timeout(Duration::from_secs(10)).is_ok()
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

    /// Makes a file look a minute old.
    fn age(p: &Path) {
        let f = std::fs::OpenOptions::new().write(true).open(p).unwrap();
        f.set_modified(std::time::SystemTime::now() - Duration::from_secs(60))
            .unwrap();
    }

    #[test]
    fn leftovers_are_deleted_at_start() {
        let base = temp("leftover");
        // An old-style folder, an abandoned run with a free lock and one
        // with no lock at all.
        std::fs::create_dir_all(base.join("stranger-9-abc/EBWebView")).unwrap();
        std::fs::write(base.join("stranger-9-abc/cookies"), "x").unwrap();
        std::fs::create_dir_all(base.join("run-dead/stranger-1-x")).unwrap();
        std::fs::write(base.join("run-dead.lock"), "").unwrap();
        age(&base.join("run-dead.lock"));
        // A lock still being made by a copy that is starting stays.
        std::fs::write(base.join("run-new.lock.new"), "").unwrap();
        std::fs::create_dir_all(base.join("run-nolock/stranger-1-x")).unwrap();
        let s = Strangers::new(base.clone());
        assert!(!base.join("stranger-9-abc").exists());
        assert!(!base.join("run-dead").exists());
        assert!(!base.join("run-dead.lock").exists());
        assert!(!base.join("run-nolock").exists());
        assert!(base.join("run-new.lock.new").exists());
        assert!(s.open_labels().is_empty());
        s.wipe_all();
        std::fs::remove_file(base.join("run-new.lock.new")).unwrap();
        let _ = std::fs::remove_dir(&base);
        assert!(!base.exists());
    }

    #[test]
    fn a_running_copy_keeps_its_folder() {
        let base = temp("twocopies");
        let first = Strangers::new(base.clone());
        std::fs::create_dir_all(first.root().join("stranger-1-x")).unwrap();
        std::fs::write(first.root().join("stranger-1-x/cookies"), "x").unwrap();
        // A second copy starts while the first runs.
        let second = Strangers::new(base.clone());
        assert!(first.root().join("stranger-1-x/cookies").exists());
        assert_ne!(first.root(), second.root());
        // The second quits; the first's folder stays.
        second.wipe_all();
        assert!(first.root().join("stranger-1-x/cookies").exists());
        first.wipe_all();
        assert!(!base.exists());
    }

    #[test]
    fn another_live_copy_is_recognised() {
        let base = temp("liveother");
        let first = Strangers::new(base.clone());
        let second = Strangers::new(base.clone());
        std::fs::create_dir_all(second.root().join("stranger-1-x")).unwrap();
        assert!(first.belongs_to_other_live_run(second.lock_path()));
        assert!(first.belongs_to_other_live_run(&second.root().join("stranger-1-x")));
        assert!(!first.belongs_to_other_live_run(first.lock_path()));
        second.wipe_all();
        drop(second);
        assert!(!first.belongs_to_other_live_run(&base.join("run-gone.lock")));
        first.wipe_all();
    }

    #[test]
    fn a_run_left_by_a_crash_goes_at_next_start() {
        let base = temp("crash");
        let crashed = Strangers::new(base.clone());
        std::fs::create_dir_all(crashed.root().join("stranger-1-x")).unwrap();
        let left = crashed.root().to_path_buf();
        let lock = crashed.lock_path().to_path_buf();
        // A crash: the lock is let go without wiping.
        drop(crashed);
        age(&lock);
        let next = Strangers::new(base.clone());
        assert!(!left.exists());
        next.wipe_all();
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
