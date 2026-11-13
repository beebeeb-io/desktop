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
    /// Ids the reclaim leaves (open in an app); their pin change works.
    stuck: Vec<String>,
    /// Ids that cannot be kept (again): only `set_kept(id, true)` fails.
    fails_keep: Vec<String>,
    /// What is kept and what was freed, as the engine's rows would say (`states_of`).
    kept: Vec<String>,
    freed: Vec<String>,
    /// This system has a reclaim for a selection.
    can_free: bool,
    /// The whole reclaim call fails.
    reclaim_fails: bool,
    /// The pin change of this id panics.
    panic_on: Option<String>,
    /// The pin change of this id is made, and the vault is locked by the time it returns.
    lock_on: Option<String>,
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
            stuck: Vec::new(),
            fails_keep: Vec::new(),
            kept: Vec::new(),
            freed: Vec::new(),
            can_free: true,
            reclaim_fails: false,
            panic_on: None,
            lock_on: None,
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
        let door = self.door.lock().unwrap();
        let mut s = self.s();
        s.kept_calls.push((id.to_string(), kept));
        if let Some(other) = s.switch_to.take() {
            s.account = Some(other);
        }
        if s.panic_on.as_deref() == Some(id) {
            // No lock is held when it panics: the next call must not find a poisoned one.
            drop(s);
            drop(door);
            panic!("an invented panic in the pin change");
        }
        if s.fails.iter().any(|f| f == id) || (kept && s.fails_keep.iter().any(|f| f == id)) {
            return Err("refused".into());
        }
        s.kept.retain(|k| k != id);
        if kept {
            s.kept.push(id.to_string());
        }
        if s.lock_on.as_deref() == Some(id) {
            s.unlocked = false;
        }
        Ok(())
    }
    fn can_free_selection(&self) -> bool {
        self.s().can_free
    }
    fn free_up_space_of(&self, ids: &[String]) -> Result<Vec<(String, Reclaimed)>, String> {
        let mut s = self.s();
        s.freed_calls.push(ids.to_vec());
        if s.reclaim_fails {
            return Err("refused".into());
        }
        Ok(ids
            .iter()
            .map(|id| {
                // As the engine's reclaim: a kept file is never touched.
                let what = if s.kept.contains(id) {
                    Reclaimed::Kept
                } else if s.fails.contains(id) || s.stuck.contains(id) {
                    Reclaimed::Left
                } else {
                    s.freed.push(id.clone());
                    Reclaimed::Freed
                };
                (id.clone(), what)
            })
            .collect())
    }
    fn states_of(&self, ids: &[String]) -> Result<Vec<Option<Option<String>>>, LocalStateRefused> {
        let s = self.s();
        Ok(ids
            .iter()
            .map(|id| {
                let mark = if s.freed.contains(id) {
                    "online_only"
                } else if s.kept.contains(id) {
                    "available_offline"
                } else {
                    "on_this_pc"
                };
                (!s.unknown.contains(id)).then(|| Some(mark.to_string()))
            })
            .collect())
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

fn wait_idle(copies: &LocalCopies<Fake>) {
    for _ in 0..2_000 {
        if copies.idle() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    panic!("the worker did not finish");
}

/// Everything the worker answered, once it is idle (also after a job that taking an answer started).
fn settled(copies: &mut LocalCopies<Fake>) -> Vec<Change> {
    let mut all = Vec::new();
    loop {
        wait_idle(copies);
        all.extend(copies.take());
        if copies.idle() {
            return all;
        }
    }
}

/// The changes in words a test compares: per item `id:what the engine says now:done or why not`.
fn said(changes: &[Change]) -> Vec<String> {
    changes
        .iter()
        .map(|change| match change {
            Change::Local { action, items } => {
                let per: Vec<String> = items
                    .iter()
                    .map(|o| {
                        let now = match &o.now {
                            None => "?",
                            Some(None) => "-",
                            Some(Some(mark)) => mark,
                        };
                        let left = o.left.map_or("done".to_string(), |why| format!("{why:?}"));
                        format!("{}:{now}:{left}", o.id)
                    })
                    .collect();
                format!("{action:?} {}", per.join(" "))
            }
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
    // Per item: what the engine says of it now, and whether it was done.
    assert_eq!(
        said(&changes),
        ["KeepOffline f-1:available_offline:done f-2:on_this_pc:Failed b-1:on_this_pc:Protected gone:?:Failed"]
    );
    // The Backups item and the unknown id never reached the engine.
    assert_eq!(
        fake.s().kept_calls,
        [("f-1".to_string(), true), ("f-2".to_string(), true)]
    );
    assert!(fake.s().wakes >= 1, "the window was not woken for the answer");
    // Remove from this PC is the same change the other way.
    copies.ask(Action::RemoveOffline, items(&[("f-1", true)])).unwrap();
    // The row reads what the engine says, not what the action suggests: no longer kept, and still on this PC.
    assert_eq!(said(&settled(&mut copies)), ["RemoveOffline f-1:on_this_pc:done"]);
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
        ["FreeUpSpace f-1:online_only:done f-2:online_only:done f-3:on_this_pc:Failed b-1:on_this_pc:Protected"]
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
    assert_eq!(said(&settled(&mut copies)), ["FreeUpSpace b-1:on_this_pc:Protected"]);
    assert!(
        fake.s().freed_calls.is_empty(),
        "the reclaim was asked for a Backups item"
    );
    fake.s().fails = vec!["f-1".into()];
    copies.ask(Action::KeepOffline, items(&[("f-1", false)])).unwrap();
    assert_eq!(said(&settled(&mut copies)), ["KeepOffline f-1:on_this_pc:Failed"]);
}

#[test]
fn a_free_up_space_that_did_not_free_does_not_cost_the_pin() {
    // Two kept items: the reclaim frees one and leaves the other (open in an app). The one left is kept again.
    let fake = Arc::new(Fake::default());
    {
        let mut s = fake.s();
        s.kept = vec!["f-open".into(), "f-ok".into()];
        s.stuck = vec!["f-open".into()];
    }
    let mut copies = LocalCopies::new(fake.clone());
    copies
        .ask(Action::FreeUpSpace, items(&[("f-open", true), ("f-ok", true)]))
        .unwrap();
    assert_eq!(
        said(&settled(&mut copies)),
        ["FreeUpSpace f-open:available_offline:Failed f-ok:online_only:done"]
    );
    // Released for the reclaim, both; kept again, only the one that was not freed.
    assert_eq!(
        fake.s().kept_calls,
        [
            ("f-open".to_string(), false),
            ("f-ok".to_string(), false),
            ("f-open".to_string(), true)
        ]
    );
    assert_eq!(fake.s().kept, ["f-open"]);

    // Released, then the whole reclaim fails: both are kept again, and the rows say so.
    let fake = Arc::new(Fake::default());
    {
        let mut s = fake.s();
        s.kept = vec!["f-1".into(), "f-2".into()];
        s.reclaim_fails = true;
    }
    let mut copies = LocalCopies::new(fake.clone());
    copies
        .ask(
            Action::FreeUpSpace,
            items(&[("f-1", true), ("f-2", true), ("f-3", false)]),
        )
        .unwrap();
    assert_eq!(
        said(&settled(&mut copies)),
        ["FreeUpSpace f-1:available_offline:Failed f-2:available_offline:Failed f-3:on_this_pc:Failed"]
    );
    assert_eq!(
        fake.s().kept_calls,
        [
            ("f-1".to_string(), false),
            ("f-2".to_string(), false),
            ("f-1".to_string(), true),
            ("f-2".to_string(), true)
        ],
        "what was released for a reclaim that failed was not kept again (or something else was)"
    );

    // Kept again fails too: the row is not said to be kept. It reads what the engine says: on this PC.
    let fake = Arc::new(Fake::default());
    {
        let mut s = fake.s();
        s.kept = vec!["f-1".into()];
        s.reclaim_fails = true;
        s.fails_keep = vec!["f-1".into()];
    }
    let mut copies = LocalCopies::new(fake.clone());
    copies.ask(Action::FreeUpSpace, items(&[("f-1", true)])).unwrap();
    assert_eq!(said(&settled(&mut copies)), ["FreeUpSpace f-1:on_this_pc:Failed"]);
}

#[test]
fn where_a_selection_cannot_be_reclaimed_free_up_space_is_refused_before_anything_changes() {
    let fake = Arc::new(Fake::default());
    {
        let mut s = fake.s();
        s.can_free = false;
        s.kept = vec!["f-1".into()];
    }
    let mut copies = LocalCopies::new(fake.clone());
    assert_eq!(
        copies.ask(Action::FreeUpSpace, items(&[("f-1", true), ("f-2", false)])),
        Err(ActionError::NotOnThisSystem)
    );
    assert!(copies.idle() && copies.take().is_empty());
    let s = fake.s();
    assert!(
        s.kept_calls.is_empty(),
        "a kept item was released for a reclaim that cannot run"
    );
    assert!(s.freed_calls.is_empty());
    drop(s);
    // Keeping is not this refusal's business.
    assert_eq!(copies.ask(Action::KeepOffline, items(&[("f-2", false)])), Ok(()));
    assert_eq!(said(&settled(&mut copies)), ["KeepOffline f-2:available_offline:done"]);
}

#[test]
fn an_accepted_change_whose_job_ended_early_is_said_as_failed() {
    // A panic in the middle: the first item was kept, the second panicked, the third was never reached.
    let fake = Arc::new(Fake::default());
    fake.s().panic_on = Some("f-2".into());
    let mut copies = LocalCopies::new(fake.clone());
    copies
        .ask(
            Action::KeepOffline,
            items(&[("f-1", false), ("f-2", false), ("f-3", false)]),
        )
        .unwrap();
    // Every item is said as not done (the job is not trusted), and each row reads what the engine says now.
    assert_eq!(
        said(&settled(&mut copies)),
        ["KeepOffline f-1:available_offline:Failed f-2:on_this_pc:Failed f-3:on_this_pc:Failed"]
    );
    // Its marks are gone: the items can be asked again.
    fake.s().panic_on = None;
    assert_eq!(copies.ask(Action::KeepOffline, items(&[("f-2", false)])), Ok(()));
    assert_eq!(said(&settled(&mut copies)), ["KeepOffline f-2:available_offline:done"]);

    // A lock while the change is made and an unlock before the window looks again: the window never saw the
    // session change, so the answer is its own, and it is said (as failed), with the rows as they are.
    let fake = Arc::new(Fake::default());
    fake.s().lock_on = Some("f-1".into());
    let mut copies = LocalCopies::new(fake.clone());
    copies
        .ask(Action::KeepOffline, items(&[("f-1", false), ("f-2", false)]))
        .unwrap();
    wait_idle(&copies);
    assert!(!fake.s().unlocked, "the invented lock did not happen");
    {
        let mut s = fake.s();
        s.lock_on = None;
        s.unlocked = true;
    }
    assert_eq!(
        said(&settled(&mut copies)),
        ["KeepOffline f-1:available_offline:Failed f-2:on_this_pc:Failed"]
    );
    assert_eq!(copies.ask(Action::KeepOffline, items(&[("f-2", false)])), Ok(()));
    settled(&mut copies);

    // Still locked when the window looks: that is another session, and nothing of it is said.
    let fake = Arc::new(Fake::default());
    fake.s().lock_on = Some("f-1".into());
    let mut copies = LocalCopies::new(fake.clone());
    copies.ask(Action::KeepOffline, items(&[("f-1", false)])).unwrap();
    assert!(
        settled(&mut copies).is_empty(),
        "an answer was said under a locked vault"
    );
}

#[test]
fn the_window_is_woken_when_a_message_is_due_and_not_before() {
    let (tx, rx) = mpsc::channel::<Instant>();
    let woke = Arc::new(Mutex::new(Vec::<Instant>::new()));
    let seen = woke.clone();
    let timer = std::thread::spawn(move || wake_when_due(&rx, || seen.lock().unwrap().push(Instant::now())));
    let start = Instant::now();
    // The later one first: each is woken at its own time.
    let (late, soon) = (start + Duration::from_millis(800), start + Duration::from_millis(40));
    tx.send(late).unwrap();
    tx.send(soon).unwrap();
    for _ in 0..2_000 {
        if woke.lock().unwrap().len() >= 2 {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let at = woke.lock().unwrap().clone();
    assert_eq!(at.len(), 2, "one wake per message");
    assert!(
        at[0] >= soon && at[0] < late,
        "the short message was not woken at its own time"
    );
    assert!(at[1] >= late, "woken before the message was due");
    // The source is dropped: the timer ends.
    drop(tx);
    timer.join().unwrap();

    // Through the adapter: the engine's wake, once, after the time asked.
    let fake = Arc::new(Fake::default());
    let mut copies = LocalCopies::new(fake.clone());
    let asked = Instant::now();
    copies.wake_after(Duration::from_millis(30));
    for _ in 0..2_000 {
        if fake.s().wakes > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(fake.s().wakes, 1);
    assert!(asked.elapsed() >= Duration::from_millis(30));
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
