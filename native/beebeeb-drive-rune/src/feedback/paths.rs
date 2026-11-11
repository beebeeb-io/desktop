//! Where notes go. Precedence: `--feedback-dir <path>` (a development flag: read by
//! `shell::instance::parse_args` under the rule of the shell's other tooling flags, so a build
//! that is not a dev build ignores it), then `BEEBEEB_FEEDBACK_DIR`, then
//! `%LOCALAPPDATA%\<app id>\feedback\inbox` on Windows (elsewhere: that id's data folder).
//!
//! The app id is the shell's (`shell::identity::AppIdentity::feedback_dir_id`): the TEST
//! identity's, whatever identity the build has. Never the real app's data folder.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::shell::identity::AppIdentity;

pub const DIR_ENV: &str = "BEEBEEB_FEEDBACK_DIR";

/// What `--feedback-dir` said, as the shell parsed it. Set once, before the first state.
static FLAG_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();

/// The shell hands over the launch's `--feedback-dir` (`None` in a build that is not a dev
/// build, and on a command line that carries a link). The first call wins.
pub fn set_flag_dir(dir: Option<PathBuf>) {
    let _ = FLAG_DIR.set(dir);
}

/// The inbox for this flag, identity and environment. `None` only when no home folder exists.
pub fn inbox_from(
    flag: Option<&Path>,
    identity: AppIdentity,
    var: impl Fn(&str) -> Option<OsString>,
) -> Option<PathBuf> {
    if let Some(dir) = flag {
        return Some(dir.to_path_buf());
    }
    if let Some(dir) = var(DIR_ENV).filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    let id = identity.feedback_dir_id();
    let base = if cfg!(target_os = "windows") {
        var("LOCALAPPDATA")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)?
            .join(id)
    } else {
        rune::dirs::AppDirs::from_env(id, &var)?.data().to_path_buf()
    };
    Some(base.join("feedback").join("inbox"))
}

/// This process's inbox. A test never looks at a real folder: it names its own
/// (`Feedback::with_inbox`).
pub fn inbox() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    let flag = FLAG_DIR.get().cloned().flatten();
    inbox_from(flag.as_deref(), AppIdentity::CURRENT, |k| std::env::var_os(k))
}
