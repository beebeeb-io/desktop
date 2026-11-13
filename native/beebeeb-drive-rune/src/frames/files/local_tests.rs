//! A source that answers later (as the engine's does), and what the window draws and says of its answers: per
//! item, from what the engine reports, never before it and never from the action; Try again for exactly the
//! items that were left; a message between presses with its own wake. Every id and name is the board's.

use super::super::Toast;
use super::super::protected::poll;
use super::super::source::{Account, Counts, Details, FilesSource, LocalOutcome, NoEngine, StorageSplit};
use super::super::tests::{find, runtime_of, settle};
use super::*;
use crate::source::DriveItem;
use rune::Runtime;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

type R = Result<(), ActionError>;

/// What the test keeps of the source: what it was asked, what it will answer, when it was asked to wake.
#[derive(Default, Clone)]
struct Spy {
    later: Arc<Mutex<Vec<Change>>>,
    /// Each change of what this PC holds that was asked: the direction and the ids.
    asked: Arc<Mutex<Vec<(Action, Vec<String>)>>>,
    /// The next change of what this PC holds is refused at once with this.
    refuse: Arc<Mutex<Option<ActionError>>>,
    wakes: Arc<Mutex<Vec<Duration>>>,
    uploads: Arc<Mutex<Vec<Vec<PathBuf>>>>,
}

impl Spy {
    fn asked(&self) -> Vec<(Action, Vec<String>)> {
        self.asked.lock().unwrap().clone()
    }
    fn answer(&self, action: Action, items: &[(&str, Option<&str>, Option<ActionError>)]) {
        let items = items
            .iter()
            .map(|(id, now, left)| LocalOutcome {
                id: id.to_string(),
                now: Some(now.map(str::to_string)),
                left: *left,
            })
            .collect();
        self.later.lock().unwrap().push(Change::Local { action, items });
    }
    fn local(&self, action: Action, items: &[DriveItem]) -> R {
        if let Some(why) = self.refuse.lock().unwrap().take() {
            return Err(why);
        }
        let ids = items.iter().map(|item| item.id.clone()).collect();
        self.asked.lock().unwrap().push((action, ids));
        Ok(())
    }
}

/// A source that answers later: every action is accepted, and what it did is what the test puts in `later`.
/// An upload is refused when `left` says what was not copied.
struct Later {
    spy: Spy,
    left: Option<Vec<PathBuf>>,
}

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
    fn keep_offline(&mut self, items: &[DriveItem], keep: bool) -> R {
        self.spy.local(if keep { Action::KeepOffline } else { Action::RemoveOffline }, items)
    }
    fn copy_link(&mut self, _: &[DriveItem]) -> R { Ok(()) }
    fn open(&mut self, _: &[DriveItem]) -> R { Ok(()) }
    fn show_in_file_explorer(&mut self, _: &[DriveItem]) -> R { Ok(()) }
    fn upload(&mut self, _: Option<&str>, files: &[PathBuf]) -> R {
        self.spy.uploads.lock().unwrap().push(files.to_vec());
        if self.left.is_some() { Err(ActionError::Failed) } else { Ok(()) }
    }
    fn upload_left(&mut self) -> Option<Vec<PathBuf>> { self.left.take() }
    fn new_folder(&mut self, _: Option<&str>, _: &str) -> R { Ok(()) }
    fn rename(&mut self, _: &DriveItem, _: &str) -> R { Ok(()) }
    fn move_to(&mut self, _: &[DriveItem], _: Option<&str>) -> R { Ok(()) }
    fn free_up_space(&mut self, items: &[DriveItem]) -> R { self.spy.local(Action::FreeUpSpace, items) }
    fn open_in_file_explorer(&mut self, _: Option<&str>) -> R { Ok(()) }
    fn changes(&mut self) -> Vec<Change> { std::mem::take(&mut self.spy.later.lock().unwrap()) }
    fn answers_later(&self) -> bool { true }
    fn wake_after(&mut self, after: Duration) { self.spy.wakes.lock().unwrap().push(after); }
}

/// The window over such a source, with `rows` files that can change (not Backups) first in the list, each
/// with this mark, all selected. Their ids, and the id of a row that is NOT selected.
fn window(left: Option<Vec<PathBuf>>, rows: usize, mark: &str) -> (Runtime<State>, Spy, Vec<String>, String) {
    let spy = Spy::default();
    let mut state = crate::init();
    state.actions.source = Box::new(Later { spy: spy.clone(), left });
    let free: Vec<usize> = (0..state.listing.items.len())
        .filter(|&row| !state.listing.items[row].is_protected && !state.listing.items[row].is_folder)
        .take(rows + 1)
        .collect();
    assert_eq!(free.len(), rows + 1, "the board has too few plain files for this test");
    for (to, from) in free.into_iter().enumerate() {
        state.listing.items.swap(to, from);
    }
    state.selected = Some(0);
    for row in 0..rows {
        state.listing.items[row].pc_status = Some(mark.into());
        state.actions.selection.insert(row);
    }
    state.listing.items[rows].pc_status = Some("available_offline".into());
    let ids = state.listing.items[..rows].iter().map(|item| item.id.clone()).collect();
    let other = state.listing.items[rows].id.clone();
    (runtime_of(state), spy, ids, other)
}

fn marks(rt: &Runtime<State>, rows: usize) -> Vec<Option<String>> {
    rt.state.listing.items[..rows]
        .iter()
        .map(|item| item.pc_status.clone())
        .collect()
}

fn mark(mark: &str) -> Option<String> {
    Some(mark.to_string())
}

/// One frame's look at the source.
fn frame(rt: &mut Runtime<State>) -> bool {
    let mut changed = false;
    rt.update(|s, _| changed = poll(s));
    settle(rt);
    changed
}

const FAILED: Option<ActionError> = Some(ActionError::Failed);

#[test]
fn nothing_of_a_change_is_drawn_before_the_source_said_what_it_did_and_then_per_item() {
    let (mut rt, spy, ids, _) = window(None, 2, "on_this_pc");
    rt.update(|s, tx| assert!(s.free_up_space(tx)));
    assert_eq!(spy.asked(), [(Action::FreeUpSpace, ids.clone())]);
    assert!(!frame(&mut rt), "a frame with no answer changed something");
    assert_eq!(
        marks(&rt, 2),
        [mark("on_this_pc"), mark("on_this_pc")],
        "drawn before it was done"
    );
    assert_eq!(rt.state.actions.toast, None, "said before it was done");
    // The engine freed the first and left the second.
    spy.answer(
        Action::FreeUpSpace,
        &[
            (&ids[0], Some("online_only"), None),
            (&ids[1], Some("on_this_pc"), FAILED),
        ],
    );
    assert!(frame(&mut rt));
    assert_eq!(marks(&rt, 2), [mark("online_only"), mark("on_this_pc")]);
    assert_eq!(
        rt.state.actions.toast,
        Some(Toast::Failed(Action::FreeUpSpace, ActionError::NotAll))
    );
    // Try again asks for the one that was left, alone.
    assert!(find(&rt, "toast-action").is_some());
    rt.update(|s, tx| s.retry(tx));
    assert_eq!(spy.asked().last(), Some(&(Action::FreeUpSpace, vec![ids[1].clone()])));
    // All of it done: the plain message, woken by the source after its own 3 seconds.
    spy.answer(Action::FreeUpSpace, &[(&ids[1], Some("online_only"), None)]);
    assert!(frame(&mut rt));
    assert_eq!(marks(&rt, 2), [mark("online_only"), mark("online_only")]);
    assert_eq!(rt.state.actions.toast, Some(Toast::FreedUp));
    assert_eq!(
        *spy.wakes.lock().unwrap(),
        [Duration::from_secs(8), Duration::from_secs(3)],
        "each message between presses is woken at its own time"
    );
}

#[test]
fn a_row_reads_what_the_engine_says_of_it_not_what_the_action_suggests() {
    // Remove from this PC only stops keeping: the copy is still here, so Free up space is still offered.
    let (mut rt, spy, ids, _) = window(None, 1, "available_offline");
    rt.update(|s, tx| assert!(s.toggle_keep_offline(tx)));
    assert_eq!(spy.asked(), [(Action::RemoveOffline, ids.clone())]);
    spy.answer(Action::RemoveOffline, &[(&ids[0], Some("on_this_pc"), None)]);
    assert!(frame(&mut rt));
    assert_eq!(marks(&rt, 1), [mark("on_this_pc")]);
    assert!(
        rt.state.selection_is_on_this_pc(),
        "Free up space is gone for bytes that are still here"
    );
    assert_eq!(rt.state.actions.toast, None, "Remove from this PC has no message");
    // Keep on this PC of a file that is not downloaded yet: the engine says "online only" until it is, and so
    // does the row. An item the engine left reads what the engine says too (kept again after a failed reclaim).
    rt.update(|s, tx| assert!(s.toggle_keep_offline(tx)));
    spy.answer(Action::KeepOffline, &[(&ids[0], Some("online_only"), None)]);
    assert!(frame(&mut rt));
    assert_eq!(marks(&rt, 1), [mark("online_only")]);
    assert_eq!(rt.state.actions.toast, Some(Toast::Keeping));
    spy.answer(Action::FreeUpSpace, &[(&ids[0], Some("available_offline"), FAILED)]);
    assert!(frame(&mut rt));
    assert_eq!(marks(&rt, 1), [mark("available_offline")]);
    // A folder that is no longer kept has no mark; an item the engine could not read stays as drawn.
    spy.answer(Action::RemoveOffline, &[(&ids[0], None, None)]);
    frame(&mut rt);
    assert_eq!(marks(&rt, 1), [None]);
    rt.update(|s, _| s.listing.items[0].pc_status = mark("on_this_pc"));
    spy.later.lock().unwrap().push(Change::Local {
        action: Action::KeepOffline,
        items: vec![LocalOutcome {
            id: ids[0].clone(),
            now: None,
            left: FAILED,
        }],
    });
    frame(&mut rt);
    assert_eq!(marks(&rt, 1), [mark("on_this_pc")]);
}

#[test]
fn try_again_asks_for_the_items_that_were_left_in_the_same_direction() {
    // Three kept rows, Remove from this PC, the engine changes two.
    let (mut rt, spy, ids, other) = window(None, 3, "available_offline");
    rt.update(|s, tx| assert!(s.toggle_keep_offline(tx)));
    spy.answer(
        Action::RemoveOffline,
        &[
            (&ids[0], Some("on_this_pc"), None),
            (&ids[1], Some("on_this_pc"), None),
            (&ids[2], Some("available_offline"), FAILED),
        ],
    );
    assert!(frame(&mut rt));
    assert_eq!(
        rt.state.actions.toast,
        Some(Toast::Failed(Action::RemoveOffline, ActionError::NotAll))
    );
    // The selection moves to another row (a kept one) before Try again is pressed.
    rt.update(|s, _| {
        s.actions.selection.clear();
        s.actions.selection.insert(3);
        s.selected = Some(3);
        assert_eq!(s.listing.items[3].id, other);
    });
    settle(&mut rt);
    assert!(find(&rt, "toast-action").is_some());
    rt.update(|s, tx| s.retry(tx));
    assert_eq!(
        spy.asked(),
        [
            (Action::RemoveOffline, ids.clone()),
            (Action::RemoveOffline, vec![ids[2].clone()])
        ],
        "Try again kept everything again, or changed what is selected now"
    );
    // A refusal at once holds its items the same way: asked again for them, whatever is selected then.
    *spy.refuse.lock().unwrap() = Some(ActionError::InFlight);
    rt.update(|s, tx| assert!(s.toggle_keep_offline(tx)));
    assert_eq!(
        rt.state.actions.toast,
        Some(Toast::Failed(Action::RemoveOffline, ActionError::InFlight))
    );
    rt.update(|s, tx| {
        s.actions.selection.clear();
        s.actions.selection.insert(0);
        s.selected = Some(0);
        s.retry(tx);
    });
    assert_eq!(spy.asked().last(), Some(&(Action::RemoveOffline, vec![other.clone()])));
    // The session ends: nothing of it is held for a Try again under the next one.
    rt.update(|s, _| {
        s.actions.local.retry = Some((Action::KeepOffline, vec![other.clone()]));
        s.actions.local.unsaid.push(Late::Done(Action::KeepOffline));
        super::super::reset::end_session(s, true);
        assert!(s.actions.local.retry.is_none() && s.actions.local.unsaid.is_empty());
    });
}

#[test]
fn an_answer_between_presses_does_not_take_another_messages_try_again_away() {
    let files: Vec<PathBuf> = ["one.pdf", "two.pdf"].iter().map(PathBuf::from).collect();
    let (mut rt, spy, ids, _) = window(Some(files[1..].to_vec()), 2, "on_this_pc");
    rt.update(|s, tx| {
        assert!(s.toggle_keep_offline(tx));
        assert!(!s.upload(None, &files, tx));
    });
    let upload_failed = Some(Toast::Failed(Action::Upload, ActionError::Failed));
    assert_eq!(rt.state.actions.toast, upload_failed);
    // Keep is done for both while "Couldn't upload" still offers Try again: the rows change, the message stays.
    spy.answer(
        Action::KeepOffline,
        &[
            (&ids[0], Some("available_offline"), None),
            (&ids[1], Some("available_offline"), None),
        ],
    );
    assert!(frame(&mut rt));
    assert_eq!(marks(&rt, 2), [mark("available_offline"), mark("available_offline")]);
    assert_eq!(
        rt.state.actions.toast, upload_failed,
        "a plain message took the failure's place"
    );
    assert!(rt.state.actions.retry_upload.is_some() && find(&rt, "toast-action").is_some());
    // A failure that comes now waits for it.
    spy.answer(Action::RemoveOffline, &[(&ids[0], Some("available_offline"), FAILED)]);
    frame(&mut rt);
    assert_eq!(rt.state.actions.toast, upload_failed);
    assert!(spy.wakes.lock().unwrap().is_empty());
    // The upload's message went (its time is over): the failure is said, with its own items and its own wake.
    rt.update(|s, _| {
        s.actions.toast_until = Some(std::time::Instant::now());
        assert!(super::super::reset::poll(s));
    });
    assert!(frame(&mut rt));
    assert_eq!(
        rt.state.actions.toast,
        Some(Toast::Failed(Action::RemoveOffline, ActionError::Failed))
    );
    assert_eq!(
        rt.state.actions.local.retry,
        Some((Action::RemoveOffline, vec![ids[0].clone()]))
    );
    assert_eq!(*spy.wakes.lock().unwrap(), [Duration::from_secs(8)]);
}

#[test]
fn try_again_is_not_drawn_where_it_would_repeat_nothing() {
    // Every file of the drop is in, and the answer was still a refusal: nothing is left to send.
    let files: Vec<PathBuf> = ["one.pdf", "two.pdf"].iter().map(PathBuf::from).collect();
    let (mut rt, spy, ids, _) = window(Some(Vec::new()), 1, "on_this_pc");
    rt.update(|s, tx| assert!(!s.upload(None, &files, tx)));
    settle(&mut rt);
    assert_eq!(
        rt.state.live_toast(),
        Some(Toast::Failed(Action::Upload, ActionError::Failed))
    );
    assert!(
        find(&rt, "toast-action").is_none(),
        "a Try again that would send nothing"
    );
    rt.update(|s, tx| s.retry(tx));
    assert_eq!(
        spy.uploads.lock().unwrap().len(),
        1,
        "every file was sent a second time"
    );
    // What this system cannot do at all: said in its own words, and nothing to try again.
    *spy.refuse.lock().unwrap() = Some(ActionError::NotOnThisSystem);
    rt.update(|s, tx| assert!(s.free_up_space(tx)));
    settle(&mut rt);
    assert_eq!(
        rt.state.live_toast(),
        Some(Toast::Failed(Action::FreeUpSpace, ActionError::NotOnThisSystem))
    );
    assert_eq!(
        Toast::failure(Action::FreeUpSpace, ActionError::NotOnThisSystem),
        "Couldn't free up space. It is not available on this computer yet."
    );
    assert!(find(&rt, "toast-action").is_none());
    rt.update(|s, tx| s.retry(tx));
    assert!(spy.asked().is_empty(), "asked again for what cannot be done here");
    // A late failure for a row that is no longer shown: said, with nothing to ask again.
    spy.answer(Action::KeepOffline, &[("an-id-no-row-has", Some("on_this_pc"), FAILED)]);
    assert!(frame(&mut rt));
    assert_eq!(
        rt.state.live_toast(),
        Some(Toast::Failed(Action::KeepOffline, ActionError::Failed))
    );
    assert!(find(&rt, "toast-action").is_none());
    // And one for a row that is shown offers it.
    spy.answer(Action::KeepOffline, &[(&ids[0], Some("on_this_pc"), FAILED)]);
    assert!(frame(&mut rt));
    assert!(find(&rt, "toast-action").is_some());
}

#[test]
fn a_refusal_of_a_protected_item_that_comes_later_takes_the_path_of_one_at_once_and_is_never_silent() {
    use super::super::Intent;
    // At once: the frame that says why is asked for (DW-1110). This source has no words of its own.
    let (mut rt, spy, _, _) = window(None, 1, "on_this_pc");
    *spy.refuse.lock().unwrap() = Some(ActionError::Protected);
    rt.update(|s, tx| assert!(s.toggle_keep_offline(tx)));
    assert_eq!(rt.state.actions.intents.last(), Some(&Intent::Frame("DW-1110")));
    // An item the window itself knows as protected, kept: the switch says "keep" (as it always did), and
    // nothing is held for a Try again in either direction: asking again changes nothing.
    let (mut rt, spy, ids, _) = window(None, 1, "available_offline");
    rt.update(|s, tx| {
        s.listing.items[0].is_protected = true;
        assert!(s.toggle_keep_offline(tx));
        assert_eq!(s.actions.local.retry, Some((Action::KeepOffline, Vec::new())));
        assert!(!s.offers_try_again(Action::KeepOffline, ActionError::Protected));
    });
    assert!(
        spy.asked().is_empty(),
        "the source was asked for a protected item: {ids:?}"
    );
    // Later: accepted, and the engine found the item in the Backups tree. The same frame is asked for, and
    // since nothing else says it after "accepted", the plain message does. Asking again changes nothing.
    let (mut rt, spy, ids, _) = window(None, 2, "on_this_pc");
    rt.update(|s, tx| assert!(s.toggle_keep_offline(tx)));
    assert_eq!(rt.state.actions.intents.last(), None);
    spy.answer(
        Action::KeepOffline,
        &[
            (&ids[0], Some("on_this_pc"), Some(ActionError::Protected)),
            (&ids[1], Some("on_this_pc"), Some(ActionError::Protected)),
        ],
    );
    assert!(frame(&mut rt));
    assert_eq!(rt.state.actions.intents.last(), Some(&Intent::Frame("DW-1110")));
    assert_eq!(
        rt.state.live_toast(),
        Some(Toast::Failed(Action::KeepOffline, ActionError::Protected))
    );
    assert!(
        find(&rt, "toast-action").is_none(),
        "Try again for an item that cannot change"
    );
    assert_eq!(*spy.wakes.lock().unwrap(), [Duration::from_secs(8)]);
}

#[test]
fn what_free_up_space_left_is_said_with_the_engines_reason() {
    // A file with a change that is not uploaded: said, and worth asking again later.
    let (mut rt, spy, ids, _) = window(None, 2, "on_this_pc");
    rt.update(|s, tx| assert!(s.free_up_space(tx)));
    spy.answer(
        Action::FreeUpSpace,
        &[
            (&ids[0], Some("on_this_pc"), Some(ActionError::NotSent)),
            (&ids[1], Some("on_this_pc"), Some(ActionError::NotSent)),
        ],
    );
    assert!(frame(&mut rt));
    assert_eq!(
        rt.state.live_toast(),
        Some(Toast::Failed(Action::FreeUpSpace, ActionError::NotSent))
    );
    assert_eq!(
        Toast::failure(Action::FreeUpSpace, ActionError::NotSent),
        "Couldn't free up space. It has changes that are not uploaded yet."
    );
    assert!(find(&rt, "toast-action").is_some());
    rt.update(|s, tx| s.retry(tx));
    assert_eq!(spy.asked().last(), Some(&(Action::FreeUpSpace, ids.clone())));
    // A folder that holds something kept: said, and no Try again (asking again changes nothing).
    spy.answer(
        Action::FreeUpSpace,
        &[
            (&ids[0], None, Some(ActionError::HoldsKept)),
            (&ids[1], Some("online_only"), None),
        ],
    );
    assert!(frame(&mut rt));
    assert_eq!(marks(&rt, 2), [None, mark("online_only")]);
    assert_eq!(
        rt.state.live_toast(),
        Some(Toast::Failed(Action::FreeUpSpace, ActionError::NotAll))
    );
    assert!(
        find(&rt, "toast-action").is_none(),
        "Try again with nothing it could change"
    );
    rt.update(|s, _| {
        s.actions.toast_until = Some(std::time::Instant::now());
        super::super::reset::poll(s);
    });
    spy.answer(Action::FreeUpSpace, &[(&ids[0], None, Some(ActionError::HoldsKept))]);
    assert!(frame(&mut rt));
    assert_eq!(
        rt.state.live_toast(),
        Some(Toast::Failed(Action::FreeUpSpace, ActionError::HoldsKept))
    );
    assert_eq!(
        Toast::failure(Action::FreeUpSpace, ActionError::HoldsKept),
        "Couldn't free up space. Something in it is kept on this PC."
    );
    assert!(find(&rt, "toast-action").is_none());
}

#[test]
fn an_answer_drained_by_another_caller_is_still_said() {
    // A dialog that closes takes the changes (`State::take_changes`) and has no words for this one.
    let (mut rt, spy, ids, _) = window(None, 1, "on_this_pc");
    rt.update(|s, tx| assert!(s.toggle_keep_offline(tx)));
    spy.answer(Action::KeepOffline, &[(&ids[0], Some("available_offline"), None)]);
    rt.update(|s, _| {
        assert_eq!(s.take_changes(), (false, Vec::new()));
        assert_eq!(s.actions.toast, None);
    });
    assert_eq!(marks(&rt, 1), [mark("available_offline")]);
    assert!(frame(&mut rt));
    assert_eq!(rt.state.actions.toast, Some(Toast::Keeping));
}

#[test]
fn try_again_after_a_drop_that_went_in_partly_holds_only_what_was_not_copied() {
    let files: Vec<PathBuf> = ["one.pdf", "two.pdf", "three.pdf"].iter().map(PathBuf::from).collect();
    let (mut rt, spy, _, _) = window(Some(files[1..].to_vec()), 1, "on_this_pc");
    rt.update(|s, tx| {
        assert!(!s.upload(None, &files, tx));
        assert_eq!(
            s.actions.retry_upload,
            Some((None, files[1..].to_vec())),
            "Try again would copy a file twice"
        );
        // Try again sends those alone, and it goes in.
        s.retry(tx);
        assert_eq!(s.actions.retry_upload, None);
    });
    assert_eq!(*spy.uploads.lock().unwrap(), [files.clone(), files[1..].to_vec()]);
}
