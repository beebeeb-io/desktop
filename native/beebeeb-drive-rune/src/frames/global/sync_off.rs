//! The refusal modal (the owner's ruling Q3 on the sync-start contract,
//! `docs/superpowers/specs/2026-11-12-native-sync-start-and-sign-out.md` §4.3): a sync start was refused, and the
//! modal says what happened, what the person can do, and offers a fix only where one exists in this build: "Try
//! again" where a retry can work after an outside fix or a passing fault, "Quit" where only a restart helps. A kind
//! with no fix says so plainly and has only "Close".
//!
//! The shell opens it once per refused attempt (`shell::host::poll`) and draws it over whatever the window shows
//! (`registry::DIALOGS`); this file is its words and its card, drawn with the one dialog card (`ui::dialog`). The
//! board has no frame for it: a UI test and a look at the headless render stand in for a frame gate.
//!
//! The folder appears by its name only, never its path; nothing else of the person's is shown.

use rune::{Cx, El, row};

use crate::State;
use crate::shell::app::{Dialog, Event};
use crate::shell::registry::{DialogFrame, FeedbackPolicy, FrameGroup};
use crate::theme::tokens::{self, type_};
use crate::ui::settings_dialog::{self, Weight};
use crate::ui::{dialog, kit};

/// Why the start was refused, as the engine said it (`SyncStartRefused`, one for one).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    FolderOfAnotherProvider,
    FolderUnreadable,
    FolderMissing,
    RegistrationFailed { code: Option<i32> },
    OtherAccountsData,
    IdentityUnknown,
    StopUnconfirmed,
    StateInUse,
    Other,
}

impl Refused {
    /// Every kind, for the tests (a code where one is carried).
    pub const ALL: [Refused; 9] = [
        Refused::FolderOfAnotherProvider,
        Refused::FolderUnreadable,
        Refused::FolderMissing,
        Refused::RegistrationFailed {
            code: Some(0x8007_017C_u32 as i32),
        },
        Refused::OtherAccountsData,
        Refused::IdentityUnknown,
        Refused::StopUnconfirmed,
        Refused::StateInUse,
        Refused::Other,
    ];
}

/// One refused start attempt: what the modal shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncOff {
    /// Which attempt it was (`SyncView::attempt`): the modal opens once for each.
    pub attempt: u64,
    pub kind: Refused,
    /// The name of the folder the start asked for (never its path); `None` when it is not known.
    pub folder: Option<String>,
}

/// What a button of the modal does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Close the modal and ask for one start.
    TryAgain,
    /// Quit the app (a restart is the fix).
    Quit,
    Close,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Action::TryAgain => "Try again",
            Action::Quit => "Quit",
            Action::Close => "Close",
        }
    }
}

/// The modal's words for one refusal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Words {
    pub title: String,
    /// What happened, and that nothing was changed.
    pub happened: String,
    /// What the person can do, or that there is no fix in this version.
    pub fix: String,
    /// The fix first, "Close" last.
    pub actions: &'static [Action],
}

const RETRY: &[Action] = &[Action::TryAgain, Action::Close];
const RESTART: &[Action] = &[Action::Quit, Action::Close];
const NO_FIX: &[Action] = &[Action::Close];
const SUPPORT: &str = "support@beebeeb.io";

/// The words for `off`, with the app's name (`AppIdentity::product_name`: "Beebeeb Test" in a test build).
pub fn words(off: &SyncOff, product: &str) -> Words {
    let folder = match off.folder.as_deref() {
        Some(name) => format!("\u{201c}{name}\u{201d}"),
        None => "the sync folder".to_string(),
    };
    let no_fix = format!("There\u{2019}s no fix for this in this version of {product}. Contact {SUPPORT}.");
    let (title, happened, fix, actions) = match off.kind {
        Refused::FolderOfAnotherProvider => (
            "Another app syncs this folder".to_string(),
            format!(
                "Windows has {folder} registered as another app\u{2019}s sync folder, so {product} didn\u{2019}t \
                 sync into it. Nothing was changed."
            ),
            format!("To sync it with {product}, stop the other app from syncing this folder, then try again."),
            RETRY,
        ),
        Refused::FolderUnreadable => (
            "Windows didn\u{2019}t say whose folder this is".to_string(),
            format!(
                "{product} couldn\u{2019}t read whether another app syncs {folder}, so it didn\u{2019}t sync into \
                 it. Nothing was changed."
            ),
            "This is usually a passing fault. Try again in a moment.".to_string(),
            RETRY,
        ),
        Refused::FolderMissing => (
            "The sync folder is gone".to_string(),
            format!(
                "The folder {folder} isn\u{2019}t where {product} synced it. Sync doesn\u{2019}t start again in a \
                 new, empty folder, so nothing was changed on this PC or in your account."
            ),
            "If you moved or renamed it, put it back where it was, then try again.".to_string(),
            RETRY,
        ),
        Refused::RegistrationFailed { code } => (
            "Windows didn\u{2019}t set up the sync folder".to_string(),
            format!(
                "Windows refused to set up {folder} for {product}{}, so nothing syncs. Nothing was changed.",
                code.map(|code| format!(" (error 0x{:08X})", code as u32))
                    .unwrap_or_default()
            ),
            format!("Try again. If Windows refuses again, restart the PC, then open {product}."),
            RETRY,
        ),
        Refused::OtherAccountsData => (
            "Sync can\u{2019}t start on this PC".to_string(),
            format!(
                "{product} couldn\u{2019}t confirm that the files on this PC belong to the account you\u{2019}re \
                 signed in to, so it didn\u{2019}t sync. Nothing was uploaded or deleted."
            ),
            no_fix,
            NO_FIX,
        ),
        Refused::IdentityUnknown => (
            "Sync can\u{2019}t confirm your account".to_string(),
            format!(
                "{product} couldn\u{2019}t confirm which account the files on this PC belong to, so it \
                 didn\u{2019}t sync. Nothing was uploaded or deleted."
            ),
            no_fix,
            NO_FIX,
        ),
        Refused::StopUnconfirmed => (
            format!("Quit and reopen {product}"),
            format!(
                "Sync didn\u{2019}t confirm that it stopped, so it can\u{2019}t start again until {product} \
                 restarts. Nothing was changed."
            ),
            format!("Quit {product}, then open it again."),
            RESTART,
        ),
        Refused::StateInUse => (
            "Another program is syncing".to_string(),
            format!(
                "Another {product} program, such as an older version or the command line, is syncing on this PC, \
                 so this one didn\u{2019}t start. Nothing was changed."
            ),
            "Quit the other one, then try again.".to_string(),
            RETRY,
        ),
        Refused::Other => (
            "Sync didn\u{2019}t start".to_string(),
            "Sync stopped as it started, or couldn\u{2019}t open its data on this PC. Nothing was changed.".to_string(),
            format!("Try again. If it keeps happening, quit {product} and open it again."),
            RETRY,
        ),
    };
    Words {
        title,
        happened,
        fix,
        actions,
    }
}

/// The registry's line (`shell::registry::DIALOGS`). Comment mode stays off: the modal names the sync folder.
pub const DIALOG: DialogFrame = DialogFrame {
    group: FrameGroup::SyncOffDialog,
    view: card,
    on_key: None,
    feedback: FeedbackPolicy::Denied("it names your sync folder"),
};

/// The card's width (the size of the board's small dialogs, `.wn-dlg` at 440 px).
pub const WIDTH: f32 = 440.0;

/// The card: the title, what happened, what to do, and the buttons (the fix first, Close last). Enter presses the
/// first; Esc closes it (the shell's).
pub fn card(_dialog: &Dialog, s: &State, cx: &mut Cx<State>) -> El {
    let t = tokens::theme(cx.system_dark());
    let unknown = SyncOff {
        attempt: 0,
        kind: Refused::Other,
        folder: None,
    };
    let off = s.shell.sync_off().unwrap_or(&unknown);
    let words = words(off, s.identity.product_name());
    let body = vec![
        kit::label("sync-off-happened", &words.happened, type_::BODY, t.text).wrap(),
        kit::label("sync-off-fix", &words.fix, type_::BODY, t.text_secondary).wrap(),
    ];
    let only = words.actions.len() == 1;
    let mut foot = row().center_y().justify_end();
    for (index, action) in words.actions.iter().copied().enumerate() {
        let weight = if index == 0 && (action != Action::Close || only) {
            Weight::Primary
        } else {
            Weight::Quiet
        };
        let mut button = settings_dialog::button(index, action.label(), weight, &t);
        if index == 0 {
            button = button.default_button();
        }
        foot = foot.child(match action {
            Action::TryAgain => button.on_press(crate::shell::host::try_sync_again),
            Action::Quit => button.on_press_tx(|s: &mut State, tx: &mut rune::Tx| {
                crate::shell::host::dispatch(s, Event::Escape);
                tx.quit();
            }),
            Action::Close => button.on_press(|s: &mut State| {
                crate::shell::host::dispatch(s, Event::Escape);
            }),
        });
    }
    dialog::card(&words.title, WIDTH, &words.title, None, body, foot, &t).key("sync-off")
}

#[cfg(test)]
#[path = "sync_off_tests.rs"]
mod tests;
