//! Settings' `Intent::Open(target)`: the page or place it names, routed to the file that owns it.

use rune::Tx;

use super::host::log;
use crate::State;

/// Opens `target`; one no file takes is logged as `NotWired(Open)`.
pub fn open(s: &mut State, tx: &mut Tx, target: &str) {
    let taken = match target {
        "privacy" | "export-my-data" | "open-source-licences" => super::settings_open_pages::open(s, tx, target),
        "file-explorer" | "support-bundle" | "choose-photo" => super::settings_open_places::open(s, tx, target),
        _ => false,
    };
    if !taken {
        log("NotWired(Open)");
    }
}
