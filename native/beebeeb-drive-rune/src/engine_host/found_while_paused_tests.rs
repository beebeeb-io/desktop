//! What the found-while-paused card shows, and what each press asks of the engine (a double records the asks).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::found_while_paused::{KeepResult, Review};

use super::{Decisions, FoundWhilePausedSource};
use crate::frames::this_pc::found_while_paused::{Action, Card, Held, Kept};

const NOT_YOURS: &str =
    "Beebeeb couldn’t confirm these files belong to the account you’re signed in to, so nothing was changed.";

fn turn_of(n: u64) -> SessionGeneration {
    SessionGeneration::for_test(n)
}

#[derive(Default)]
struct Asked {
    reviews: Vec<u64>,
    uploads: Vec<(SessionGeneration, Vec<String>)>,
    keeps: Vec<(SessionGeneration, Vec<String>)>,
    shows: Vec<SessionGeneration>,
    opened: Vec<PathBuf>,
}

struct Double {
    turn: Mutex<u64>,
    asked: Mutex<Asked>,
    review: Mutex<Result<Review, String>>,
    refuse: Mutex<Option<String>>,
}

impl Double {
    fn new(ids: &[&str]) -> Arc<Double> {
        Arc::new(Double {
            turn: Mutex::new(7),
            asked: Mutex::default(),
            review: Mutex::new(Ok(Review {
                new_files: 2,
                new_folders: 1,
                edits: 1,
                deletions: 1,
                in_shared_folders: 1,
                op_ids: ids.iter().map(|s| s.to_string()).collect(),
                synced_by: Some("OneDrive".into()),
            })),
            refuse: Mutex::new(None),
        })
    }
    fn refused(&self) -> Result<(), String> {
        self.refuse.lock().unwrap().clone().map_or(Ok(()), Err)
    }
}

impl Decisions for Double {
    fn turn(&self) -> SessionGeneration {
        turn_of(*self.turn.lock().unwrap())
    }
    fn review(&self, turn: SessionGeneration) -> Result<Review, String> {
        self.asked
            .lock()
            .unwrap()
            .reviews
            .push(if turn == turn_of(7) { 7 } else { 0 });
        self.review.lock().unwrap().clone()
    }
    fn upload(&self, turn: SessionGeneration, op_ids: &[String]) -> Result<usize, String> {
        self.asked.lock().unwrap().uploads.push((turn, op_ids.to_vec()));
        self.refused().map(|_| op_ids.len())
    }
    fn keep(&self, turn: SessionGeneration, op_ids: &[String]) -> Result<KeepResult, String> {
        self.asked.lock().unwrap().keeps.push((turn, op_ids.to_vec()));
        self.refused().map(|_| KeepResult {
            moved_items: 2,
            moved_files: 5,
            kept_path: Some("x".into()),
            not_moved: 1,
            stays_held: 0,
            synced_by: Some("OneDrive".into()),
        })
    }
    fn kept_folder(&self, turn: SessionGeneration) -> Result<PathBuf, String> {
        self.asked.lock().unwrap().shows.push(turn);
        self.refused().map(|_| PathBuf::from("kept-folder"))
    }
    fn open_folder(&self, folder: &Path) -> bool {
        self.asked.lock().unwrap().opened.push(folder.to_path_buf());
        true
    }
}

/// Pump until `done` holds (the workers answer on their own threads).
fn settle(source: &mut FoundWhilePausedSource<Double>, done: impl Fn(&Card) -> bool) {
    let until = Instant::now() + Duration::from_secs(5);
    while !done(&source.card) {
        assert!(Instant::now() < until, "the worker never answered");
        source.pump();
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn read(engine: &Arc<Double>) -> FoundWhilePausedSource<Double> {
    let mut source = FoundWhilePausedSource::new(engine.clone());
    source.refresh();
    settle(&mut source, |c| c.held.is_some());
    source
}

#[test]
fn a_card_with_nothing_held_is_not_drawn() {
    assert!(!Card::default().visible(), "a shipped build draws no sample card");
    let empty = Card {
        held: Some(Held::default()),
        ..Card::default()
    };
    assert!(!empty.visible());
    assert!(
        crate::frames::this_pc::found_while_paused::view(&empty, |_, _| {}, &crate::theme::tokens::theme(false))
            .is_none()
    );
}

#[test]
fn the_card_says_what_was_found_in_counts_and_never_a_name() {
    let card = Card {
        held: Some(Held {
            new_folders: 1,
            new_files: 3,
            edits: 1,
            deletions: 0,
            in_shared_folders: 2,
            op_ids: vec!["a".into()],
            synced_by: Some("Dropbox".into()),
        }),
        ..Card::default()
    };
    assert_eq!(
        card.found_line().unwrap(),
        "While sync was paused, these changes were made in your Beebeeb folder, and Beebeeb hasn’t sent them: 1 new folder, 3 new files, 1 change to a file. 2 in shared folders."
    );
    assert_eq!(
        card.synced_note().unwrap(),
        "That folder is inside Dropbox, which may copy them to its own cloud."
    );
    assert!(card.can_decide());
    assert!(
        crate::frames::this_pc::found_while_paused::view(&card, |_, _| {}, &crate::theme::tokens::theme(true))
            .is_some()
    );
}

#[test]
fn what_a_keep_did_is_said_in_counts() {
    let card = Card {
        kept: Some(Kept {
            moved_items: 1,
            moved_files: 4,
            not_moved: 2,
            stays_held: 1,
            synced_by: None,
        }),
        ..Card::default()
    };
    assert_eq!(
        card.kept_line().unwrap(),
        "Moved 1 item (4 files) to “Beebeeb - not uploaded”. 2 couldn’t be moved and stay where they are, not uploaded. 1 change to items Beebeeb already has stay on hold."
    );
    assert!(card.visible() && card.can_show());
}

#[test]
fn upload_asks_the_engine_for_exactly_the_reviewed_ids_under_the_turn_it_was_read_at() {
    let engine = Double::new(&["op-1", "op-2"]);
    let mut source = read(&engine);
    // The session moves after the read: the ask still carries the turn of the read, so the engine refuses it.
    *engine.turn.lock().unwrap() = 8;
    assert!(source.upload());
    assert!(!source.upload(), "one action at a time");
    settle(&mut source, |c| c.busy.is_none());
    let asked = engine.asked.lock().unwrap();
    assert_eq!(
        asked.uploads,
        vec![(turn_of(7), vec!["op-1".to_string(), "op-2".to_string()])]
    );
    assert!(asked.keeps.is_empty());
}

#[test]
fn upload_reads_the_held_changes_again_afterwards() {
    let engine = Double::new(&["op-1"]);
    let mut source = read(&engine);
    source.upload();
    settle(&mut source, |c| c.busy.is_none());
    let until = Instant::now() + Duration::from_secs(5);
    while engine.asked.lock().unwrap().reviews.len() < 2 {
        assert!(Instant::now() < until, "no second review");
        source.pump();
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn keep_asks_for_the_same_ids_and_shows_what_moved() {
    let engine = Double::new(&["op-9"]);
    let mut source = read(&engine);
    assert!(source.keep());
    settle(&mut source, |c| c.busy.is_none() && c.kept.is_some());
    assert_eq!(
        engine.asked.lock().unwrap().keeps,
        vec![(turn_of(7), vec!["op-9".to_string()])]
    );
    assert!(engine.asked.lock().unwrap().uploads.is_empty(), "a keep never uploads");
    assert_eq!(source.card.kept.as_ref().unwrap().moved_items, 2);
    assert!(source.card.can_show());
}

#[test]
fn show_opens_only_the_folder_the_engine_names() {
    let engine = Double::new(&["op-9"]);
    let mut source = read(&engine);
    assert!(!source.show(), "nothing was kept yet");
    source.keep();
    settle(&mut source, |c| c.busy.is_none() && c.kept.is_some());
    assert!(source.show());
    settle(&mut source, |c| c.busy.is_none());
    let asked = engine.asked.lock().unwrap();
    assert_eq!(asked.shows, vec![turn_of(7)]);
    assert_eq!(asked.opened, vec![PathBuf::from("kept-folder")]);
}

#[test]
fn an_engine_refusal_is_shown_in_its_own_words_and_nothing_else_happens() {
    let engine = Double::new(&["op-1"]);
    let mut source = read(&engine);
    *engine.refuse.lock().unwrap() = Some(NOT_YOURS.into());
    source.upload();
    settle(&mut source, |c| c.busy.is_none());
    assert_eq!(source.card.said.as_deref(), Some(NOT_YOURS));
    assert!(source.card.kept.is_none());
}

#[test]
fn a_review_that_failed_leaves_nothing_to_decide() {
    let engine = Double::new(&["op-1"]);
    *engine.review.lock().unwrap() = Err("no".into());
    let mut source = FoundWhilePausedSource::new(engine.clone());
    source.refresh();
    settle(&mut source, |c| c.said.is_some());
    assert!(source.card.held.is_none());
    assert!(!source.upload() && !source.keep(), "an unread card decides nothing");
    let asked = engine.asked.lock().unwrap();
    assert!(asked.uploads.is_empty() && asked.keeps.is_empty());
}

#[test]
fn a_review_with_no_ids_offers_no_decision() {
    let engine = Double::new(&[]);
    let mut source = read(&engine);
    assert!(!source.upload() && !source.keep());
    assert!(engine.asked.lock().unwrap().uploads.is_empty());
    assert!(!source.press(Action::Upload) && !source.press(Action::Keep) && !source.press(Action::Show));
}
