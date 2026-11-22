//! "Discard and switch" (dc-111): the confirmation the refusal modal (`frames::global::sync_off`) turns into when the
//! previous account's unsent files keep sync from starting (`UnsentFileOfAnotherAccount`) and the person presses
//! "Discard and switch". It names what is lost (how many files, and the names of up to five, never a path), says a
//! copy of each is kept in "Changes not sent" first, and offers "Discard and switch" or "Cancel".
//!
//! The engine does the work (`Engine::discard_and_switch`): keep, then clear in 014's order, then start; neither
//! account's key slot is forgotten (R4). This file holds only the words, the card and which refused attempt the
//! confirmation belongs to. Cancel asks the engine nothing: the ask only read, so nothing changed.
//!
//! The pending confirmation is held here (one window, one modal), tied to the refused attempt it was asked for: a new
//! refusal drops it.

use std::any::Any;
use std::sync::{Arc, Mutex, PoisonError};

use rune::{Cx, El, row};

use crate::State;
use crate::shell::actions::ActionError;
use crate::shell::app::Event;
use crate::theme::tokens::{self, type_};
use crate::ui::settings_dialog::{self, Weight};
use crate::ui::{dialog, kit};

/// How many names the confirmation shows (the engine sends at most this many).
pub const NAMES_SHOWN: usize = 5;

/// What would be lost, as the engine answered the ask. `token` is the engine's own ask, handed back unchanged on
/// confirm, so the engine discards exactly what was shown or nothing.
#[derive(Clone)]
pub struct Ask {
    pub files: usize,
    pub names: Vec<String>,
    pub token: Arc<dyn Any + Send + Sync>,
}

impl std::fmt::Debug for Ask {
    /// The count only: a name is the person's and never goes into a log line.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ask").field("files", &self.files).finish_non_exhaustive()
    }
}

/// The two engine calls the confirmation makes, so a test can stand in for the shell.
pub trait DiscardActions {
    fn discard_ask(&self) -> Result<Ask, ActionError>;
    fn discard_and_switch(&self, ask: Ask) -> Result<(), ActionError>;
}

impl<T: crate::shell::actions::ShellActions + ?Sized> DiscardActions for T {
    fn discard_ask(&self) -> Result<Ask, ActionError> {
        crate::shell::actions::ShellActions::discard_ask(self)
    }
    fn discard_and_switch(&self, ask: Ask) -> Result<(), ActionError> {
        crate::shell::actions::ShellActions::discard_and_switch(self, ask)
    }
}

/// The confirmation waiting for an answer, and the refused attempt it belongs to.
#[derive(Clone, Debug)]
pub struct Pending {
    pub attempt: u64,
    pub ask: Ask,
}

/// One window, one refusal modal: its pending confirmation.
#[derive(Default)]
pub struct Confirmation(Mutex<Option<Pending>>);

impl Confirmation {
    pub const fn new() -> Self {
        Self(Mutex::new(None))
    }

    fn slot(&self) -> std::sync::MutexGuard<'_, Option<Pending>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// "Discard and switch" in the refusal modal: ask the engine what would be lost. `false` when it offers nothing
    /// (the modal stays as it is).
    pub fn open<A: DiscardActions + ?Sized>(&self, attempt: u64, actions: &A) -> bool {
        match actions.discard_ask() {
            Ok(ask) if ask.files > 0 => {
                *self.slot() = Some(Pending { attempt, ask });
                true
            }
            _ => false,
        }
    }

    /// The confirmation for `attempt`, if one waits; one of an earlier attempt is dropped.
    pub fn for_attempt(&self, attempt: u64) -> Option<Pending> {
        let mut slot = self.slot();
        if slot.as_ref().is_some_and(|pending| pending.attempt != attempt) {
            *slot = None;
        }
        slot.clone()
    }

    /// Cancel: the confirmation goes, and the engine is asked nothing.
    pub fn cancel(&self) {
        *self.slot() = None;
    }

    /// "Discard and switch": the shown ask goes back to the engine. Nothing happens without a pending confirmation.
    pub fn confirm<A: DiscardActions + ?Sized>(&self, actions: &A) -> Result<(), ActionError> {
        let Some(pending) = self.slot().take() else {
            return Err(ActionError::NotAvailable("nothing was asked"));
        };
        actions.discard_and_switch(pending.ask)
    }
}

/// The window's confirmation.
pub static CONFIRMATION: Confirmation = Confirmation::new();

/// The card's words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Words {
    pub title: String,
    /// What is lost: the count.
    pub lost: String,
    /// The names, at most [`NAMES_SHOWN`], then "and N more".
    pub names: Vec<String>,
    /// That a copy of each is kept first, and where.
    pub kept: String,
    pub confirm: &'static str,
    pub cancel: &'static str,
}

pub fn words(ask: &Ask) -> Words {
    let files = if ask.files == 1 {
        "1 file".to_string()
    } else {
        format!("{} files", ask.files)
    };
    let mut names: Vec<String> = ask
        .names
        .iter()
        .take(NAMES_SHOWN)
        .map(|name| format!("\u{2022} {name}"))
        .collect();
    let more = ask.files.saturating_sub(names.len());
    if more > 0 {
        names.push(format!("and {more} more"));
    }
    Words {
        title: "Discard the unsent changes?".to_string(),
        lost: format!(
            "{files} in the sync folder, left by the account that was signed in before, may hold changes that were \
             never sent. Discarding takes them out of the sync folder, and that account won\u{2019}t get the changes:"
        ),
        names,
        kept: "A copy of each file is kept first, in the \u{201c}Changes not sent\u{201d} folder. Both accounts stay \
               on this computer as they are."
            .to_string(),
        confirm: "Discard and switch",
        cancel: "Cancel",
    }
}

/// The confirmation card, drawn by the refusal modal in its place while [`CONFIRMATION`] waits for its attempt.
pub fn card(pending: &Pending, cx: &mut Cx<State>) -> El {
    let t = tokens::theme(cx.system_dark());
    let words = words(&pending.ask);
    let mut body = vec![kit::label("discard-switch-lost", &words.lost, type_::BODY, t.text).wrap()];
    for (index, name) in words.names.iter().enumerate() {
        body.push(kit::label(format!("discard-switch-name-{index}"), name, type_::BODY, t.text).wrap());
    }
    body.push(kit::label("discard-switch-kept", &words.kept, type_::BODY, t.text_secondary).wrap());
    let cancel = settings_dialog::button(0, words.cancel, Weight::Quiet, &t)
        .default_button()
        .on_press(|_s: &mut State| CONFIRMATION.cancel());
    let confirm = settings_dialog::button_danger(1, words.confirm, &t).on_press(|s: &mut State| {
        let actions = s.shell.actions.clone();
        if let Err(error) = CONFIRMATION.confirm(&*actions) {
            tracing::warn!(?error, "discard and switch was not asked");
        }
        crate::shell::host::dispatch(s, Event::Escape);
    });
    let foot = row().center_y().justify_end().child(cancel).child(confirm);
    dialog::card(&words.title, super::super::global::sync_off::WIDTH, &words.title, None, body, foot, &t)
        .key("discard-switch")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn ask(files: usize) -> Ask {
        Ask {
            files,
            names: (1..=files.min(NAMES_SHOWN)).map(|n| format!("Invented {n}.txt")).collect(),
            token: Arc::new(files),
        }
    }

    #[derive(Default)]
    struct Fake {
        asks: AtomicUsize,
        discards: Mutex<Vec<usize>>,
        files: usize,
    }

    impl DiscardActions for Fake {
        fn discard_ask(&self) -> Result<Ask, ActionError> {
            self.asks.fetch_add(1, Ordering::SeqCst);
            Ok(ask(self.files))
        }
        fn discard_and_switch(&self, ask: Ask) -> Result<(), ActionError> {
            let shown = *ask.token.downcast_ref::<usize>().expect("the engine's own ask comes back");
            self.discards.lock().unwrap().push(shown);
            Ok(())
        }
    }

    #[test]
    fn the_confirmation_names_up_to_five_and_counts_the_rest() {
        let w = words(&ask(7));
        assert!(w.lost.starts_with("7 files"), "{}", w.lost);
        assert_eq!(w.names.len(), 6);
        assert_eq!(w.names[..5], (1..=5).map(|n| format!("\u{2022} Invented {n}.txt")).collect::<Vec<_>>()[..]);
        assert_eq!(w.names[5], "and 2 more");
        assert!(w.kept.contains("Changes not sent"));
        let one = words(&ask(1));
        assert!(one.lost.starts_with("1 file "), "{}", one.lost);
        assert_eq!(one.names, vec!["\u{2022} Invented 1.txt".to_string()]);
        for text in w.names.iter().chain([&w.lost, &w.kept]) {
            assert!(!text.contains('/') && !text.contains('\\'), "a path: {text}");
        }
    }

    #[test]
    fn cancel_asks_the_engine_nothing() {
        let (confirmation, fake) = (Confirmation::new(), Fake { files: 2, ..Fake::default() });
        assert!(confirmation.open(4, &fake));
        confirmation.cancel();
        assert!(confirmation.for_attempt(4).is_none());
        assert_eq!(confirmation.confirm(&fake), Err(ActionError::NotAvailable("nothing was asked")));
        assert_eq!(fake.discards.lock().unwrap().len(), 0, "no discard after a cancel");
    }

    #[test]
    fn confirm_hands_back_exactly_the_ask_that_was_shown_once() {
        let (confirmation, fake) = (Confirmation::new(), Fake { files: 3, ..Fake::default() });
        assert!(confirmation.open(9, &fake));
        assert_eq!(confirmation.for_attempt(9).map(|p| p.ask.files), Some(3));
        confirmation.confirm(&fake).unwrap();
        assert!(confirmation.confirm(&fake).is_err(), "one confirm, one discard");
        assert_eq!(*fake.discards.lock().unwrap(), vec![3]);
    }

    #[test]
    fn a_new_refusal_drops_the_old_confirmation_and_nothing_unsent_opens_none() {
        let (confirmation, fake) = (Confirmation::new(), Fake { files: 1, ..Fake::default() });
        assert!(confirmation.open(1, &fake));
        assert!(confirmation.for_attempt(2).is_none());
        assert!(confirmation.for_attempt(1).is_none(), "dropped, not hidden");
        assert!(!confirmation.open(3, &Fake::default()), "nothing to discard: no confirmation");
        assert!(confirmation.for_attempt(3).is_none());
    }
}
