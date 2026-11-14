//! What Settings (and the sign-in screens) asked the shell for: web pages, the clipboard, Windows Settings.
//! Every variant has an arm; one this build does not do is logged as `NotWired(<variant>)`, never dropped.

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
            Intent::DownloadUpdate => log("NotWired(DownloadUpdate)"),
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
