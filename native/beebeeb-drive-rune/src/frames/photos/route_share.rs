//! Photos routes: ShareSeveral, CopyLink, ChooseExpiry and VersionHistory, by Files' own dialogs and source.
//!
//! Share… on a selection and Version history open Files' dialogs over Photos (`begin_share_for`,
//! `begin_versions_for`: the same dialog, defaults and source call as in Files). The Share dialog of an open photo is
//! Photos' own (`overlay::share`); its Copy link asks `FilesSource::share_link` ONCE, for that photo, with the
//! expiry and password that were chosen. A link has no view-only limit (decision 11-17), so there is no "Can edit".
//! The password is typed in a secure field (`Ui::share_password`) and is wiped here as soon as it was used, whatever
//! the answer; the window is kept out of captures while the field shows (`shell::host::poll_capture`).

use rune::Tx;

use super::{Dialog, Intent, PhotoId, Toast};
use crate::State;
use crate::frames::files::actions_data::{LinkAccess, LinkExpiry, ShareOptions};
use crate::frames::files::share::Share;
use crate::frames::files::source::ActionError;

const NOT_HERE: &str = concat!("Couldn\u{2019}t find that file on ", beebeeb_engine::device_word!());
const PROTECTED: &str = "Couldn\u{2019}t share \u{00B7} this is kept safe and can\u{2019}t be shared";

/// Takes the intent it can; `Err` hands it back and Photos says "Not available in this version yet".
pub fn handle(s: &mut State, _tx: &mut Tx, intent: Intent) -> Result<(), Intent> {
    match intent {
        Intent::ShareSeveral(ids) => share_several(s, &ids),
        Intent::CopyLink { id, expires, password } => copy_link(s, id, expires, password),
        Intent::ChooseExpiry(_) => {
            if let Some(Dialog::Share(draft)) = &mut s.photos.dialog {
                draft.menu = !draft.menu;
            }
        }
        Intent::VersionHistory(id) => versions(s, id),
        other => return Err(other),
    }
    Ok(())
}

fn share_several(s: &mut State, ids: &[PhotoId]) {
    let items = s.photos.source.file_items(ids).filter(|items| !items.is_empty());
    let Some(items) = items else {
        s.photos.toast = Some(Toast::Words(NOT_HERE));
        return;
    };
    s.photos.menu = None;
    if !s.begin_share_for(items) {
        s.photos.toast = Some(Toast::Words(PROTECTED));
    }
}

fn versions(s: &mut State, id: PhotoId) {
    let item = s.photos.source.file_items(&[id]).and_then(|items| items.into_iter().next());
    let Some(item) = item else {
        s.photos.toast = Some(Toast::Words(NOT_HERE));
        return;
    };
    s.photos.menu = None;
    if !s.begin_versions_for(item) {
        s.photos.toast = Some(Toast::Words("Version history is for files only"));
    }
}

fn copy_link(s: &mut State, id: PhotoId, expires: LinkExpiry, password: bool) {
    // Taken out of the field now: whatever happens next, nothing typed stays behind.
    let typed = password.then(|| s.photos.share_password.text.clone());
    s.photos.wipe_share_password();
    let items = s.photos.source.file_items(&[id]).filter(|items| !items.is_empty());
    let Some(items) = items else {
        s.photos.toast = Some(Toast::Words(NOT_HERE));
        return;
    };
    if s.protects(&items) {
        s.photos.toast = Some(Toast::Words(PROTECTED));
        return;
    }
    // One link at a time: an answer names no item, so a second is not asked for while one is on its way.
    if s.actions.link.asked.is_some() || s.actions.link.share.is_some() {
        s.photos.toast = Some(Toast::Words("Couldn\u{2019}t make the link. The last change is still on its way."));
        return;
    }
    let options = ShareOptions { access: LinkAccess::default(), expires, password: typed };
    let answer = s.actions.source.share_link(&items, &options);
    let password_on = options.password.is_some();
    drop(options);
    match answer {
        // Accepted, not made: Files' poll puts the link on the clipboard when the source answers (`files::link`).
        Ok(()) if s.actions.source.answers_later() => {
            s.actions.link.share = Some(Share {
                items,
                access: LinkAccess::default(),
                expires,
                password_on,
                password_missing: false,
                menu: None,
            });
        }
        Ok(()) => s.photos.toast = Some(Toast::Words("Link copied")),
        Err(ActionError::Offline) => {
            s.photos.toast = Some(Toast::Words("Couldn\u{2019}t make the link \u{00B7} needs a connection"))
        }
        Err(_) => s.photos.toast = Some(Toast::Words("Couldn\u{2019}t make the link")),
    }
}
