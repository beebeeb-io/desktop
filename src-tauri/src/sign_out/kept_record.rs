//! Spec 2026-10-10 W9, W13 and §9 (task 1887): the kept folder's two keys in `desktop.toml`. Paths are never logged.
#![cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]

use std::path::Path;

use crate::config::{ConfigFs, DesktopConfig};
use crate::finder_removal::DismissOutcome;

/// The keep step's record: the row's folder, and the folder added to the unfinished list (unless it is there).
/// Returns the `kept_unsent_folder` it replaced.
pub(crate) fn record_kept_unsent(cfg: &mut DesktopConfig, folder: &str) -> Option<String> {
    if !cfg.kept_unsent_unfinished.iter().any(|known| known == folder) {
        cfg.kept_unsent_unfinished.push(folder.to_string());
    }
    cfg.kept_unsent_folder.replace(folder.to_string())
}

/// A failed keep step after its save (W8): no key may name the folder it is about to remove.
pub(crate) fn take_back_kept_unsent(cfg: &mut DesktopConfig, folder: &str, previous: Option<&str>) -> bool {
    let before = (cfg.kept_unsent_folder.clone(), cfg.kept_unsent_unfinished.len());
    cfg.kept_unsent_unfinished.retain(|known| known != folder);
    if cfg.kept_unsent_folder.as_deref() == Some(folder) {
        cfg.kept_unsent_folder = previous.map(str::to_string);
    }
    before != (cfg.kept_unsent_folder.clone(), cfg.kept_unsent_unfinished.len())
}

/// The row's "Dismiss": clears only the folder the row showed (1882's rule, `finder_removal::dismiss_kept_folder`).
pub(crate) fn dismiss_kept_unsent(cfg: &mut DesktopConfig, shown: &str) -> bool {
    if cfg.kept_unsent_folder.as_deref() != Some(shown) {
        return false;
    }
    cfg.kept_unsent_folder = None;
    true
}

/// W9 and W13: both keys in ONE durable save, the keep step's last act.
#[cfg_attr(not(test), allow(dead_code))] // first production caller: Task 5
pub(crate) fn save_kept_unsent_durably(
    path: &Path,
    folder: &str,
    fs_steps: &dyn ConfigFs,
) -> Result<Option<String>, String> {
    DesktopConfig::update_at_durable_with(path, |cfg| (true, record_kept_unsent(cfg, folder)), fs_steps)
}

#[cfg_attr(not(test), allow(dead_code))] // first production caller: Task 5
pub(crate) fn take_back_kept_unsent_at(path: &Path, folder: &str, previous: Option<&str>) -> Result<(), String> {
    DesktopConfig::update_at(path, |cfg| (take_back_kept_unsent(cfg, folder, previous), ()))
}

/// W13: a purge that cleared the rows clears the list (plain, not durable: a lost clear only repeats A3e once).
#[cfg_attr(not(test), allow(dead_code))] // first production caller: Task 6
pub(crate) fn clear_unfinished_at(path: &Path) -> Result<(), String> {
    DesktopConfig::update_at(path, |cfg| {
        let changed = !cfg.kept_unsent_unfinished.is_empty();
        cfg.kept_unsent_unfinished.clear();
        (changed, ())
    })
}

#[cfg_attr(not(test), allow(dead_code))] // first production caller: Task 10
pub(crate) fn dismiss_saved_kept_unsent_at(path: &Path, shown: &str) -> Result<DismissOutcome, String> {
    DesktopConfig::update_at(path, |cfg| {
        let cleared = dismiss_kept_unsent(cfg, shown);
        (
            cleared,
            DismissOutcome {
                cleared,
                current: cfg.kept_unsent_folder.clone(),
            },
        )
    })
}

#[cfg_attr(not(test), allow(dead_code))] // first production caller: Task 5
pub(crate) fn saved_kept_unsent_at(path: &Path) -> Result<Option<String>, String> {
    Ok(DesktopConfig::load_from(path)?.kept_unsent_folder)
}

#[cfg_attr(not(test), allow(dead_code))] // first production caller: Task 7
pub(crate) fn unfinished_at(path: &Path) -> Result<Vec<String>, String> {
    Ok(DesktopConfig::load_from(path)?.kept_unsent_unfinished)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ConfigFs, DesktopConfig, DesktopSettings, RealConfigFs};
    use crate::source_pin::squeeze;
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    fn config_file() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("desktop.toml");
        (dir, path)
    }

    // The real durable steps flush a file opened read-only and then a directory. Windows can do neither through
    // std (FlushFileBuffers needs a handle with write access; a directory opens only with backup semantics), and
    // the durable save runs on macOS only (W5). The recording test below runs everywhere.
    /// U22.
    #[cfg_attr(windows, ignore = "the real durable steps need unix flushes; the save is macOS only")]
    #[test]
    fn kept_unsent_folder_follows_1882s_rules_and_its_own_key() {
        let (_dir, path) = config_file();
        crate::finder_removal::remember_kept_folder_at(&path, "/macos/kept").unwrap();
        save_kept_unsent_durably(&path, "/ours/one", &RealConfigFs).unwrap();
        assert_eq!(
            save_kept_unsent_durably(&path, "/ours/two", &RealConfigFs)
                .unwrap()
                .as_deref(),
            Some("/ours/one")
        );
        assert_eq!(
            saved_kept_unsent_at(&path).unwrap().as_deref(),
            Some("/ours/two"),
            "a newer folder replaces an older one"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "the durable write is owner-only: {mode:o}");
        }
        // A settings save cannot clear it. A window sends only the fields that changed (task 1889), and a settings
        // object built from the defaults stands for one that was read before the folder was saved.
        let patch: DesktopSettings = serde_json::from_str(r#"{"upload_kbps_limit":2000}"#).unwrap();
        DesktopConfig::update_at(&path, |cfg| {
            cfg.apply_settings(patch);
            (true, ())
        })
        .unwrap();
        assert_eq!(
            saved_kept_unsent_at(&path).unwrap().as_deref(),
            Some("/ours/two"),
            "a settings save of the one field that changed"
        );
        let defaults = DesktopSettings::from(&DesktopConfig::default());
        DesktopConfig::update_at(&path, |cfg| {
            cfg.apply_settings(defaults);
            (true, ())
        })
        .unwrap();
        assert_eq!(
            saved_kept_unsent_at(&path).unwrap().as_deref(),
            Some("/ours/two"),
            "a whole settings object built from the defaults"
        );
        // The two keys never touch each other: our saves left 1882's key as it was, and 1882's save, made while our row
        // is set, leaves our row and our list.
        let cfg = DesktopConfig::load_from(&path).unwrap();
        assert_eq!(cfg.kept_unsynced_folder.as_deref(), Some("/macos/kept"));
        crate::finder_removal::remember_kept_folder_at(&path, "/macos/newer").unwrap();
        assert_eq!(
            saved_kept_unsent_at(&path).unwrap().as_deref(),
            Some("/ours/two"),
            "1882's save leaves our row alone"
        );
        assert_eq!(
            unfinished_at(&path).unwrap(),
            ["/ours/one", "/ours/two"],
            "1882's save leaves our list alone"
        );
        // Dismiss clears only the folder the row showed. It leaves the list and 1882's key.
        let stale = dismiss_saved_kept_unsent_at(&path, "/ours/one").unwrap();
        assert_eq!((stale.cleared, stale.current.as_deref()), (false, Some("/ours/two")));
        let shown = dismiss_saved_kept_unsent_at(&path, "/ours/two").unwrap();
        assert_eq!((shown.cleared, shown.current), (true, None));
        let cfg = DesktopConfig::load_from(&path).unwrap();
        assert_eq!(
            cfg.kept_unsent_unfinished,
            ["/ours/one", "/ours/two"],
            "a dismiss leaves the list"
        );
        assert_eq!(cfg.kept_unsynced_folder.as_deref(), Some("/macos/newer"));
    }

    #[derive(Default)]
    struct Recorded {
        steps: RefCell<Vec<String>>,
        fail_at: Option<&'static str>,
    }

    impl Recorded {
        fn step(&self, name: &str, path: &Path) -> std::io::Result<()> {
            self.steps
                .borrow_mut()
                .push(format!("{name} {}", path.file_name().unwrap().to_string_lossy()));
            if self.fail_at == Some(name) {
                return Err(std::io::Error::other("injected"));
            }
            Ok(())
        }
    }

    impl ConfigFs for Recorded {
        fn write(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
            self.step("write", path)?;
            RealConfigFs.write(path, bytes)
        }
        fn sync_file(&self, path: &Path) -> std::io::Result<()> {
            self.step("sync_file", path)
        }
        fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
            self.step("rename", to)?;
            RealConfigFs.rename(from, to)
        }
        fn sync_dir(&self, dir: &Path) -> std::io::Result<()> {
            self.step("sync_dir", dir)
        }
    }

    /// U43, the save's half (the keep step's tests cover its take-back and its folder removal).
    #[test]
    fn the_marker_save_is_one_durable_write() {
        let (dir, path) = config_file();
        crate::finder_removal::remember_kept_folder_at(&path, "/macos/kept").unwrap();
        let fs = Recorded::default();
        save_kept_unsent_durably(&path, "/ours/one", &fs).unwrap();
        let dir_name = dir.path().file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(
            *fs.steps.borrow(),
            [
                "write desktop.toml.tmp".to_string(),
                "sync_file desktop.toml.tmp".to_string(),
                "rename desktop.toml".to_string(),
                format!("sync_dir {dir_name}"),
            ],
            "one write, one flush, ONE rename for both keys, then the directory"
        );
        let cfg = DesktopConfig::load_from(&path).unwrap();
        assert_eq!(cfg.kept_unsent_folder.as_deref(), Some("/ours/one"));
        assert_eq!(cfg.kept_unsent_unfinished, ["/ours/one"]);
        assert_eq!(cfg.kept_unsynced_folder.as_deref(), Some("/macos/kept"));

        let before_rename = Recorded {
            fail_at: Some("sync_file"),
            ..Default::default()
        };
        assert!(save_kept_unsent_durably(&path, "/ours/two", &before_rename).is_err());
        assert_eq!(
            unfinished_at(&path).unwrap(),
            ["/ours/one"],
            "a failure before the rename changes nothing"
        );

        let after_rename = Recorded {
            fail_at: Some("sync_dir"),
            ..Default::default()
        };
        assert!(save_kept_unsent_durably(&path, "/ours/three", &after_rename).is_err());
        // A failure after the rename returns an error with both keys already in the file, for the caller to take back.
        let cfg = DesktopConfig::load_from(&path).unwrap();
        assert_eq!(
            cfg.kept_unsent_folder.as_deref(),
            Some("/ours/three"),
            "the row is in the file"
        );
        assert_eq!(
            cfg.kept_unsent_unfinished,
            ["/ours/one", "/ours/three"],
            "and so is the list"
        );
        take_back_kept_unsent_at(&path, "/ours/three", Some("/ours/one")).unwrap();
        let cfg = DesktopConfig::load_from(&path).unwrap();
        assert_eq!(
            cfg.kept_unsent_unfinished,
            ["/ours/one"],
            "the take-back leaves no key naming the folder"
        );
        assert_eq!(
            cfg.kept_unsent_folder.as_deref(),
            Some("/ours/one"),
            "and puts the older row back"
        );

        // A take-back that failed leaves its folder listed, and a retry in the same second can keep into a folder of
        // the same name: the list names it once (spec §7.3, "unless it is already there").
        let after_rename = Recorded {
            fail_at: Some("sync_dir"),
            ..Default::default()
        };
        assert!(save_kept_unsent_durably(&path, "/ours/four", &after_rename).is_err());
        save_kept_unsent_durably(&path, "/ours/four", &Recorded::default()).unwrap();
        assert_eq!(
            unfinished_at(&path).unwrap(),
            ["/ours/one", "/ours/four"],
            "a folder already listed is listed once"
        );

        // Every other save stays as it was: no flush in `save_to` (spec §7.3, amended for 1889). Line ends are read as
        // LF, because a Windows checkout has CRLF.
        let config_rs = include_str!("../config.rs").replace("\r\n", "\n");
        let from = config_rs.find("    fn save_to(&self, path: &Path)").expect("save_to");
        let save_to = &config_rs[from..from + config_rs[from..].find("\n    }\n").unwrap()];
        assert!(!save_to.contains("sync_all"), "{save_to}");
        // Nothing else picks the durable write: the two plain ways in pass no write step, and `config.rs` names the
        // real steps only in their struct and its impl.
        let production = production_text(&config_rs);
        for (start, plain) in [
            (
                "    pub(crate) fn try_update_at<R>(",
                "Self::try_update_at_then(path, change, |_| {}, None)",
            ),
            (
                "    pub(crate) fn update_at_then<R>(",
                "Self::try_update_at_then(path, change, saved, None)",
            ),
        ] {
            let body = block(&production, start, 4);
            assert!(
                squeeze(body).contains(&squeeze(plain)),
                "{start} passes no write step:\n{body}"
            );
        }
        assert_eq!(
            production.matches("RealConfigFs").count(),
            2,
            "config.rs names the real steps only in their struct and its impl"
        );
        // The real steps are the durable ones. The recording seam above never runs them, so their text is pinned: both
        // flushes are made, and the write is owner-only.
        let real = block(&production, "impl ConfigFs for RealConfigFs {", 0);
        for (step, must) in [
            ("    fn sync_file(", ".sync_all()"),
            ("    fn sync_dir(", ".sync_all()"),
            ("    fn write(", "from_mode(0o600)"),
        ] {
            let body = block(real, step, 4);
            assert!(body.contains(must), "{step} must contain {must}:\n{body}");
        }
    }

    /// `text` as production code: the part before its test module, without the lines that are comments.
    fn production_text(text: &str) -> String {
        let end = text.find("#[cfg(test)]\nmod tests").expect("the test module");
        text[..end]
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The text from `start` up to the line that closes it at `indent` spaces.
    fn block<'a>(text: &'a str, start: &str, indent: usize) -> &'a str {
        let at = text.find(start).unwrap_or_else(|| panic!("{start} exists"));
        let end = format!("\n{}}}", " ".repeat(indent));
        let to = text[at..].find(&end).unwrap_or_else(|| panic!("{start} ends"));
        &text[at..at + to]
    }

    #[cfg_attr(windows, ignore = "the real durable steps need unix flushes; the save is macOS only")]
    #[test]
    fn a_completed_purge_clears_the_unfinished_list_and_nothing_else() {
        let (_dir, path) = config_file();
        save_kept_unsent_durably(&path, "/ours/one", &RealConfigFs).unwrap();
        save_kept_unsent_durably(&path, "/ours/two", &RealConfigFs).unwrap();
        clear_unfinished_at(&path).unwrap();
        let cfg = DesktopConfig::load_from(&path).unwrap();
        assert!(cfg.kept_unsent_unfinished.is_empty());
        assert_eq!(cfg.kept_unsent_folder.as_deref(), Some("/ours/two"), "the row stays");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            !text.contains("kept_unsent_unfinished"),
            "absent from the file while empty: {text}"
        );
    }
}
