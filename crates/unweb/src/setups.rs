//! Saved setups: a window's strangers, by language and location, kept under
//! a name so the same set can be opened again ("4 strangers in 4
//! countries").
//!
//! A setup holds only what each stranger was opened with: no address, no
//! cookie, nothing a stranger saw. Setups are the one thing Unweb keeps, and
//! only when someone saves one: a small file, `setups.json`, in Unweb's
//! settings folder. Deleting the last setup deletes the file.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::strangers::Settings;

/// The longest name a setup can have, in characters.
pub const MAX_NAME: usize = 60;
/// The most setups kept.
pub const MAX_SETUPS: usize = 50;
/// A setups file larger than this is not read.
const MAX_FILE: u64 = 256 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Setup {
    pub name: String,
    pub strangers: Vec<SetupStranger>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SetupStranger {
    /// A language tag ("" for the system's).
    #[serde(default)]
    pub language: String,
    /// A proxy address ("" to go direct).
    #[serde(default)]
    pub location: String,
}

#[derive(Serialize, Deserialize)]
struct SetupsFile {
    format: String,
    format_version: u32,
    setups: Vec<Setup>,
}

const FORMAT: &str = "unweb-setups";

pub struct Setups {
    path: PathBuf,
    /// One change at a time in this copy of Unweb; `setups.json.lock`
    /// beside the file does the same between copies. Each change reads the
    /// file afresh, so another copy's saves aren't lost.
    lock: Mutex<()>,
}

impl Setups {
    pub fn new(path: PathBuf) -> Setups {
        // A save cut short leaves its new file behind; one older than a
        // minute isn't another copy's save in progress.
        if let Some(dir) = path.parent() {
            if let Ok(rd) = std::fs::read_dir(dir) {
                for e in rd.flatten() {
                    let old = e
                        .metadata()
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok())
                        .is_some_and(|age| age > std::time::Duration::from_secs(60));
                    if old
                        && e.file_name()
                            .to_string_lossy()
                            .starts_with("setups.json.tmp")
                    {
                        let _ = std::fs::remove_file(e.path());
                    }
                }
            }
        }
        Setups {
            path,
            lock: Mutex::new(()),
        }
    }

    /// Where the setups are kept.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Every saved setup, oldest first. A file that can't be read is set
    /// aside (renamed, never deleted) and counts as none.
    pub fn list(&self) -> Vec<Setup> {
        let _held = self.lock.lock();
        self.read()
    }

    /// The saved setups' names, oldest first.
    pub fn names(&self) -> Vec<String> {
        self.list().into_iter().map(|s| s.name).collect()
    }

    /// The strangers of setup `name`. A setup with a location that doesn't
    /// pass the check (a file edited by hand, say) is refused: that stranger
    /// would otherwise go direct.
    pub fn get(&self, name: &str) -> Result<Vec<Settings>, String> {
        let name = clean_name(name)?;
        let setup = self
            .list()
            .into_iter()
            .find(|s| s.name == name)
            .ok_or("There is no setup by that name")?;
        setup
            .strangers
            .iter()
            .enumerate()
            .map(|(i, t)| {
                to_settings(t).map_err(|_| {
                    format!(
                        "Stranger {} of this setup has a location Unweb doesn't take ({}); it won't open it without one",
                        i + 1,
                        t.location
                    )
                })
            })
            .collect()
    }

    /// Saves `strangers` as setup `name`, replacing one of the same name.
    pub fn save(&self, name: &str, strangers: &[Settings]) -> Result<(), String> {
        let name = clean_name(name)?;
        if strangers.is_empty() {
            return Err("This window has no strangers to save".into());
        }
        let _held = self.lock.lock();
        let _file_lock = self.lock_file()?;
        let mut all = self.read();
        let entry = Setup {
            name: name.clone(),
            strangers: strangers
                .iter()
                .take(crate::window::MAX_TILES)
                .map(|s| SetupStranger {
                    language: s.lang.clone(),
                    location: crate::location::show(s.location.as_ref()),
                })
                .collect(),
        };
        let full = all.len() >= MAX_SETUPS;
        match all.iter_mut().find(|s| s.name == name) {
            Some(s) => *s = entry,
            None if full => {
                return Err(format!(
                    "Unweb keeps {MAX_SETUPS} setups at most; delete one first"
                ))
            }
            None => all.push(entry),
        }
        self.write(&all)
    }

    /// Deletes setup `name`. The file goes with the last one.
    pub fn delete(&self, name: &str) -> Result<(), String> {
        let name = clean_name(name)?;
        let _held = self.lock.lock();
        let file_lock = self.lock_file()?;
        let mut all = self.read();
        let before = all.len();
        all.retain(|s| s.name != name);
        if all.len() == before {
            return Err("There is no setup by that name".into());
        }
        self.write(&all)?;
        if all.is_empty() {
            // The last one: the lock file and an empty folder go too.
            drop(file_lock);
            let _ = std::fs::remove_file(self.path.with_file_name("setups.json.lock"));
            if let Some(dir) = self.path.parent() {
                let _ = std::fs::remove_dir(dir);
            }
        }
        Ok(())
    }

    /// Holds `setups.json.lock` while a change is made, so two copies of
    /// Unweb don't write at once. Made only when a change is.
    fn lock_file(&self) -> Result<std::fs::File, String> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("Couldn't save the setup: {e}"))?;
        }
        let f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.path.with_file_name("setups.json.lock"))
            .map_err(|e| format!("Couldn't save the setup: {e}"))?;
        f.lock()
            .map_err(|e| format!("Couldn't save the setup: {e}"))?;
        Ok(f)
    }

    fn read(&self) -> Vec<Setup> {
        let Ok(meta) = std::fs::metadata(&self.path) else {
            return Vec::new();
        };
        let parsed = (meta.len() <= MAX_FILE)
            .then(|| std::fs::read(&self.path).ok())
            .flatten()
            .and_then(|b| serde_json::from_slice::<SetupsFile>(&b).ok())
            .filter(|f| f.format == FORMAT && f.format_version == 1);
        match parsed {
            Some(f) => f
                .setups
                .into_iter()
                .filter_map(|s| {
                    let name = clean_name(&s.name).ok()?;
                    let strangers: Vec<SetupStranger> = s
                        .strangers
                        .into_iter()
                        .take(crate::window::MAX_TILES)
                        .map(|t| SetupStranger {
                            language: crate::window::known_language(&t.language)
                                .unwrap_or("")
                                .to_string(),
                            // A location that doesn't pass is kept as it is, so
                            // opening the setup can refuse it rather than go direct.
                            location: match crate::location::parse_location(&t.location) {
                                Ok(l) => crate::location::show(l.as_ref()),
                                Err(_) => t.location.trim().chars().take(300).collect(),
                            },
                        })
                        .collect();
                    (!strangers.is_empty()).then_some(Setup { name, strangers })
                })
                .take(MAX_SETUPS)
                .collect(),
            None => {
                // Kept for whoever wants to look at it; Unweb starts over.
                let secs = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or_default();
                let aside = self.path.with_file_name(format!("setups.json.bad-{secs}"));
                let _ = std::fs::rename(&self.path, aside);
                Vec::new()
            }
        }
    }

    /// Writes all setups, all at once: a new file first, then renamed over
    /// the old one, so a crash leaves the old or the new, never half.
    fn write(&self, all: &[Setup]) -> Result<(), String> {
        if all.is_empty() {
            match std::fs::remove_file(&self.path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("Couldn't delete the setups file: {e}")),
            }
            return Ok(());
        }
        let file = SetupsFile {
            format: FORMAT.into(),
            format_version: 1,
            setups: all.to_vec(),
        };
        let json = serde_json::to_vec_pretty(&file).map_err(|e| e.to_string())?;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("Couldn't save the setup: {e}"))?;
        }
        let mut b = [0u8; 6];
        let _ = getrandom::fill(&mut b);
        let tmp = self.path.with_file_name(format!(
            "setups.json.tmp-{}-{}",
            std::process::id(),
            b.iter().map(|x| format!("{x:02x}")).collect::<String>()
        ));
        let written = (|| {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&json)?;
            f.sync_all()?;
            std::fs::rename(&tmp, &self.path)
        })();
        if let Err(e) = written {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("Couldn't save the setup: {e}"));
        }
        Ok(())
    }
}

/// A setup's stranger as the settings it opens with. A language this
/// version doesn't offer is left out; a location that doesn't pass the
/// check is an error, never "direct".
fn to_settings(t: &SetupStranger) -> Result<Settings, String> {
    Ok(Settings {
        lang: crate::window::known_language(&t.language)
            .unwrap_or("")
            .to_string(),
        location: crate::location::parse_location(&t.location)?,
    })
}

/// A setup's name: trimmed, 1 to 60 characters, no control characters.
pub fn clean_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Give the setup a name".into());
    }
    if name.chars().count() > MAX_NAME {
        return Err(format!(
            "A setup's name can be {MAX_NAME} characters at most"
        ));
    }
    if name.chars().any(|c| c.is_control()) {
        return Err("A setup's name can't have control characters".into());
    }
    Ok(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        let mut b = [0u8; 8];
        getrandom::fill(&mut b).unwrap();
        let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
        std::env::temp_dir()
            .join(format!("unweb-setups-{}-{hex}", std::process::id()))
            .join("setups.json")
    }

    fn s(lang: &str, loc: &str) -> Settings {
        Settings {
            lang: lang.into(),
            location: crate::location::parse_location(loc).unwrap(),
        }
    }

    #[test]
    fn save_open_delete() {
        let path = temp();
        let setups = Setups::new(path.clone());
        assert!(setups.names().is_empty());
        let four = [
            s("de-DE", "socks5://de.proxy.example:1080"),
            s("fr-FR", "fr.proxy.example:8080"),
            s("", ""),
            s("ja-JP", ""),
        ];
        setups.save("  Four countries ", &four).unwrap();
        assert_eq!(setups.names(), vec!["Four countries"]);
        assert_eq!(setups.get("Four countries").unwrap(), four.to_vec());
        assert!(setups.get("No such").is_err());
        // The file holds the settings and nothing else.
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"format\": \"unweb-setups\""));
        assert!(text.contains("socks5://de.proxy.example:1080"));
        assert!(text.contains("http://fr.proxy.example:8080"));
        // Same name replaces; another name adds.
        setups.save("Four countries", &four[..2]).unwrap();
        setups.save("Plain", &[s("", "")]).unwrap();
        assert_eq!(setups.names(), vec!["Four countries", "Plain"]);
        assert_eq!(setups.get("Four countries").unwrap().len(), 2);
        // Deleting the last one deletes the file and its folder.
        setups.delete("Four countries").unwrap();
        assert!(setups.delete("Four countries").is_err());
        setups.delete("Plain").unwrap();
        assert!(!path.exists());
        assert!(!path.parent().unwrap().exists());
    }

    #[test]
    fn names_are_checked() {
        assert_eq!(clean_name(" a ").unwrap(), "a");
        assert!(clean_name("  ").is_err());
        assert!(clean_name(&"x".repeat(61)).is_err());
        assert!(clean_name(&"é".repeat(60)).is_ok());
        assert!(clean_name("a\nb").is_err());
        assert!(clean_name("a\u{85}b").is_err());
        let setups = Setups::new(temp());
        assert!(setups.save("x", &[]).is_err());
        for i in 0..MAX_SETUPS {
            setups.save(&format!("n{i}"), &[s("", "")]).unwrap();
        }
        assert!(setups.save("one more", &[s("", "")]).is_err());
        // Replacing still works when full.
        setups.save("n0", &[s("de-DE", "")]).unwrap();
        for i in 0..MAX_SETUPS {
            setups.delete(&format!("n{i}")).unwrap();
        }
        assert!(!setups.path().exists());
    }

    #[test]
    fn a_damaged_or_odd_file_is_set_aside_or_cleaned() {
        let path = temp();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{ not json").unwrap();
        let setups = Setups::new(path.clone());
        assert!(setups.names().is_empty());
        assert!(!path.exists());
        let aside: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("setups.json.bad-")
            })
            .collect();
        assert_eq!(aside.len(), 1);
        // Hand-edited entries that don't pass are cleaned up on reading:
        // a made-up language, a proxy with a password, a bad name.
        std::fs::write(
            &path,
            r#"{"format":"unweb-setups","format_version":1,"setups":[
              {"name":"ok","strangers":[{"language":"xx-YY --x","location":"http://u:p@h:1"},{"language":"de-DE","location":"h:8080"},{},{},{},{}]},
              {"name":"bad\u0007","strangers":[{}]},
              {"name":"empty","strangers":[]}]}"#,
        )
        .unwrap();
        let list = setups.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].strangers.len(), crate::window::MAX_TILES);
        // The made-up language is dropped; the proxy with a password stays
        // as written, and opening the setup is refused rather than going
        // direct.
        assert_eq!(list[0].strangers[0].language, "");
        assert_eq!(list[0].strangers[0].location, "http://u:p@h:1");
        assert_eq!(list[0].strangers[1].location, "http://h:8080");
        let refused = setups.get("ok").unwrap_err();
        assert!(refused.contains("Stranger 1"), "{refused}");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
