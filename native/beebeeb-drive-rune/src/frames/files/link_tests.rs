//! Copy link and the Share dialog's link over a source that answers later (as the engine's does): nothing is said
//! and nothing is copied at the press; the link goes to the clipboard when the source answered with one, and only
//! then; what was not made is said, with a Try again that asks for the same items or none at all; nothing of an
//! earlier session reaches the clipboard. The clipboard is the toolkit's in-memory one. Every id is the board's;
//! the link is invented.

use super::super::Toast;
use super::super::dialogs::Dialog;
use super::super::protected;
use super::super::source::{
    Account, Action, ActionError, Change, Counts, Details, FilesSource, NoEngine, StorageSplit,
};
use super::super::tests::{runtime_of, settle};
use super::{LinkText, poll};
use crate::State;
use crate::frames::files::actions_data::ShareOptions;
use crate::source::DriveItem;
use rune::Runtime;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

type R = Result<(), ActionError>;

const LINK: &str = "https://app.beebeeb.io/s/an-invented-token#key=an-invented-key";
const BEFORE: &str = "what was on the clipboard before";

/// What the test keeps of the source: what it was asked, and what it will answer.
#[derive(Default, Clone)]
struct Spy {
    later: Arc<Mutex<Vec<Change>>>,
    asked: Arc<Mutex<Vec<(Action, Vec<String>)>>>,
    /// The next link is refused at once with this.
    refuse: Arc<Mutex<Option<ActionError>>>,
    /// The source has no link on its way any more (the answer of the last press was dropped).
    dropped: Arc<Mutex<bool>>,
}

impl Spy {
    fn asked(&self) -> Vec<(Action, Vec<String>)> {
        std::mem::take(&mut self.asked.lock().unwrap())
    }
    fn answer(&self, action: Action, answer: Result<&str, ActionError>) {
        let answer = answer.map(LinkText::new);
        self.later.lock().unwrap().push(Change::Link { action, answer });
    }
    fn ask(&self, action: Action, items: &[DriveItem]) -> R {
        if let Some(why) = self.refuse.lock().unwrap().take() {
            return Err(why);
        }
        let ids = items.iter().map(|item| item.id.clone()).collect();
        self.asked.lock().unwrap().push((action, ids));
        Ok(())
    }
}

/// A source that answers later: a link is accepted, and what comes of it is what the test says.
struct Later(Spy);

#[rustfmt::skip]
impl FilesSource for Later {
    fn counts(&self) -> Option<Counts> { None }
    fn account(&self) -> Option<Account> { None }
    fn storage_split(&self) -> Option<StorageSplit> { None }
    fn sync_state(&self) -> Option<String> { None }
    fn details(&self, item: &DriveItem, folder: &str) -> Details { NoEngine.details(item, folder) }
    fn changed_label(&self, _: &DriveItem) -> Option<String> { None }
    fn can_add(&self, _: Option<&str>) -> bool { true }
    fn sort_of(&self, _: Option<&str>) -> Option<super::super::Sort> { None }
    fn set_sort(&mut self, _: Option<&str>, _: super::super::Sort) {}
    fn trash(&mut self, _: &[DriveItem]) -> R { Ok(()) }
    fn restore(&mut self, _: &[DriveItem]) -> R { Ok(()) }
    fn duplicate(&mut self, _: &[DriveItem], _: &[&str]) -> R { Ok(()) }
    fn keep_offline(&mut self, _: &[DriveItem], _: bool) -> R { Ok(()) }
    fn copy_link(&mut self, items: &[DriveItem]) -> R { self.0.ask(Action::CopyLink, items) }
    fn share_link(&mut self, items: &[DriveItem], _: &ShareOptions) -> R { self.0.ask(Action::ShareLink, items) }
    fn open(&mut self, _: &[DriveItem]) -> R { Ok(()) }
    fn show_in_file_explorer(&mut self, _: &[DriveItem]) -> R { Ok(()) }
    fn upload(&mut self, _: Option<&str>, _: &[PathBuf]) -> R { Ok(()) }
    fn new_folder(&mut self, _: Option<&str>, _: &str) -> R { Ok(()) }
    fn rename(&mut self, _: &DriveItem, _: &str) -> R { Ok(()) }
    fn move_to(&mut self, _: &[DriveItem], _: Option<&str>) -> R { Ok(()) }
    fn free_up_space(&mut self, _: &[DriveItem]) -> R { Ok(()) }
    fn open_in_file_explorer(&mut self, _: Option<&str>) -> R { Ok(()) }
    fn changes(&mut self) -> Vec<Change> { std::mem::take(&mut self.0.later.lock().unwrap()) }
    fn answers_later(&self) -> bool { true }
    fn link_pending(&self) -> bool { !*self.0.dropped.lock().unwrap() }
    fn wake_after(&mut self, _: Duration) {}
}

/// The window over such a source, two plain files first in the list, the first selected; a clipboard that holds
/// [`BEFORE`]. The ids of the two files.
fn window() -> (Runtime<State>, Spy, rune::clipboard::MemoryClipboard, Vec<String>) {
    let spy = Spy::default();
    let mut state = crate::init();
    state.actions.source = Box::new(Later(spy.clone()));
    let free: Vec<usize> = (0..state.listing.items.len())
        .filter(|&row| !state.listing.items[row].is_protected && !state.listing.items[row].is_folder)
        .take(2)
        .collect();
    assert_eq!(free.len(), 2, "the board has too few plain files for this test");
    for (to, from) in free.into_iter().enumerate() {
        state.listing.items.swap(to, from);
    }
    state.selected = Some(0);
    state.actions.selection.insert(0);
    let ids = state.listing.items[..2].iter().map(|item| item.id.clone()).collect();
    let mut rt = runtime_of(state);
    let clipboard = rune::clipboard::memory();
    clipboard.set_text(BEFORE);
    rt.use_clipboard(&clipboard);
    (rt, spy, clipboard, ids)
}

fn copied(clipboard: &rune::clipboard::MemoryClipboard) -> String {
    clipboard.text().map(|text| text.to_string()).unwrap_or_default()
}

/// One frame's look at the source: what it did, then a link for the clipboard.
fn frame(rt: &mut Runtime<State>) -> bool {
    let mut changed = false;
    rt.update(|s, tx| {
        changed = protected::poll(s);
        changed |= poll(s, tx);
    });
    settle(rt);
    changed
}

/// The same through the shell's own poll, as a running app takes it.
fn shell_frame(rt: &mut Runtime<State>) {
    rt.update(|s, tx| {
        crate::shell::host::poll(s, 0.0, tx);
    });
    settle(rt);
}

fn select_only(rt: &mut Runtime<State>, row: usize) {
    rt.update(|s, _| {
        s.actions.selection.clear();
        s.actions.selection.insert(row);
        s.selected = Some(row);
    });
}

#[test]
fn nothing_is_said_or_copied_before_the_source_answered_and_then_the_link_is_on_the_clipboard() {
    let (mut rt, spy, clipboard, ids) = window();
    rt.update(|s, tx| assert!(s.copy_link(tx)));
    assert_eq!(spy.asked(), [(Action::CopyLink, vec![ids[0].clone()])]);
    assert!(!frame(&mut rt), "a frame with no answer changed something");
    assert_eq!(rt.state.actions.toast, None, "said before there was a link");
    assert_eq!(
        copied(&clipboard),
        BEFORE,
        "the clipboard was written before there was a link"
    );

    spy.answer(Action::CopyLink, Ok(LINK));
    assert!(frame(&mut rt));
    assert_eq!(copied(&clipboard), LINK, "the link Beebeeb accepted, whole");
    assert_eq!(rt.state.actions.toast, Some(Toast::LinkCopied));
    // Held for that one step only.
    assert!(rt.state.actions.link.landed.is_empty());
    assert_eq!(rt.state.actions.link.asked, None);
}

#[test]
fn a_password_of_spaces_is_no_password_and_no_link_is_asked_for() {
    let (mut rt, spy, _, _) = window();
    rt.update(|s, tx| {
        assert!(s.begin_share());
        if let Some(Dialog::Share(share)) = &mut s.actions.dialog {
            share.password_on = true;
        }
        s.actions.share_password = rune::TextEdit::new("   ").secure();
        s.share_copy_link(tx);
    });
    assert_eq!(
        spy.asked(),
        [],
        "a link that should ask for a password was asked for without one"
    );
    assert!(
        matches!(&rt.state.actions.dialog, Some(Dialog::Share(share)) if share.password_missing),
        "the dialog stays and says a password is missing"
    );
}

#[test]
fn the_shells_poll_puts_a_landed_link_on_the_clipboard() {
    let (mut rt, spy, clipboard, _) = window();
    rt.update(|s, tx| assert!(s.copy_link(tx)));
    shell_frame(&mut rt);
    assert_eq!(copied(&clipboard), BEFORE);
    spy.answer(Action::CopyLink, Ok(LINK));
    shell_frame(&mut rt);
    assert_eq!(copied(&clipboard), LINK);
    assert_eq!(rt.state.actions.toast, Some(Toast::LinkCopied));
}

#[test]
fn while_a_link_is_on_its_way_no_second_one_is_asked_for_and_its_answer_is_the_first_ones() {
    let (mut rt, spy, clipboard, ids) = window();
    rt.update(|s, tx| assert!(s.copy_link(tx)));
    assert_eq!(spy.asked(), [(Action::CopyLink, vec![ids[0].clone()])]);

    // Another file, while the first link is on its way: said, with no Try again, and the source is not asked.
    select_only(&mut rt, 1);
    rt.update(|s, tx| assert!(s.copy_link(tx)));
    assert_eq!(spy.asked(), [], "a second link was asked for");
    assert_eq!(
        rt.state.actions.toast,
        Some(Toast::Failed(Action::CopyLink, ActionError::InFlight))
    );
    assert!(!rt.state.offers_try_again(Action::CopyLink, ActionError::InFlight));
    // The Share dialog's link is not asked for either.
    rt.update(|s, tx| {
        assert!(s.begin_share());
        s.share_copy_link(tx);
    });
    assert_eq!(spy.asked(), [], "a second link was asked for");

    // The one answer there is, is the first press's: it cannot be taken for another file's.
    spy.answer(Action::CopyLink, Ok(LINK));
    assert!(frame(&mut rt));
    assert_eq!(copied(&clipboard), LINK);
    // Answered: the next press asks again.
    rt.update(|s, tx| assert!(s.copy_link(tx)));
    assert_eq!(spy.asked(), [(Action::CopyLink, vec![ids[1].clone()])]);
}

#[test]
fn a_press_whose_answer_was_dropped_does_not_keep_the_next_one_from_being_asked() {
    let (mut rt, spy, clipboard, ids) = window();
    rt.update(|s, tx| assert!(s.copy_link(tx)));
    spy.asked();
    // A lock and an unlock between two frames: the source dropped the answer, and nothing reset the window.
    *spy.dropped.lock().unwrap() = true;
    frame(&mut rt);
    assert_eq!(rt.state.actions.toast, None, "nothing is said of it");
    assert_eq!(copied(&clipboard), BEFORE);
    *spy.dropped.lock().unwrap() = false;
    rt.update(|s, tx| assert!(s.copy_link(tx)));
    assert_eq!(
        spy.asked(),
        [(Action::CopyLink, vec![ids[0].clone()])],
        "the next press was refused as still on its way"
    );
    // The same for the dialog's link.
    spy.answer(Action::CopyLink, Ok(LINK));
    frame(&mut rt);
    rt.update(|s, tx| {
        assert!(s.begin_share());
        s.share_copy_link(tx);
    });
    spy.asked();
    *spy.dropped.lock().unwrap() = true;
    frame(&mut rt);
    *spy.dropped.lock().unwrap() = false;
    rt.update(|s, tx| assert!(s.copy_link(tx)));
    assert_eq!(spy.asked(), [(Action::CopyLink, vec![ids[0].clone()])]);
}

#[test]
fn a_failure_with_nothing_held_and_a_message_that_went_leave_nothing_to_try_again() {
    let (mut rt, spy, _, _) = window();
    // An answer nobody here pressed for (nothing is held of its press): Try again is not "the selection".
    spy.answer(Action::CopyLink, Err(ActionError::Failed));
    assert!(frame(&mut rt));
    assert_eq!(
        rt.state.actions.toast,
        Some(Toast::Failed(Action::CopyLink, ActionError::Failed))
    );
    assert!(!rt.state.offers_try_again(Action::CopyLink, ActionError::Failed));
    rt.update(|s, tx| s.retry(tx));
    assert_eq!(spy.asked(), [], "Try again asked for the selection");

    // A message whose time is over takes what its Try again held with it.
    rt.update(|s, tx| assert!(s.copy_link(tx)));
    spy.asked();
    spy.answer(Action::CopyLink, Err(ActionError::Offline));
    assert!(frame(&mut rt));
    assert!(rt.state.actions.link.again.is_some());
    rt.update(|s, _| {
        s.actions.toast_until = Some(std::time::Instant::now() - Duration::from_secs(1));
        assert!(super::super::reset::poll(s));
    });
    assert_eq!(rt.state.actions.link.again, None);
}

#[test]
fn a_link_that_was_not_made_is_said_and_try_again_asks_for_the_same_item() {
    let (mut rt, spy, clipboard, ids) = window();
    rt.update(|s, tx| assert!(s.copy_link(tx)));
    spy.asked();
    spy.answer(Action::CopyLink, Err(ActionError::Offline));
    assert!(frame(&mut rt));
    assert_eq!(
        rt.state.actions.toast,
        Some(Toast::Failed(Action::CopyLink, ActionError::Offline))
    );
    assert_eq!(copied(&clipboard), BEFORE, "nothing was copied");
    assert!(rt.state.offers_try_again(Action::CopyLink, ActionError::Offline));

    // Another row is selected by now: Try again is for the item of the press.
    select_only(&mut rt, 1);
    rt.update(|s, tx| s.retry(tx));
    assert_eq!(spy.asked(), [(Action::CopyLink, vec![ids[0].clone()])]);
    spy.answer(Action::CopyLink, Ok(LINK));
    assert!(frame(&mut rt));
    assert_eq!(copied(&clipboard), LINK);
}

#[test]
fn what_asking_again_cannot_change_is_said_in_its_own_words_without_try_again() {
    let (mut rt, spy, clipboard, _) = window();
    for (why, words) in [
        (ActionError::HasLink, "already has a link"),
        (ActionError::NotYet, "not available yet"),
    ] {
        rt.update(|s, tx| assert!(s.copy_link(tx)));
        spy.answer(Action::CopyLink, Err(why));
        assert!(frame(&mut rt));
        assert_eq!(rt.state.actions.toast, Some(Toast::Failed(Action::CopyLink, why)));
        assert!(Toast::failure(Action::CopyLink, why).contains(words));
        assert!(!rt.state.offers_try_again(Action::CopyLink, why), "{why:?}");
        // A key that reaches Try again all the same asks for nothing.
        spy.asked();
        rt.update(|s, tx| s.retry(tx));
        assert_eq!(spy.asked(), []);
    }
    assert_eq!(copied(&clipboard), BEFORE);
}

#[test]
fn a_refusal_at_the_press_is_said_and_one_still_on_its_way_offers_no_try_again() {
    let (mut rt, spy, _, ids) = window();
    // Refused at once (no connection is known at the press): Try again asks for the same item.
    *spy.refuse.lock().unwrap() = Some(ActionError::Offline);
    rt.update(|s, tx| assert!(s.copy_link(tx)));
    assert_eq!(
        rt.state.actions.toast,
        Some(Toast::Failed(Action::CopyLink, ActionError::Offline))
    );
    select_only(&mut rt, 1);
    assert!(rt.state.offers_try_again(Action::CopyLink, ActionError::Offline));
    rt.update(|s, tx| s.retry(tx));
    assert_eq!(spy.asked(), [(Action::CopyLink, vec![ids[0].clone()])]);

    // The first press is still on its way: its answer is the link, there is nothing to ask again.
    *spy.refuse.lock().unwrap() = Some(ActionError::InFlight);
    rt.update(|s, tx| assert!(s.copy_link(tx)));
    assert_eq!(
        rt.state.actions.toast,
        Some(Toast::Failed(Action::CopyLink, ActionError::InFlight))
    );
    assert!(!rt.state.offers_try_again(Action::CopyLink, ActionError::InFlight));
}

#[test]
fn a_link_that_landed_for_a_session_that_ended_never_reaches_the_clipboard() {
    let (mut rt, spy, clipboard, _) = window();
    rt.update(|s, tx| assert!(s.copy_link(tx)));
    spy.answer(Action::CopyLink, Ok(LINK));
    // The source's answer is taken in, and the session ends before this frame's clipboard step.
    rt.update(|s, tx| {
        s.take_changes();
        assert_eq!(s.actions.link.landed.len(), 1);
        super::super::reset::end_session(s, true);
        assert!(!poll(s, tx));
    });
    settle(&mut rt);
    assert_eq!(
        copied(&clipboard),
        BEFORE,
        "a link of the session that ended was copied"
    );
    assert_eq!(rt.state.actions.toast, None);
    assert_eq!(rt.state.actions.link.asked, None);
}

#[test]
fn the_dialogs_link_waits_for_its_answer_too_and_a_failure_opens_the_dialog_again() {
    let (mut rt, spy, clipboard, ids) = window();
    rt.update(|s, tx| {
        assert!(s.begin_share());
        s.share_copy_link(tx);
    });
    assert_eq!(spy.asked(), [(Action::ShareLink, vec![ids[0].clone()])]);
    assert!(rt.state.actions.dialog.is_none(), "the dialog closed");
    assert!(!frame(&mut rt));
    assert_eq!(rt.state.actions.toast, None, "said before there was a link");
    assert_eq!(copied(&clipboard), BEFORE);

    spy.answer(Action::ShareLink, Err(ActionError::Failed));
    assert!(frame(&mut rt));
    assert_eq!(
        rt.state.actions.toast,
        Some(Toast::Failed(Action::ShareLink, ActionError::Failed))
    );
    // Try again opens the dialog on the same item; no link is asked for blind.
    rt.update(|s, tx| s.retry(tx));
    assert!(rt.state.actions.dialog.is_some(), "the dialog is open again");
    assert_eq!(spy.asked(), []);

    rt.update(|s, tx| s.share_copy_link(tx));
    spy.answer(Action::ShareLink, Ok(LINK));
    assert!(frame(&mut rt));
    assert_eq!(copied(&clipboard), LINK);
    assert_eq!(rt.state.actions.toast, Some(Toast::LinkCopied));
    assert!(rt.state.actions.retry_share.is_none() && rt.state.actions.link.share.is_none());
}
