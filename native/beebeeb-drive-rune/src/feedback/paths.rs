//! Where notes go. Precedence: `--feedback-dir <path>`, then `BEEBEEB_FEEDBACK_DIR`, then the
//! app's local data folder: `%LOCALAPPDATA%\<app id>\feedback\inbox` on Windows.
//!
//! The crate has no data-dir resolution of its own yet, so the app id is resolved here, in one
//! place: `BEEBEEB_APP_ID` at run time, else the `BEEBEEB_APP_ID` the build was compiled with
//! (packaging may set its own), else the test identity `io.beebeeb.app.test`: never the real
//! app's folder by default.

use std::ffi::OsString;
use std::path::PathBuf;

/// The TEST identity: the feature exists only in test builds, so its notes never land in the
/// real app's data folder unless the build or the environment names that identity itself.
pub const DEFAULT_APP_ID: &str = "io.beebeeb.app.test";
pub const DIR_FLAG: &str = "--feedback-dir";
pub const DIR_ENV: &str = "BEEBEEB_FEEDBACK_DIR";
pub const APP_ID_ENV: &str = "BEEBEEB_APP_ID";

pub fn app_id(var: &impl Fn(&str) -> Option<OsString>) -> String {
    var(APP_ID_ENV)
        .and_then(|v| v.into_string().ok())
        .filter(|v| !v.is_empty())
        .or_else(|| {
            option_env!("BEEBEEB_APP_ID")
                .filter(|v| !v.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| DEFAULT_APP_ID.to_string())
}

/// The inbox for these arguments and this environment. `None` only when no home folder exists.
pub fn inbox_from(args: &[String], var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    if let Some(i) = args.iter().position(|a| a == DIR_FLAG) {
        if let Some(dir) = args.get(i + 1).filter(|d| !d.is_empty()) {
            return Some(PathBuf::from(dir));
        }
    }
    if let Some(dir) = var(DIR_ENV).filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    let id = app_id(&var);
    let base = if cfg!(target_os = "windows") {
        var("LOCALAPPDATA")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)?
            .join(id)
    } else {
        rune::dirs::AppDirs::from_env(&id, &var)?.data().to_path_buf()
    };
    Some(base.join("feedback").join("inbox"))
}

/// Reads the process's own arguments (not `rune::extra_args`, which parses Rune's flags and
/// would do so inside a test binary too).
pub fn inbox() -> Option<PathBuf> {
    let args: Vec<String> = std::env::args().collect();
    inbox_from(&args, |k| std::env::var_os(k))
}
