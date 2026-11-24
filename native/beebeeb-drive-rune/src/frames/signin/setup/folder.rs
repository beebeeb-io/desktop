//! Two states of the first run the board has no frame for, drawn with the board's own parts (the setup kit and the
//! account kit): the sync folder's question (the owner's ruling Q2 on the sync-start contract: the first run asks
//! for the folder before the first start), and a step nothing answers for in this version.
//!
//! No frame gate: a UI test and a look at the headless render instead (as the contract says of the refusal modal).

use rune::tok::Family;
use rune::{col, El, TextStyle};

use super::source::{FolderAsk, FolderSaid};
use super::view::{actions, ghost, primary};
use super::{act, icons, Step, Ui};
use crate::theme::tokens::{type_, Theme};
use crate::ui::au::{self, ButtonState, Keys, Tone};
use crate::ui::view_state::{try_again, view_state, ViewState};
use crate::ui::{kit, setup};

/// The folder as the PC writes it: mono, as every path the app shows.
const PATH: TextStyle = TextStyle::new(12.0, 400.0).family(Family::Mono).lh(17.0);

/// Why a folder was not taken, and what to do. Names no path: the folder is the one the person just chose.
pub fn said(why: FolderSaid) -> (&'static str, &'static str) {
    match why {
        FolderSaid::Network => (
            "That folder is on the network",
            concat!("Beebeeb's folder has to be on ", beebeeb_engine::device_word!(), ". Choose a folder on one of its drives."),
        ),
        FolderSaid::System => (
            "That folder belongs to Windows",
            "A drive itself and the folders of Windows and its programs can't be Beebeeb's folder. Choose another folder.",
        ),
        FolderSaid::Home => (
            "That folder holds everything of yours",
            "Your user folder and the folder apps keep their data in can't be Beebeeb's folder. Choose a folder inside your user folder.",
        ),
        FolderSaid::OwnData => (
            "Beebeeb keeps its own data there",
            "Syncing it would never finish. Choose another folder.",
        ),
        FolderSaid::NotAFolder => ("That isn't a folder", "Choose a folder, or a place for a new one."),
        FolderSaid::AnotherApps => (
            "That folder is another app's sync folder",
            "Two apps can't sync one folder. Choose another folder.",
        ),
        FolderSaid::Unreadable => (
            "That folder couldn't be read",
            "Nothing was changed. Try again, or choose another folder.",
        ),
        FolderSaid::NotAFullPath => (
            concat!("That isn't a folder on ", beebeeb_engine::device_word!()),
            concat!("Choose a folder on one of ", beebeeb_engine::device_word!(), "'s drives."),
        ),
        FolderSaid::NoPicker => (
            "Windows' folder picker didn't open",
            "Nothing was changed. Try again, or use the suggested folder.",
        ),
    }
}

/// The words of the state "sync started, but the folder could not be written to the settings".
const UNSAVED_TITLE: &str = "Couldn't save your folder choice";
const UNSAVED_TEXT: &str = concat!(
    "Sync is running in the folder you chose, but ",
    beebeeb_engine::device_word!(),
    " didn't keep the choice. Your files are safe."
);
const UNSAVED_BANNER_TITLE: &str = "Beebeeb's settings couldn't be written";
const UNSAVED_BANNER_TEXT: &str =
    "Try again in a moment. Until it is saved, Beebeeb may use another folder after a restart.";

/// A pick after a start worked: moving the folder isn't available yet; nothing changed.
pub fn already_syncs(k: &mut Keys, t: &Theme) -> El {
    setup::column()
        .child(au::heading(
            "Beebeeb already syncs in another folder",
            Some("Moving your Beebeeb folder isn't available in this version yet. Your files keep syncing in the folder you chose first."),
            false,
            t,
        ))
        .child(actions().child(primary(k, "OK", ButtonState::Enabled, t, Ui::go_on)))
}

/// The File Explorer step when sync runs but the folder was not written to the settings.
pub fn unsaved(k: &mut Keys, t: &Theme) -> El {
    setup::column()
        .child(au::heading(UNSAVED_TITLE, Some(UNSAVED_TEXT), false, t))
        .child(au::banner(
            k,
            Tone::Red,
            &icons::ALERT,
            Some(UNSAVED_BANNER_TITLE),
            Some(UNSAVED_BANNER_TEXT),
            true,
            t,
        ))
        .child(
            actions()
                .child(primary(k, "Try again", ButtonState::Enabled, t, Ui::go_on))
                .child(ghost(k, "Continue for now", ButtonState::Enabled, t, Ui::not_now)),
        )
}

/// The File Explorer step while it asks for the sync folder.
pub fn ask(ask: &FolderAsk, k: &mut Keys, t: &Theme) -> El {
    if *ask == FolderAsk::AlreadySyncs {
        return already_syncs(k, t);
    }
    let FolderAsk::Ask {
        folder,
        said: why,
        busy,
    } = ask
    else {
        return setup::column();
    };
    let state = if *busy { ButtonState::Busy } else { ButtonState::Enabled };
    let place = col()
        .key("folder-place")
        .role("group")
        .label(format!("Beebeeb's folder: {folder}"))
        .w_full()
        .px(13.0)
        .py(11.0)
        .gap(3.0)
        .radius(13.0)
        .fill(t.text.alpha(0.05))
        .child(
            kit::label(
                "folder-place-title",
                "Beebeeb's folder",
                type_::CAPTION,
                t.text_tertiary,
            )
            .a11y_hidden(),
        )
        .child(
            kit::label("folder-place-path", folder.clone(), PATH, t.text)
                .wrap()
                .w_full()
                .a11y_hidden(),
        );
    setup::column()
        .child(au::heading(
            "Choose where Beebeeb's folder goes",
            Some(concat!(
                "Your files show up in this folder, and in ",
                beebeeb_engine::file_manager!(),
                ". Nothing syncs until you choose."
            )),
            false,
            t,
        ))
        .child(place)
        .children(why.map(|why| {
            let (title, text) = said(why);
            au::banner(k, Tone::Red, &icons::ALERT, Some(title), Some(text), true, t)
        }))
        .child(
            actions()
                .child(primary(k, "Use this folder", state, t, Ui::go_on))
                .child(ghost(k, "Choose another folder\u{2026}", state, t, Ui::choose_folder)),
        )
}

/// A step nothing answers for in this version: it keeps its place and its name, says so, and turns nothing on.
pub fn not_yet(step: Step, k: &mut Keys, t: &Theme) -> El {
    setup::column()
        .child(au::heading(
            step.label(),
            Some(concat!(
                "This isn't in this version of Beebeeb yet. Nothing is turned on, and nothing is changed on ",
                beebeeb_engine::device_word!(),
                "."
            )),
            false,
            t,
        ))
        .child(actions().child(primary(k, "Continue", ButtonState::Enabled, t, Ui::go_on)))
}

/// Step 3 while the folders on this PC are being read: the step's own heading and a loading state, no folder and
/// no switch until the read answered. "Not now" stays; turning backup on waits for the folders.
pub fn backup_loading(k: &mut Keys, t: &Theme) -> El {
    setup::column()
        .child(au::heading("Back up your folders", None, false, t))
        .child(view_state(
            ViewState::Loading {
                words: concat!("Looking at the folders on ", beebeeb_engine::device_word!(), "\u{2026}"),
            },
            t,
        ))
        .child(
            actions()
                .child(primary(k, "Turn on backup", ButtonState::Disabled, t, Ui::go_on))
                .child(ghost(k, "Not now", ButtonState::Enabled, t, Ui::not_now)),
        )
}

/// Step 3 when reading the folders failed: said as a failure, never as "not in this version". Try again reads them
/// again; Not now goes on with nothing turned on.
pub fn backup_failed(k: &mut Keys, t: &Theme) -> El {
    let retry = try_again("backup-failed", t).on_press_tx(act(Ui::retry_backup));
    setup::column()
        .child(au::heading("Back up your folders", None, false, t))
        .child(view_state(
            ViewState::Failed {
                key: "backup-failed",
                title: "Couldn\u{2019}t read your folders",
                why: concat!(
                    "Beebeeb couldn\u{2019}t read the folders on ",
                    beebeeb_engine::device_word!(),
                    ". Nothing is turned on."
                ),
                retry: Some(retry),
            },
            t,
        ))
        .child(actions().child(ghost(k, "Not now", ButtonState::Enabled, t, Ui::not_now)))
}

/// What step 3 says when the read worked and found nothing: only what is true (this app cannot add folders).
pub const BACKUP_NONE: &str = concat!(
    "Beebeeb found no folders it can back up on ",
    beebeeb_engine::device_word!(),
    ". Nothing is turned on."
);

/// Step 3 when the read worked and this PC has no folder to offer: said as that, with a way on.
pub fn backup_none(k: &mut Keys, t: &Theme) -> El {
    setup::column()
        .child(au::heading("Back up your folders", Some(BACKUP_NONE), false, t))
        .child(actions().child(primary(k, "Continue", ButtonState::Enabled, t, Ui::not_now)))
}
