//! What Settings (and the sign-in screens) asked the shell for: web pages, the clipboard, Windows Settings.
//! Every variant has an arm; one this build does not do is logged as `NotWired(<variant>)`, never dropped.
//! And what the frames of Trash, Shared and Search asked for (`frames::Intent`): an item's Details and its
//! read-only Preview (`frames::item`); every other intent is taken and logged by its fixed words only.

use rune::Tx;

use super::host::log;
use super::web_pages;
use crate::State;
use crate::frames::settings::Intent;

/// Opens `address` (one of the table's) and says so when it did not open: a log line, and the toast of the
/// pane that asked.
fn open(s: &mut State, address: &str) {
    if crate::engine_host::signin::open_web_page(address).is_err() {
        log("a web page did not open");
        s.settings.toast = Some("Could not open that page");
    }
}

#[cfg(test)]
thread_local! {
    static POLL_DRAINS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Tests only: let the shell's poll act on Settings' and the frames' queues. By default a test's window leaves
/// both alone, because the frames' own tests read what a press asked for (`settings.intents`, `frames.intents`).
#[cfg(test)]
pub fn poll_drains_for_this_test() {
    POLL_DRAINS.set(true);
}

/// The shell's poll: acts on the queues (in a test, only where the test asked for it).
pub fn drain_in_poll(s: &mut State, tx: &mut Tx) -> bool {
    #[cfg(test)]
    if !POLL_DRAINS.get() {
        return false;
    }
    let settings = drain_settings_intents(s, tx);
    let frames = drain_frame_intents(s, tx);
    settings | frames
}

/// Takes every intent the frames of Trash, Shared and Search queued. Details (`About("DW-0411", id)`) and Preview
/// (`Do("preview", id)`) open over the section shown when the id is in its current list; every other intent is
/// logged as `NotWired(<its fixed words>)`. No line carries an id, a name or typed words. `true`: something was
/// taken.
pub fn drain_frame_intents(s: &mut State, tx: &mut Tx) -> bool {
    use crate::frames::Intent as Frame;
    let intents = std::mem::take(&mut s.frames.intents);
    for intent in &intents {
        match intent {
            Frame::About("DW-0411", id) => {
                if !crate::frames::item::open_details(s, id, tx) {
                    log("an item's Details found no row");
                }
            }
            Frame::Do("preview", id) => {
                if !crate::frames::item::open_preview(s, id, tx) {
                    log("an item's Preview found no row");
                }
            }
            other => log(format!("NotWired({})", crate::frames::item::tag(other))),
        }
    }
    !intents.is_empty()
}

/// Takes every intent Settings queued and acts on it. `true`: something was taken.
pub fn drain_settings_intents(s: &mut State, tx: &mut Tx) -> bool {
    let intents = s.settings.take_intents();
    for intent in &intents {
        match intent {
            Intent::Web(name) => match web_pages::url(name) {
                Some(address) => open(s, address),
                None => log(format!("NotWired(web page {name})")),
            },
            Intent::Copy(text) => {
                tx.copy(text.clone());
                s.settings.toast = Some("Copied");
            }
            Intent::OpenWindowsNotificationSettings => open(s, web_pages::WINDOWS_NOTIFICATIONS),
            Intent::Show(_) => log("NotWired(Show)"),
            Intent::Open(_) => log("NotWired(Open)"),
            // Taken by the settings source's updater (`SettingsSource::download_update`, `restart_to_update`): the
            // shell's part of the hand-off (quit once the installer started, start sync again after a failed start)
            // comes through `frames::settings::poll`.
            Intent::DownloadUpdate | Intent::RestartToUpdate => {}
            Intent::SaveBundleAndWriteEmail => log("NotWired(SaveBundleAndWriteEmail)"),
            Intent::CheckForUpdates => log("NotWired(CheckForUpdates)"),
            Intent::ShowInFiles(_) => log("NotWired(ShowInFiles)"),
        }
    }
    !intents.is_empty()
}

pub(super) fn open_for_signin(address: &str) {
    if crate::engine_host::signin::open_web_page(address).is_err() {
        log("a web page did not open");
    }
}

#[cfg(test)]
#[path = "intents_tests.rs"]
mod tests;
