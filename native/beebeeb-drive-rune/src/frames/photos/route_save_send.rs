//! Photos routes: SaveToPc, SendCopy (audit P1).
//!
//! - Save to PC: the Platform's save panel, named as the photo is, then `engine_host::save_copy` (through
//!   `shell::viewer_save::save_with`, which binds the whole save to the session of the press). With sync off, or a
//!   photo with no file in the Beebeeb folder, the copy is made in memory there; Save to PC works in both cases.
//! - Send a copy: the Platform's Share sheet on the photo's OWN file in the Beebeeb folder (`PhotosSource::kept_path`;
//!   reading it hydrates a placeholder). No decrypted temp copy is ever written: with sync off, or a photo with no
//!   file there (a device backup), it is refused in words.
//!
//! Must not: log a path or a name; write any file of its own.

use std::path::PathBuf;
use std::sync::Arc;

use rune::Tx;

use super::{Intent, PhotoId, Toast};
use crate::State;
use crate::engine_host::save_copy::CopySaver;
use crate::shell::platform::{self, Platform};
use crate::shell::viewer_save;

#[cfg(test)]
#[path = "route_save_send_tests.rs"]
mod tests;

/// What Send a copy says when the photo's file is not in the Beebeeb folder on this PC.
const NEEDS_SYNC_ELSEWHERE: &str =
    "Couldn\u{2019}t send a copy. It sends from your Beebeeb folder, so sync has to be on for this PC.";
const SHEET_FAILED: &str = "Couldn\u{2019}t open the share sheet.";
const NOT_AVAILABLE: &str = "Not saved: saving a copy is not available here";
const NOT_HERE: &str = "Couldn\u{2019}t find that file on this PC";

/// Takes the intent it can; `Err` hands it back and Photos says "Not available in this version yet".
pub fn handle(s: &mut State, tx: &mut Tx, intent: Intent) -> Result<(), Intent> {
    match intent {
        Intent::SendCopy(id) => send_copy(s, &*platform::current(), id),
        Intent::SaveToPc(id) => match viewer_save::saver() {
            Some(saver) => save_to_pc(s, tx, platform::current(), saver, id),
            None => s.photos.toast = Some(Toast::Words(NOT_AVAILABLE)),
        },
        other => return Err(other),
    }
    Ok(())
}

pub(super) fn send_copy(s: &mut State, platform: &dyn Platform, id: PhotoId) {
    let Some(path) = s.photos.source.kept_path(id) else {
        s.photos.toast = Some(Toast::Words(NEEDS_SYNC_ELSEWHERE));
        return;
    };
    let slot = s.shell.main_window_slot();
    let shown = match slot.lock() {
        Ok(window) => platform.share_file(window.as_ref(), &path),
        Err(_) => platform.share_file(None, &path),
    };
    s.photos.toast = shown.err().map(|_| Toast::Words(SHEET_FAILED));
}

pub(super) fn save_to_pc(
    s: &mut State,
    tx: &mut Tx,
    platform: Arc<dyn Platform>,
    saver: Arc<dyn CopySaver>,
    id: PhotoId,
) {
    let item = s.photos.source.file_items(&[id]).and_then(|items| items.into_iter().next());
    let Some(item) = item else {
        s.photos.toast = Some(Toast::Words(NOT_HERE));
        return;
    };
    // The result lands as `Toast::Said` when the save ends (`viewer_save::poll`).
    s.photos.toast = None;
    viewer_save::save_with(s, tx, platform, saver, item.id, item.name, start_folder());
}

/// Where the panel opens: the person's Downloads folder when there is one.
fn start_folder() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(|home| PathBuf::from(home).join("Downloads"))
        .unwrap_or_default()
}
