//! Move to Trash and Undo over a source that answers later (as the engine's does): nothing is drawn at the press,
//! rows leave and come back per item when the source said what Beebeeb did, by id; Try again asks for exactly the
//! items that were left; nothing of an earlier session is put back. Every id and name is the board's.

use super::super::Toast;
use super::super::protected::poll;
use super::super::source::{
    Account, Action, ActionError, Change, Counts, Details, FilesSource, NoEngine, StorageSplit, TrashOutcome,
};
use super::super::tests::{runtime_of, settle};
use crate::State;
use crate::source::DriveItem;
use rune::Runtime;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

type R = Result<(), ActionError>;

/// What the test keeps of the source: what it was asked, and what it will answer.
type Asked = Arc<Mutex<Vec<(Action, Vec<String>)>>>;

#[derive(Default, Clone)]
struct Spy {
    later: Arc<Mutex<Vec<Change>>>,
    asked: Asked,
    /// The next move to Trash is refused at once with this.
    refuse: Arc<Mutex<Option<ActionError>>>,
}

impl Spy {
    fn asked(&self) -> Vec<(Action, Vec<String>)> {
        std::mem::take(&mut self.asked.lock().unwrap())
    }
    fn answer(&self, action: Action, items: &[(&str, Option<ActionError>)]) {
        let items = items
            .iter()
            .map(|(id, left)| TrashOutcome {
                id: id.to_string(),
                left: *left,
            })
            .collect();
        self.later.lock().unwrap().push(Change::Trash { action, items });
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

/// A source that answers later: a move to Trash and an Undo are accepted, and what they did is what the test says.
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
    fn trash(&mut self, items: &[DriveItem]) -> R { self.0.ask(Action::Trash, items) }
    fn restore(&mut self, items: &[DriveItem]) -> R { self.0.ask(Action::Restore, items) }
    fn duplicate(&mut self, _: &[DriveItem], _: &[&str]) -> R { Ok(()) }
    fn keep_offline(&mut self, _: &[DriveItem], _: bool) -> R { Ok(()) }
    fn copy_link(&mut self, _: &[DriveItem]) -> R { Ok(()) }
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
    fn wake_after(&mut self, _: Duration) {}
}

/// The window over such a source, two plain files first in the list, both selected. Their ids, and the id of the
/// third row (not selected).
fn window() -> (Runtime<State>, Spy, Vec<String>, String) {
    let spy = Spy::default();
    let mut state = crate::init();
    state.actions.source = Box::new(Later(spy.clone()));
    let free: Vec<usize> = (0..state.listing.items.len())
        .filter(|&row| !state.listing.items[row].is_protected && !state.listing.items[row].is_folder)
        .take(3)
        .collect();
    assert_eq!(free.len(), 3, "the board has too few plain files for this test");
    for (to, from) in free.into_iter().enumerate() {
        state.listing.items.swap(to, from);
    }
    state.selected = Some(0);
    state.actions.selection.extend([0, 1]);
    let ids = state.listing.items[..2].iter().map(|item| item.id.clone()).collect();
    let other = state.listing.items[2].id.clone();
    (runtime_of(state), spy, ids, other)
}

fn shown(rt: &Runtime<State>, id: &str) -> bool {
    rt.state.listing.items.iter().any(|item| item.id == id)
}

/// One frame's look at the source.
fn frame(rt: &mut Runtime<State>) -> bool {
    let mut changed = false;
    rt.update(|s, _| changed = poll(s));
    settle(rt);
    changed
}

#[test]
fn no_row_leaves_before_the_source_said_so_and_then_only_the_ones_beebeeb_moved() {
    let (mut rt, spy, ids, other) = window();
    let rows = rt.state.listing.items.len();
    rt.update(|s, tx| assert!(s.trash_selection(tx)));
    assert_eq!(spy.asked(), [(Action::Trash, ids.clone())]);
    assert!(!frame(&mut rt), "a frame with no answer changed something");
    assert_eq!(rt.state.listing.items.len(), rows, "a row left before Beebeeb moved it");
    assert_eq!(rt.state.actions.toast, None, "said before it was done");
    assert!(rt.state.actions.undo.is_empty(), "nothing to undo yet");

    // The rows are in other places when the answer lands: found by id.
    rt.update(|s, _| s.listing.items.swap(0, 2));
    // Beebeeb moved the first and refused the second.
    spy.answer(Action::Trash, &[(&ids[0], None), (&ids[1], Some(ActionError::Failed))]);
    assert!(frame(&mut rt));
    assert!(!shown(&rt, &ids[0]), "the row Beebeeb moved is gone");
    assert!(shown(&rt, &ids[1]) && shown(&rt, &other), "every other row stays");
    assert_eq!(rt.state.listing.items.len(), rows - 1);
    assert_eq!(
        rt.state.actions.toast,
        Some(Toast::Failed(Action::Trash, ActionError::NotAll))
    );
    let held: Vec<&str> = rt.state.actions.undo.iter().map(|(_, item)| item.id.as_str()).collect();
    assert_eq!(held, [ids[0].as_str()], "Undo is for the one that was moved");

    // Try again asks for the one that was left, whatever is selected now.
    rt.update(|s, tx| {
        let at = s.listing.items.iter().position(|item| item.id == other).unwrap();
        s.select_only(at);
        s.retry(tx);
    });
    assert_eq!(spy.asked(), [(Action::Trash, vec![ids[1].clone()])]);
}

#[test]
fn moved_is_said_with_undo_and_undo_puts_back_only_when_the_source_said_so() {
    let (mut rt, spy, ids, _) = window();
    let rows = rt.state.listing.items.len();
    rt.update(|s, tx| assert!(s.trash_selection(tx)));
    spy.answer(Action::Trash, &[(&ids[0], None), (&ids[1], None)]);
    assert!(frame(&mut rt));
    assert_eq!(rt.state.listing.items.len(), rows - 2);
    assert_eq!(rt.state.actions.toast, Some(Toast::Trashed), "said once it was done");
    assert_eq!(spy.asked().len(), 1);

    rt.update(|s, tx| assert!(s.undo(tx)));
    assert_eq!(spy.asked(), [(Action::Restore, ids.clone())]);
    assert!(!frame(&mut rt));
    assert_eq!(
        rt.state.listing.items.len(),
        rows - 2,
        "a row came back before Beebeeb put it back"
    );
    // A second Undo while the first is on its way asks nothing.
    rt.update(|s, tx| assert!(!s.undo(tx)));
    assert_eq!(spy.asked(), []);

    // Beebeeb put the first back and not the second.
    spy.answer(
        Action::Restore,
        &[(&ids[0], None), (&ids[1], Some(ActionError::Offline))],
    );
    assert!(frame(&mut rt));
    assert!(shown(&rt, &ids[0]) && !shown(&rt, &ids[1]));
    assert_eq!(
        rt.state.actions.toast,
        Some(Toast::Failed(Action::Restore, ActionError::NotAll))
    );
    // Try again is Undo again, for the one that is still in Trash.
    rt.update(|s, tx| s.retry(tx));
    assert_eq!(spy.asked(), [(Action::Restore, vec![ids[1].clone()])]);
    spy.answer(Action::Restore, &[(&ids[1], None)]);
    assert!(frame(&mut rt));
    assert_eq!(rt.state.listing.items.len(), rows, "both are back");
    assert!(rt.state.actions.undo.is_empty() && rt.state.actions.trash.restoring.is_empty());
}

#[test]
fn a_move_refused_at_the_press_is_said_and_try_again_asks_for_the_same_items() {
    let (mut rt, spy, ids, other) = window();
    let rows = rt.state.listing.items.len();
    *spy.refuse.lock().unwrap() = Some(ActionError::Offline);
    rt.update(|s, tx| assert!(s.trash_selection(tx)));
    assert_eq!(
        rt.state.actions.toast,
        Some(Toast::Failed(Action::Trash, ActionError::Offline))
    );
    assert!(rt.state.offers_try_again(Action::Trash, ActionError::Offline));
    assert_eq!(rt.state.listing.items.len(), rows);
    rt.update(|s, tx| {
        let at = s.listing.items.iter().position(|item| item.id == other).unwrap();
        s.select_only(at);
        s.retry(tx);
    });
    assert_eq!(spy.asked(), [(Action::Trash, ids)], "not what is selected now");
}

#[test]
fn an_undo_answered_after_the_session_ended_puts_nothing_back() {
    let (mut rt, spy, ids, _) = window();
    rt.update(|s, tx| assert!(s.trash_selection(tx)));
    spy.answer(Action::Trash, &[(&ids[0], None), (&ids[1], None)]);
    assert!(frame(&mut rt));
    rt.update(|s, tx| assert!(s.undo(tx)));
    let rows = rt.state.listing.items.len();

    // The session ends under the window (a lock, a sign-out); a source that still answered is not believed.
    rt.update(|s, _| super::super::reset::end_session(s, true));
    spy.answer(Action::Restore, &[(&ids[0], None), (&ids[1], None)]);
    frame(&mut rt);
    assert!(
        !shown(&rt, &ids[0]) && !shown(&rt, &ids[1]),
        "a row of the earlier session"
    );
    assert_eq!(rt.state.listing.items.len(), rows);
    assert!(rt.state.actions.undo.is_empty(), "and nothing of it is Undo's");
}

#[test]
fn a_second_press_while_the_first_is_on_its_way_offers_no_try_again_and_the_answer_is_still_said() {
    let (mut rt, spy, ids, _) = window();
    rt.update(|s, tx| assert!(s.trash_selection(tx)));
    // The source refuses the second press: the first is on its way.
    *spy.refuse.lock().unwrap() = Some(ActionError::InFlight);
    rt.update(|s, tx| assert!(s.trash_selection(tx)));
    assert_eq!(
        rt.state.actions.toast,
        Some(Toast::Failed(Action::Trash, ActionError::InFlight))
    );
    assert!(
        !rt.state.offers_try_again(Action::Trash, ActionError::InFlight),
        "nothing to ask again: the first press's answer says what was moved"
    );
    spy.answer(Action::Trash, &[(&ids[0], None), (&ids[1], None)]);
    assert!(frame(&mut rt));
    assert_eq!(rt.state.actions.toast, Some(Toast::Trashed), "moved, with Undo");
}

#[test]
fn what_was_selected_since_the_press_stays_selected_when_the_answer_moves_the_rows() {
    let (mut rt, spy, ids, other) = window();
    rt.update(|s, tx| assert!(s.trash_selection(tx)));
    // Since the press: another row is selected, and its name is being typed.
    rt.update(|s, _| {
        let at = s.listing.items.iter().position(|item| item.id == other).unwrap();
        s.select_only(at);
        s.actions.rename = Some(super::super::rename::Rename {
            row: at,
            id: other.clone(),
        });
    });
    spy.answer(Action::Trash, &[(&ids[0], None), (&ids[1], None)]);
    assert!(frame(&mut rt));
    let at = rt.state.listing.items.iter().position(|item| item.id == other).unwrap();
    assert_eq!(at, 0, "the two rows above it left");
    let selected: Vec<String> = rt.state.selected_items().into_iter().map(|item| item.id).collect();
    assert_eq!(selected, std::slice::from_ref(&other), "the same item, at its new place");
    assert_eq!(rt.state.actions.rename.as_ref().map(|r| r.row), Some(0));

    // Undo: the rows come back above it, and it is still the one selected.
    rt.update(|s, tx| assert!(s.undo(tx)));
    spy.answer(Action::Restore, &[(&ids[0], None), (&ids[1], None)]);
    assert!(frame(&mut rt));
    let selected: Vec<String> = rt.state.selected_items().into_iter().map(|item| item.id).collect();
    assert_eq!(selected, std::slice::from_ref(&other));
    let at = rt.state.listing.items.iter().position(|item| item.id == other).unwrap();
    assert_eq!(rt.state.actions.rename.as_ref().map(|r| r.row), Some(at));
}

#[test]
fn try_again_is_not_offered_when_nothing_can_be_asked_again() {
    let (mut rt, spy, ids, _) = window();
    rt.update(|s, tx| assert!(s.trash_selection(tx)));
    // Beebeeb moved one and refuses the other for what it is.
    spy.answer(
        Action::Trash,
        &[(&ids[0], None), (&ids[1], Some(ActionError::Protected))],
    );
    assert!(frame(&mut rt));
    assert!(!rt.state.offers_try_again(Action::Trash, ActionError::NotAll));

    // An Undo that failed can be asked again while its rows are held, and not after.
    rt.update(|s, tx| assert!(s.undo(tx)));
    spy.answer(Action::Restore, &[(&ids[0], Some(ActionError::Failed))]);
    assert!(frame(&mut rt));
    assert_eq!(
        rt.state.actions.toast,
        Some(Toast::Failed(Action::Restore, ActionError::Failed))
    );
    assert!(rt.state.offers_try_again(Action::Restore, ActionError::Failed));
    rt.update(|s, _| s.forget_undo());
    assert!(!rt.state.offers_try_again(Action::Restore, ActionError::Failed));
}

/// Review of desktop#424: a row that leaves because an upload was cancelled moves the rows under it, and the
/// selection follows the ROWS (by id), never the drawn positions: an action on "the selection" lands on the same
/// files that were selected, never on their neighbours.
#[test]
fn a_cancelled_upload_leaves_the_selection_on_the_same_files() {
    let (mut rt, spy, ids, other) = window();
    // [other, A, B]: A and B selected (B last), `other` (not selected) is about to go.
    rt.update(|s, _| {
        s.listing.items.swap(0, 2);
        s.listing.items.swap(1, 2);
    });
    let order: Vec<String> = rt.state.listing.items[..3].iter().map(|item| item.id.clone()).collect();
    assert_eq!(order[0], other);
    let row_of = |rt: &Runtime<State>, id: &str| rt.state.listing.items.iter().position(|item| item.id == id);
    rt.update(|s, _| {
        s.selected = Some(2);
        s.actions.anchor = Some(1);
        s.actions.selection.clear();
        s.actions.selection.extend([1, 2]);
    });
    let (first, last) = (order[1].clone(), order[2].clone());
    spy.later.lock().unwrap().push(Change::Cancelled { id: other.clone() });
    frame(&mut rt);
    assert!(!shown(&rt, &other), "the cancelled upload's row is gone");
    let selected: Vec<&str> = rt
        .state
        .actions
        .selection
        .iter()
        .map(|&row| rt.state.listing.items[row].id.as_str())
        .collect();
    assert_eq!(
        selected,
        [first.as_str(), last.as_str()],
        "the same two files stay selected"
    );
    assert_eq!(
        rt.state.selected,
        row_of(&rt, &last),
        "the current row is still the same file"
    );
    assert_eq!(
        rt.state.actions.anchor,
        row_of(&rt, &first),
        "the anchor is still the same file"
    );
    let _ = ids;
}

/// The ids Undo holds now, in its order.
fn undo_ids(rt: &Runtime<State>) -> Vec<String> {
    rt.state.actions.undo.iter().map(|(_, item)| item.id.clone()).collect()
}

/// Select only the row with this id.
fn select(rt: &mut Runtime<State>, id: &str) {
    rt.update(|s, _| {
        let at = s.listing.items.iter().position(|item| item.id == id).unwrap();
        s.select_only(at);
    });
}

/// Review of 2468, L316: a second Delete after a move that failed takes the old failure away, with its Try again;
/// Try again of a failure acts on the items it was left for, never on what is selected now.
#[test]
fn second_delete_after_failed_move_clears_old_failure() {
    let (mut rt, spy, ids, other) = window();
    rt.update(|s, tx| assert!(s.trash_selection(tx)));
    spy.answer(
        Action::Trash,
        &[
            (&ids[0], Some(ActionError::Offline)),
            (&ids[1], Some(ActionError::Offline)),
        ],
    );
    assert!(frame(&mut rt));
    assert_eq!(
        rt.state.actions.toast,
        Some(Toast::Failed(Action::Trash, ActionError::Offline))
    );
    assert_eq!(spy.asked(), [(Action::Trash, ids.clone())]);

    // A second Delete, of another row, is accepted: the old failure and its Try again go.
    select(&mut rt, &other);
    rt.update(|s, tx| assert!(s.trash_selection(tx)));
    assert_eq!(spy.asked(), [(Action::Trash, vec![other.clone()])]);
    assert_eq!(rt.state.actions.toast, None, "the old failure still stands");
    assert_eq!(rt.state.actions.trash.again, None, "and its Try again with it");

    // That move fails too: its own failure, and Try again asks for that row alone, whatever is selected now.
    spy.answer(Action::Trash, &[(&other, Some(ActionError::Offline))]);
    assert!(frame(&mut rt));
    assert_eq!(
        rt.state.actions.toast,
        Some(Toast::Failed(Action::Trash, ActionError::Offline))
    );
    select(&mut rt, &ids[0]);
    rt.update(|s, tx| s.retry(tx));
    assert_eq!(spy.asked(), [(Action::Trash, vec![other.clone()])]);
}

/// Review of 2468, L317: Undo is for every row moved since the last Undo, not only the last answer's. A partial move
/// and its Try again are undone together; an Undo pressed between them puts back only what it held.
#[test]
fn undo_after_partial_move_and_retry_puts_back_both() {
    let (mut rt, spy, ids, _) = window();
    rt.update(|s, tx| assert!(s.trash_selection(tx)));
    spy.answer(Action::Trash, &[(&ids[0], None), (&ids[1], Some(ActionError::Failed))]);
    assert!(frame(&mut rt));
    rt.update(|s, tx| s.retry(tx));
    assert_eq!(
        spy.asked(),
        [(Action::Trash, ids.clone()), (Action::Trash, vec![ids[1].clone()])]
    );
    spy.answer(Action::Trash, &[(&ids[1], None)]);
    assert!(frame(&mut rt));
    assert_eq!(rt.state.actions.toast, Some(Toast::Trashed));
    assert_eq!(undo_ids(&rt), ids, "the first one moved is still Undo's");
    rt.update(|s, tx| assert!(s.undo(tx)));
    assert_eq!(spy.asked(), [(Action::Restore, ids.clone())], "Undo puts back both");
    spy.answer(Action::Restore, &[(&ids[0], None), (&ids[1], None)]);
    assert!(frame(&mut rt));
    assert!(shown(&rt, &ids[0]) && shown(&rt, &ids[1]));

    // Again, with Undo pressed while the Try again is on its way: it puts back what it held, and the row moved
    // after it is the next Undo's. (The rows came back in the folder's order: `pair` is that order.)
    let pair: Vec<String> = rt
        .state
        .listing
        .items
        .iter()
        .filter(|item| ids.contains(&item.id))
        .map(|item| item.id.clone())
        .collect();
    rt.update(|s, _| {
        let rows: Vec<usize> = (0..s.listing.items.len())
            .filter(|&row| pair.contains(&s.listing.items[row].id))
            .collect();
        s.select_only(rows[0]);
        s.actions.selection.insert(rows[1]);
    });
    rt.update(|s, tx| assert!(s.trash_selection(tx)));
    spy.answer(
        Action::Trash,
        &[(&pair[0], None), (&pair[1], Some(ActionError::Failed))],
    );
    assert!(frame(&mut rt));
    rt.update(|s, tx| s.retry(tx));
    rt.update(|s, tx| assert!(s.undo(tx)));
    assert_eq!(
        spy.asked(),
        [
            (Action::Trash, pair.clone()),
            (Action::Trash, vec![pair[1].clone()]),
            (Action::Restore, vec![pair[0].clone()])
        ]
    );
    spy.answer(Action::Trash, &[(&pair[1], None)]);
    spy.answer(Action::Restore, &[(&pair[0], None)]);
    assert!(frame(&mut rt));
    assert!(shown(&rt, &pair[0]) && !shown(&rt, &pair[1]));
    assert_eq!(undo_ids(&rt), [pair[1].clone()], "only the row moved after that Undo");
    rt.update(|s, tx| assert!(s.undo(tx)));
    assert_eq!(spy.asked(), [(Action::Restore, vec![pair[1].clone()])]);
}

/// Review of 2468, L318: a move answered after another folder was opened is still said, with Undo for those rows;
/// their Undo puts nothing in the folder shown now.
#[test]
fn move_answered_after_folder_changed_is_still_said() {
    let (mut rt, spy, ids, _) = window();
    rt.update(|s, tx| assert!(s.trash_selection(tx)));
    let folder = rt
        .state
        .listing
        .items
        .iter()
        .find(|item| item.is_folder && !item.is_protected)
        .expect("the board lists a folder")
        .id
        .clone();
    rt.update(|s, _| s.open_folder(Some(folder.clone())));
    assert_eq!(rt.state.listing.folder.id.as_deref(), Some(folder.as_str()));
    let rows: Vec<String> = rt.state.listing.items.iter().map(|item| item.id.clone()).collect();

    spy.answer(Action::Trash, &[(&ids[0], None), (&ids[1], None)]);
    assert!(frame(&mut rt));
    assert_eq!(rt.state.actions.toast, Some(Toast::Trashed), "moved, and said");
    assert_eq!(undo_ids(&rt), ids, "with Undo for those rows");
    let now: Vec<String> = rt.state.listing.items.iter().map(|item| item.id.clone()).collect();
    assert_eq!(now, rows, "the folder shown did not change");

    rt.update(|s, tx| assert!(s.undo(tx)));
    assert_eq!(
        spy.asked(),
        [(Action::Trash, ids.clone()), (Action::Restore, ids.clone())]
    );
    spy.answer(Action::Restore, &[(&ids[0], None), (&ids[1], None)]);
    frame(&mut rt);
    let now: Vec<String> = rt.state.listing.items.iter().map(|item| item.id.clone()).collect();
    assert_eq!(now, rows, "rows of another folder were put in this one");
    assert!(rt.state.actions.undo.is_empty() && rt.state.actions.trash.restoring.is_empty());
}

/// Review of 2468, L319: "Moved to Trash" that lands while a failure that offers Try again stands is held, and said
/// with its Undo once that message went; it is never dropped.
#[test]
fn moved_to_trash_during_failure_is_said_after_it() {
    let (mut rt, spy, ids, other) = window();
    rt.update(|s, tx| assert!(s.trash_selection(tx)));
    // A second press, of another row, is refused at once: its message offers Try again.
    *spy.refuse.lock().unwrap() = Some(ActionError::Offline);
    select(&mut rt, &other);
    rt.update(|s, tx| assert!(s.trash_selection(tx)));
    let failure = Some(Toast::Failed(Action::Trash, ActionError::Offline));
    assert_eq!(rt.state.actions.toast, failure);
    assert!(rt.state.offers_try_again(Action::Trash, ActionError::Offline));

    // The first press's answer lands while that message stands: the rows leave, the message stays.
    spy.answer(Action::Trash, &[(&ids[0], None), (&ids[1], None)]);
    frame(&mut rt);
    assert!(!shown(&rt, &ids[0]) && !shown(&rt, &ids[1]));
    assert_eq!(
        rt.state.actions.toast, failure,
        "taken away by an answer nobody pressed for"
    );
    assert_eq!(undo_ids(&rt), ids);

    // The failure's time is over: "Moved to Trash" is said now, with its Undo.
    rt.update(|s, _| {
        s.actions.toast_until = Some(std::time::Instant::now());
        super::super::reset::poll(s);
    });
    assert!(frame(&mut rt));
    assert_eq!(rt.state.actions.toast, Some(Toast::Trashed), "the answer was dropped");
    assert_eq!(undo_ids(&rt), ids);
}

/// Review of 2468, L320: a row whose move to Trash is on its way is not renamed and not moved to another folder; the
/// message says the last change is still on its way. Answered, it can be again.
#[test]
fn rename_of_row_being_moved_to_trash_is_refused() {
    let (mut rt, spy, ids, other) = window();
    rt.update(|s, tx| assert!(s.trash_selection(tx)));
    select(&mut rt, &ids[0]);
    rt.update(|s, tx| assert!(s.begin_rename(tx)));
    assert_eq!(
        rt.state.actions.rename, None,
        "a name field on a row on its way to Trash"
    );
    assert_eq!(
        rt.state.actions.toast,
        Some(Toast::Failed(Action::Rename, ActionError::InFlight))
    );
    rt.update(|s, tx| assert!(s.begin_move(tx)));
    assert!(
        rt.state.actions.dialog.is_none(),
        "Move to, for a row on its way to Trash"
    );
    assert_eq!(
        rt.state.actions.toast,
        Some(Toast::Failed(Action::Move, ActionError::InFlight))
    );

    // A row whose move is not on its way is renamed as before.
    select(&mut rt, &other);
    rt.update(|s, tx| assert!(s.begin_rename(tx)));
    assert_eq!(
        rt.state.actions.rename.as_ref().map(|rename| rename.id.clone()),
        Some(other.clone())
    );
    rt.update(|s, tx| s.cancel_rename(tx));

    // Answered (Beebeeb left the first one here): it is no longer on its way, and can be renamed.
    spy.answer(Action::Trash, &[(&ids[0], Some(ActionError::Failed)), (&ids[1], None)]);
    assert!(frame(&mut rt));
    select(&mut rt, &ids[0]);
    rt.update(|s, tx| assert!(s.begin_rename(tx)));
    assert_eq!(
        rt.state.actions.rename.as_ref().map(|rename| rename.id.clone()),
        Some(ids[0].clone())
    );
}
