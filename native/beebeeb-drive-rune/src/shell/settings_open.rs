//! Settings' `Intent::Open(target)`: the page or place it names, routed to the file that owns it.

use rune::Tx;

use super::host::log;
use crate::State;

/// Opens `target`; one no file takes is logged as `NotWired(Open)`.
pub fn open(s: &mut State, tx: &mut Tx, target: &str) {
    let taken = match target {
        // The installed licence notice (w17-licence-notice): About's row opens the file beside the exe.
        "open-source-licences" => {
            if let Err(why) = crate::frames::settings::about::open_licences(s) {
                log("an Open licences file did not open");
                s.settings.toast = Some(why);
            }
            true
        }
        "privacy" | "export-my-data" => super::settings_open_pages::open(s, tx, target),
        "file-explorer" | "support-bundle" | "choose-photo" => super::settings_open_places::open(s, tx, target),
        _ => false,
    };
    if !taken {
        log("NotWired(Open)");
    }
}
