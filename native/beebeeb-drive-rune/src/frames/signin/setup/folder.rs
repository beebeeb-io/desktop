//! Two states of the first run the board has no frame for, drawn with the board's own parts (the setup kit and the
//! account kit): the sync folder's question (the owner's ruling Q2 on the sync-start contract: the first run asks
//! for the folder before the first start), and a step nothing answers for in this version.
//!
//! No frame gate: a UI test and a look at the headless render instead (as the contract says of the refusal modal).

use rune::tok::Family;
use rune::{El, TextStyle, col};

use super::source::{FolderAsk, FolderSaid};
use super::view::{actions, ghost, primary};
use super::{Step, Ui, icons};
use crate::theme::tokens::{Theme, type_};
use crate::ui::au::{self, ButtonState, Keys, Tone};
use crate::ui::{kit, setup};

/// The folder as the PC writes it: mono, as every path the app shows.
const PATH: TextStyle = TextStyle::new(12.0, 400.0).family(Family::Mono).lh(17.0);

/// Why a folder was not taken, and what to do. Names no path: the folder is the one the person just chose.
pub fn said(why: FolderSaid) -> (&'static str, &'static str) {
    match why {
        FolderSaid::Network => (
            "That folder is on the network",
            "Beebeeb's folder has to be on this PC. Choose a folder on one of its drives.",
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
            "That isn't a folder on this PC",
            "Choose a folder on one of this PC's drives.",
        ),
        FolderSaid::NoPicker => (
            "Windows' folder picker didn't open",
            "Nothing was changed. Try again, or use the suggested folder.",
        ),
    }
}

/// The File Explorer step while it asks for the sync folder.
pub fn ask(ask: &FolderAsk, k: &mut Keys, t: &Theme) -> El {
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
            Some("Your files show up in this folder, and in File Explorer. Nothing syncs until you choose."),
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
            Some("This isn't in this version of Beebeeb yet. Nothing is turned on, and nothing is changed on this PC."),
            false,
            t,
        ))
        .child(actions().child(primary(k, "Continue", ButtonState::Enabled, t, Ui::go_on)))
}
