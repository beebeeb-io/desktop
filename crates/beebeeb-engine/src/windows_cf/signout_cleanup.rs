//! Portable orchestration for Windows sign-out. The database remains the retry
//! journal until every native placeholder and external plaintext cache is gone.
use std::path::{Path, PathBuf};

pub(super) fn purge(
    cache_paths: impl IntoIterator<Item = Option<String>>,
    is_disposable: impl Fn(&Path) -> bool,
    remove_placeholders: impl FnOnce() -> anyhow::Result<()>,
    finish: impl FnOnce() -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let mut external = Vec::new();
    // Resolve and validate the entire cache inventory BEFORE destructive work.
    for path in cache_paths.into_iter().flatten() {
        // The engine records plaintext hydrated inside a CFAPI placeholder as
        // Some(""). Native cleanup owns those bytes; there is no external file.
        if path.is_empty() {
            continue;
        }
        let path = PathBuf::from(path);
        anyhow::ensure!(is_disposable(&path), "Cache cleanup refused an unexpected path");
        external.push(path);
    }

    remove_placeholders()?;
    for path in external {
        // Recheck after native work too, in case an external cache path changed.
        anyhow::ensure!(is_disposable(&path), "Cache cleanup refused an unexpected path");
        match std::fs::remove_file(path) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
    }
    // Failure leaves all references intact. A retry tolerates removed files.
    finish()
}
