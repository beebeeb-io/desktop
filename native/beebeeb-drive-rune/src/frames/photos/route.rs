//! Where the intents that `session` does not handle go: one match here, one `handle` per feature file (each file is
//! edited by its own task). A handler that does not take the intent hands it back (`Err`), and Photos says "Not
//! available in this version yet" as it always did.

use rune::Tx;

use super::{Intent, Toast, route_chrome, route_items, route_original, route_save_send, route_share};
use crate::State;

/// Hands `intent` to its feature file. `true`: something drawn changed.
pub fn route(s: &mut State, tx: &mut Tx, intent: Intent) -> bool {
    let (file, result) = match intent {
        Intent::Search(_) | Intent::NewInFiles => ("route_chrome", route_chrome::handle(s, tx, intent)),
        Intent::ShareSeveral(_) | Intent::CopyLink { .. } | Intent::ChooseExpiry(_) | Intent::VersionHistory(_) => {
            ("route_share", route_share::handle(s, tx, intent))
        }
        Intent::Rename(_) | Intent::Duplicate(_) | Intent::ShowInFolder(_) => ("route_items", route_items::handle(s, tx, intent)),
        Intent::SaveToPc(_) | Intent::SendCopy(_) => ("route_save_send", route_save_send::handle(s, tx, intent)),
        Intent::DownloadOriginal(_) | Intent::StopOriginal(_) => {
            ("route_original", route_original::handle(s, tx, intent))
        }
        other => ("none", Err(other)),
    };
    #[cfg(test)]
    REACHED.with(|r| r.borrow_mut().push(file));
    let _ = file;
    if result.is_err() {
        s.photos.toast = Some(Toast::NotYet);
    }
    true
}

#[cfg(test)]
thread_local! {
    static REACHED: std::cell::RefCell<Vec<&'static str>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Which feature files were reached on this thread since `reached_clear`.
#[cfg(test)]
pub fn reached() -> Vec<&'static str> {
    REACHED.with(|r| r.borrow().clone())
}

#[cfg(test)]
pub fn reached_clear() {
    REACHED.with(|r| r.borrow_mut().clear());
}
