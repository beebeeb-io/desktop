//! Settings' Open targets privacy and export-my-data: each is a web page (the data export runs in the web app).
//! `open-source-licences` is taken by w17-licence-notice, not here.

use rune::Tx;

use super::host::log;
use super::web_pages;
use crate::State;

/// Opens `target`; `false`: not taken.
pub fn open(s: &mut State, _tx: &mut Tx, target: &str) -> bool {
    if !matches!(target, "privacy" | "export-my-data") {
        return false;
    }
    match web_pages::url(target) {
        Some(address) => {
            if crate::engine_host::signin::open_web_page(address).is_err() {
                log("a web page did not open");
                s.settings.toast = Some("Could not open that page");
            }
            true
        }
        None => false,
    }
}

#[cfg(test)]
#[path = "settings_open_pages_tests.rs"]
mod tests;
