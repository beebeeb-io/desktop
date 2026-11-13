//! `LocalCopies` against an engine of the test's own: what is sent, what is refused, whose answer counts, and what
//! is reported per item. The engine's side (`Engine::free_up_space_of`, the handle) is tested in the engine crate.
//! Every id is invented.

use std::sync::{Arc, Mutex, MutexGuard};

use super::*;

#[derive(Default)]
struct Fake {
    s: Mutex<Inner>,
    /// Held by a test: the next `set_kept` waits for it (a change that is on its way).
    door: Mutex<()>,
}

struct Inner {
    unlocked: bool,
    account: Option<String>,
    running: bool,
    backups: Vec<String>,
    unknown: Vec<String>,
    /// Ids whose pin change or reclaim fails.
    fails: Vec<String>,
    kept_calls: Vec<(String, bool)>,
    freed_calls: Vec<Vec<String>>,
    root: Option<String>,
    /// A sign-out, then this account, the moment a change is made.
    switch_to: Option<String>,
    wakes: usize,
    /// How many pin changes reached the door.
    at_door: usize,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            unlocked: true,
            account: Some("a@example.eu".into()),
            running: true,
            backups: vec!["b-1".into()],
            unknown: vec!["gone".into()],
            fails: Vec::new(),
            kept_calls: Vec::new(),
            freed_calls: Vec::new(),
            root: Some("b-root".into()),
            switch_to: None,
            wakes: 0,
            at_door: 0,
        }
    }
}

impl Fake {
    fn s(&self) -> MutexGuard<'_, Inner> {
        self.s.lock().unwrap()
    }
}

impl FilesEngine for Fake {
    fn unlocked(&self) -> bool {
        self.s().unlocked
    }
    fn account(&self) -> Option<String> {
        self.s().account.clone()
    }
    fn running(&self) -> bool {
        self.s().running
    }
    fn in_backups(&self, ids: &[String]) -> Result<Vec<Option<bool>>, LocalStateRefused> {
        let s = self.s();
        Ok(ids
            .iter()
            .map(|id| (!s.unknown.contains(id)).then(|| s.backups.contains(id)))
            .collect())
    }
    fn set_kept(&self, id: &str, kept: bool) -> Result<(), String> {
        self.s().at_door += 1;
        let _door = self.door.lock().unwrap();
        let mut s = self.s();
        s.kept_calls.push((id.to_string(), kept));
        if let Some(other) = s.switch_to.take() {
            s.account = Some(other);
        }
        if s.fails.iter().any(|f| f == id) {
            Err("refused".into())
        } else {
            Ok(())
        }
    }
    fn free_up_space_of(&self, ids: &[String]) -> Result<Vec<(String, bool)>, String> {
        let mut s = self.s();
        s.freed_calls.push(ids.to_vec());
        Ok(ids.iter().map(|id| (id.clone(), !s.fails.contains(id))).collect())
    }
    fn backups_root(&self) -> Result<Option<String>, LocalStateRefused> {
        Ok(self.s().root.clone())
    }
    fn wake(&self) {
        self.s().wakes += 1;
    }
}

fn items(ids: &[(&str, bool)]) -> Vec<(String, bool)> {
    ids.iter().map(|(id, kept)| (id.to_string(), *kept)).collect()
}

/// Everything the worker answered, once it is idle.
fn settled(copies: &mut LocalCopies<Fake>) -> Vec<Change> {
    for _ in 0..2_000 {
        if copies.idle() {
            return copies.take();
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    panic!("the worker did not finish");
}

/// The changes in words a test compares.
fn said(changes: &[Change]) -> Vec<String> {
    changes
        .iter()
        .map(|change| match change {
            Change::Local { action, done, failed } => format!("{action:?} done {done:?} failed {failed}"),
            Change::Failed(action, why) => format!("{action:?} {why:?}"),
            _ => "other".into(),
        })
        .collect()
}

#[test]
fn keeping_is_accepted_at_once_and_reported_per_item_when_the_engine_answered() {
    let fake = Arc::new(Fake::default());
    fake.s().fails = vec!["f-2".into()];
    let mut copies = LocalCopies::new(fake.clone());
    let door = fake.door.lock().unwrap();
    assert_eq!(
        copies.ask(
            Action::KeepOffline,
            items(&[("f-1", false), ("f-2", false), ("b-1", false), ("gone", false)])
        ),
        Ok(())
    );
    // On its way: nothing is said yet.
    assert!(
        copies.take().is_empty(),
        "a change was reported before the engine made it"
    );
    drop(door);
    let changes = settled(&mut copies);
    assert_eq!(said(&changes), [r#"KeepOffline done ["f-1"] failed 3"#]);
    // The Backups item and the unknown id never reached the engine.
    assert_eq!(
        fake.s().kept_calls,
        [("f-1".to_string(), true), ("f-2".to_string(), true)]
    );
    assert!(fake.s().wakes >= 1, "the window was not woken for the answer");
    // Remove from this PC is the same change the other way.
    copies.ask(Action::RemoveOffline, items(&[("f-1", true)])).unwrap();
    assert_eq!(said(&settled(&mut copies)), [r#"RemoveOffline done ["f-1"] failed 0"#]);
    assert_eq!(fake.s().kept_calls.last(), Some(&("f-1".to_string(), false)));
}

#[test]
fn free_up_space_names_the_selection_releases_what_was_kept_and_reports_per_item() {
    let fake = Arc::new(Fake::default());
    fake.s().fails = vec!["f-3".into()];
    let mut copies = LocalCopies::new(fake.clone());
    copies
        .ask(
            Action::FreeUpSpace,
            items(&[("f-1", false), ("f-2", true), ("f-3", false), ("b-1", false)]),
        )
        .unwrap();
    assert_eq!(
        said(&settled(&mut copies)),
        [r#"FreeUpSpace done ["f-1", "f-2"] failed 2"#]
    );
    // Only the row that was kept is released; the reclaim is asked for the selection alone, never Backups.
    assert_eq!(fake.s().kept_calls, [("f-2".to_string(), false)]);
    assert_eq!(
        fake.s().freed_calls,
        [vec!["f-1".to_string(), "f-2".to_string(), "f-3".to_string()]]
    );
}

#[test]
fn nothing_done_is_a_failure_with_its_reason() {
    let fake = Arc::new(Fake::default());
    let mut copies = LocalCopies::new(fake.clone());
    copies.ask(Action::FreeUpSpace, items(&[("b-1", false)])).unwrap();
    assert_eq!(said(&settled(&mut copies)), ["FreeUpSpace Protected"]);
    assert!(
        fake.s().freed_calls.is_empty(),
        "the reclaim was asked for a Backups item"
    );
    fake.s().fails = vec!["f-1".into()];
    copies.ask(Action::KeepOffline, items(&[("f-1", false)])).unwrap();
    assert_eq!(said(&settled(&mut copies)), ["KeepOffline Failed"]);
}

#[test]
fn without_an_open_vault_or_a_running_engine_nothing_is_sent() {
    let fake = Arc::new(Fake::default());
    let mut copies = LocalCopies::new(fake.clone());
    fake.s().running = false;
    assert_eq!(
        copies.ask(Action::KeepOffline, items(&[("f-1", false)])),
        Err(ActionError::Unavailable)
    );
    fake.s().running = true;
    fake.s().unlocked = false;
    assert_eq!(
        copies.ask(Action::FreeUpSpace, items(&[("f-1", false)])),
        Err(ActionError::Unavailable)
    );
    fake.s().unlocked = true;
    fake.s().account = None;
    assert_eq!(
        copies.ask(Action::KeepOffline, items(&[("f-1", false)])),
        Err(ActionError::Unavailable)
    );
    assert!(copies.idle() && fake.s().kept_calls.is_empty() && fake.s().freed_calls.is_empty());
}

#[test]
fn a_second_change_of_an_item_on_its_way_sends_nothing_and_says_so() {
    let fake = Arc::new(Fake::default());
    let mut copies = LocalCopies::new(fake.clone());
    let door = fake.door.lock().unwrap();
    copies.ask(Action::KeepOffline, items(&[("f-1", false)])).unwrap();
    assert_eq!(
        copies.ask(Action::FreeUpSpace, items(&[("f-2", false), ("f-1", false)])),
        Err(ActionError::InFlight)
    );
    drop(door);
    assert_eq!(settled(&mut copies).len(), 1);
    assert_eq!(fake.s().kept_calls.len(), 1, "the second press reached the engine");
    // Answered: the mark is gone, on a failure too.
    fake.s().fails = vec!["f-1".into()];
    copies.ask(Action::KeepOffline, items(&[("f-1", false)])).unwrap();
    settled(&mut copies);
    assert_eq!(copies.ask(Action::KeepOffline, items(&[("f-1", false)])), Ok(()));
    settled(&mut copies);
}

#[test]
fn an_answer_that_outlived_its_account_is_never_reported() {
    // The account changes while the change is made: the answer is dropped, and its marks with it.
    let fake = Arc::new(Fake::default());
    let mut copies = LocalCopies::new(fake.clone());
    fake.s().switch_to = Some("b@example.eu".into());
    copies.ask(Action::KeepOffline, items(&[("f-1", false)])).unwrap();
    assert!(
        settled(&mut copies).is_empty(),
        "another account was told of the first one's change"
    );
    assert_eq!(
        copies.ask(Action::KeepOffline, items(&[("f-1", false)])),
        Ok(()),
        "a mark outlived its account"
    );
    settled(&mut copies);

    // A lock, then an unlock, while the change is still being made: the answer is the same account's, and of a
    // session that is over. It is not said, and its mark does not hold the item.
    let fake = Arc::new(Fake::default());
    let mut copies = LocalCopies::new(fake.clone());
    let door = fake.door.lock().unwrap();
    copies.ask(Action::KeepOffline, items(&[("f-1", false)])).unwrap();
    for _ in 0..2_000 {
        if fake.s().at_door > 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(fake.s().at_door, 1, "the change never started");
    fake.s().unlocked = false;
    assert!(copies.pump());
    fake.s().unlocked = true;
    assert!(copies.pump());
    drop(door);
    assert!(
        settled(&mut copies).is_empty(),
        "an answer of the session before the lock was said after it"
    );
    assert_eq!(copies.ask(Action::KeepOffline, items(&[("f-1", false)])), Ok(()));
    settled(&mut copies);
}

#[test]
fn the_backups_id_is_read_per_session_and_handed_out_only_while_the_section_is_on() {
    let fake = Arc::new(Fake::default());
    let mut copies = LocalCopies::new(fake.clone());
    copies.pump();
    settled(&mut copies);
    if BACKUPS_SECTION_ON {
        assert_eq!(copies.backups_root().as_deref(), Some("b-root"));
        // Another account: the first one's id is not handed out, before any pump.
        fake.s().account = Some("b@example.eu".into());
        assert_eq!(copies.backups_root(), None);
    } else {
        // Off: nothing is read, nothing is handed out, and no worker was started for it.
        assert_eq!(copies.backups_root(), None);
        assert!(copies.jobs.is_none(), "a worker ran for a section that is off");
    }
    // The worker's read itself, whatever the switch says.
    let job = Job {
        ask: Ask::BackupsRoot,
        account: "a@example.eu".into(),
        generation: 7,
    };
    fake.s().account = Some("a@example.eu".into());
    assert!(matches!(
        run(&*fake, &job),
        Answer::BackupsRoot { generation: 7, whose: Some(_), root: Some(root) } if root == "b-root"
    ));
    fake.s().root = None;
    assert!(matches!(
        run(&*fake, &job),
        Answer::BackupsRoot {
            whose: Some(_),
            root: None,
            ..
        }
    ));
    fake.s().unlocked = false;
    assert!(matches!(
        run(&*fake, &job),
        Answer::BackupsRoot {
            whose: None,
            root: None,
            ..
        }
    ));
}
