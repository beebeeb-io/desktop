//! Slice A7 on the adapter (`EngineBackupsSource`): an add asked of the engine for the session of its press and its
//! answer dropped when it lands under another account; an add the engine took read back from the engine, with a pass
//! asked for; a refusal said in the engine's words; "Stop and keep the backup" and "Stop and move the backup to Trash"
//! reaching the engine as asked, the move only with the account password, one at a time. And the folder picker's
//! answer through the shell's poll (`shell::intents`), for the session of the press: one picked after the session
//! changed while the picker was open never reaches the engine (P2-1, the review of #463). Every name is invented.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::backup_devices_api::{BackupDevices, DevicesRefused};
use beebeeb_engine::app::backups_api::{BackupFolderRow, BackupsOverview};
use beebeeb_engine::backup_delete::DeleteBackupError;
use rune::{Env, Runtime};

use super::*;
use crate::State;
use crate::frames::backups::BackupsUi;
use crate::shell::actions::FixtureEngine;
use crate::shell::app::GateState;
use crate::shell::host::{Shell, poll};
use crate::shell::intents::{FolderFor, a_folder_picker_is_open, pick_folder_with, poll_drains_for_this_test};
use crate::shell::platform::PlatformError;
use crate::shell::platform::fake::FakePlatform;
use crate::shell::{registry, session_gate, state};

const A: &str = "invented-a@beebeeb.io";
const B: &str = "invented-b@beebeeb.io";
const PROJECTS: &str = "custom:Projects";
const PICKED: &str = r"C:\Users\Invented\Projects";
/// A folder picked after the session changed while the picker was open.
const PICKED_LATE: &str = r"C:\Users\Invented\Late";
/// A folder B picks in a picker B opened.
const PICKED_BY_B: &str = r"C:\Users\Invented\Theirs";

/// The engine handle as the adapter sees it: one chosen folder (Projects) beside Documents.
struct Fake {
    account: Mutex<Option<String>>,
    /// The session generation: it moves on at every sign-in.
    generation: Mutex<u64>,
    reads: Mutex<u32>,
    passes: Mutex<u32>,
    add_answer: Mutex<Result<(), AddRefused>>,
    adds: Mutex<Vec<(SessionGeneration, String)>>,
    remove_answer: Mutex<Result<(), RemoveRefused>>,
    /// Each stop: the session, the id, and the password's length when one was handed over (never the password).
    removes: Mutex<Vec<(SessionGeneration, String, Option<usize>)>>,
}

impl Fake {
    fn new() -> Arc<Fake> {
        Arc::new(Fake {
            account: Mutex::new(Some(A.into())),
            generation: Mutex::new(1),
            reads: Mutex::new(0),
            passes: Mutex::new(0),
            add_answer: Mutex::new(Ok(())),
            adds: Mutex::new(Vec::new()),
            remove_answer: Mutex::new(Ok(())),
            removes: Mutex::new(Vec::new()),
        })
    }

    fn sign_in(&self, account: &str) {
        *self.account.lock().unwrap() = Some(account.into());
        *self.generation.lock().unwrap() += 1;
    }

    fn reads(&self) -> u32 {
        *self.reads.lock().unwrap()
    }
}

fn row(key: &str, name: &str) -> BackupFolderRow {
    BackupFolderRow {
        key: key.into(),
        name: name.into(),
        on: true,
        state: FolderState::On,
        items: Some(46),
        last_pass_at_ms: Some(1_700_000_000_000),
        stop_words: None,
    }
}

impl BackupsEngine for Fake {
    fn unlocked(&self) -> bool {
        true
    }
    fn account(&self) -> Option<String> {
        self.account.lock().unwrap().clone()
    }
    fn overview(&self) -> Result<BackupsOverview, BackupsError> {
        *self.reads.lock().unwrap() += 1;
        Ok(BackupsOverview {
            root: None,
            folders: vec![row("documents", "Documents"), row(PROJECTS, "Projects")],
        })
    }
    fn set_folder(&self, _key: &str, _on: bool) -> Result<(), BackupsError> {
        Ok(())
    }
    fn pass_running(&self) -> bool {
        false
    }
    fn pass_requested(&self) -> bool {
        false
    }
    fn request_pass(&self) {
        *self.passes.lock().unwrap() += 1;
    }
    fn add_folder_for(&self, session: SessionGeneration, path: &str) -> Result<(), AddRefused> {
        self.adds.lock().unwrap().push((session, path.to_string()));
        *self.add_answer.lock().unwrap()
    }
    fn remove_folder_for(
        &self,
        session: SessionGeneration,
        id: &str,
        trash_password: Option<&[u8]>,
    ) -> Result<(), RemoveRefused> {
        self.removes
            .lock()
            .unwrap()
            .push((session, id.to_string(), trash_password.map(<[u8]>::len)));
        self.remove_answer.lock().unwrap().clone()
    }
}

/// Other devices are `backups_devices_tests`'s: none here.
impl DevicesEngine for Fake {
    fn stamp(&self) -> Stamp {
        (
            SessionGeneration::for_test(*self.generation.lock().unwrap()),
            self.account(),
        )
    }
    fn has_session(&self) -> bool {
        true
    }
    fn devices_for(&self, session: SessionGeneration) -> Result<BackupDevices, DevicesRefused> {
        Ok(BackupDevices {
            session,
            devices: Vec::new(),
            complete: true,
        })
    }
    fn delete_for(
        &self,
        _session: SessionGeneration,
        _folder_id: &str,
        _password: &[u8],
    ) -> Result<(), DeleteBackupError> {
        Err(DeleteBackupError::SignedOut)
    }
    fn wake(&self) {}
}

/// Pump until every worker finished; what the pumps said, the first of each.
fn settle(source: &mut EngineBackupsSource<Fake>) -> Pumped {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut seen = Pumped::default();
    loop {
        let pumped = source.pump();
        seen.refusal = seen.refusal.or(pumped.refusal);
        seen.stopped = seen.stopped.or(pumped.stopped);
        if source.idle() && source.backup_folders_idle() {
            let pumped = source.pump();
            seen.refusal = seen.refusal.or(pumped.refusal);
            seen.stopped = seen.stopped.or(pumped.stopped);
            return seen;
        }
        assert!(Instant::now() < deadline, "the workers did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// An add is asked for the session of its press. Answered under that session, the engine's refusal is said; the same
/// answer landing after another account signed in is dropped: nothing is said and nothing is read for it.
#[test]
fn add_is_session_bound() {
    let fake = Fake::new();
    *fake.add_answer.lock().unwrap() = Err(AddRefused::InsideSyncFolder);
    let mut source = EngineBackupsSource::new(fake.clone());
    settle(&mut source);
    assert!(source.wired(Part::AddFolder) && source.wired(Part::RemoveFolder));

    // The instrument can fail: an answer under the press's own session is said.
    assert!(source.add_folder(PICKED));
    let seen = settle(&mut source);
    assert_eq!(seen.refusal, Some(AddRefused::InsideSyncFolder.words()));

    // Late: the worker answers only after B signed in.
    let (release, hold) = mpsc::channel();
    source.backup_folders.hold_next = Some(hold);
    assert!(source.add_folder(PICKED));
    fake.sign_in(B);
    source.pump();
    release.send(()).unwrap();
    let seen = settle(&mut source);
    assert_eq!(seen.refusal, None, "an answer that outlived its session is dropped");

    let adds = fake.adds.lock().unwrap().clone();
    assert_eq!(
        adds,
        vec![
            (SessionGeneration::for_test(1), PICKED.to_string()),
            (SessionGeneration::for_test(1), PICKED.to_string()),
        ],
        "each add is asked for the session of its press"
    );
    assert_eq!(*fake.passes.lock().unwrap(), 0, "a refused add asks for no pass");
}

/// An add the engine took is shown only from the engine's next read, and asks the runner for a pass so the first
/// backup starts. With no path, or nobody signed in, nothing is asked.
#[test]
fn an_add_the_engine_took_is_read_back_and_starts_a_pass() {
    let fake = Fake::new();
    let mut source = EngineBackupsSource::new(fake.clone());
    settle(&mut source);
    let reads = fake.reads();
    assert!(!source.add_folder(""), "no path, nothing asked");
    assert!(source.add_folder(PICKED));
    let seen = settle(&mut source);
    assert_eq!(seen.refusal, None);
    assert_eq!(*fake.passes.lock().unwrap(), 1, "its first backup starts now");
    assert!(fake.reads() > reads, "the list is read again after the add");
    assert!(
        source
            .folders()
            .iter()
            .any(|f| f.id == PROJECTS && f.chosen_by_you && f.on),
        "the chosen folder is listed as the engine reads it"
    );
}

/// "Stop and keep the backup" reaches the engine without a password. A move to Trash asked without the password sends
/// nothing; with it, the password's copy reaches the engine, one at a time, and a refusal comes back through `pump`
/// for the dialog. A folder that is not a chosen one is never sent.
#[test]
fn stop_keeps_or_moves_to_trash_only_with_the_password() {
    let fake = Fake::new();
    let mut source = EngineBackupsSource::new(fake.clone());
    settle(&mut source);
    assert!(source.trash_needs_password());

    source.remove_folder(PROJECTS, false);
    settle(&mut source);
    source.remove_folder(PROJECTS, true);
    settle(&mut source);
    assert!(
        !source.remove_folder_to_trash(PROJECTS, ""),
        "no password, nothing sent"
    );
    assert!(
        !source.remove_folder_to_trash("documents", "invented password"),
        "a known folder is switched, never removed"
    );

    *fake.remove_answer.lock().unwrap() = Err(RemoveRefused::PasswordRefused(DeleteBackupError::WrongPassword));
    let (release, hold) = mpsc::channel();
    source.backup_folders.hold_next = Some(hold);
    assert!(source.remove_folder_to_trash(PROJECTS, "invented password"));
    assert!(source.stopping());
    assert!(
        !source.remove_folder_to_trash(PROJECTS, "invented password"),
        "one at a time"
    );
    release.send(()).unwrap();
    let seen = settle(&mut source);
    assert_eq!(
        seen.stopped,
        Some(Stopped::Refused("That password isn't right. Nothing was changed."))
    );
    assert!(!source.stopping());

    let session = SessionGeneration::for_test(1);
    assert_eq!(
        fake.removes.lock().unwrap().clone(),
        vec![
            (session, PROJECTS.to_string(), None),
            (session, PROJECTS.to_string(), Some("invented password".len())),
        ]
    );
}

const ENV: Env = Env {
    system_dark: false,
    scale: 2.0,
};

/// The window as the running app holds it: the shell signed in to A (the session gate open), Backups bound to the
/// adapter over `fake`, the shell's poll acting on its queues. Also the shell's engine (to sign in or out) and the count
/// of the adapter's adds and stops still out (to wait on).
fn window(fake: &Arc<Fake>) -> (Runtime<State>, Arc<FixtureEngine>, Arc<AtomicUsize>) {
    let shell = Arc::new(FixtureEngine::ready());
    shell.set_account(A);
    let source = EngineBackupsSource::new(fake.clone());
    let pending = source.backup_folders.pending.clone();
    let mut state = state::init_with(Shell::fixture_on(shell.clone(), Arc::new(FakePlatform::default())));
    state.backups = BackupsUi::new(Some(Box::new(source)));
    let mut rt = Runtime::new(state, registry::view, (1280.0, 800.0), ENV);
    rt.poll = Some(poll);
    poll_drains_for_this_test();
    rt.frame(0.0);
    settled(&mut rt, "the window opens on A's session");
    (rt, shell, pending)
}

/// Frames (each runs the shell's poll) until `done`; fails after ten seconds, saying `what`.
fn frame_until(rt: &mut Runtime<State>, what: &str, mut done: impl FnMut(&Runtime<State>) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(rt) {
        assert!(Instant::now() < deadline, "{what}");
        rt.frame(0.0);
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Frames until the window may act on the session that is signed in now (the session gate is open) and Backups has
/// read that session's list: the shell, the window and the adapter all see the same session.
fn settled(rt: &mut Runtime<State>, what: &str) {
    frame_until(rt, what, |rt| {
        session_gate::open(&rt.state)
            && rt
                .state
                .backups
                .source()
                .is_some_and(|source| !source.folders().is_empty())
    });
}

/// A folder picker that stays open until the test closes it (`close`), on `folder`.
fn held_picker(
    folder: &str,
) -> (
    mpsc::Sender<()>,
    impl FnOnce() -> Result<Option<PathBuf>, PlatformError> + Send + 'static,
) {
    let (close, closed) = mpsc::channel::<()>();
    let folder = PathBuf::from(folder);
    let picker = move || {
        let _ = closed.recv();
        Ok(Some(folder))
    };
    (close, picker)
}

/// The person closes the open picker on its folder: frames until the shell's poll took the answer (no picker is open)
/// and every add it asked of the engine has finished (it is in `Fake::adds`).
fn close_and_take(rt: &mut Runtime<State>, close: &mpsc::Sender<()>, pending: &AtomicUsize) {
    close.send(()).unwrap();
    frame_until(rt, "the picker's answer is taken", |_| !a_folder_picker_is_open());
    frame_until(rt, "no add is out", |_| pending.load(Ordering::SeqCst) == 0);
}

fn adds(fake: &Fake) -> Vec<(SessionGeneration, String)> {
    fake.adds.lock().unwrap().clone()
}

/// P2-1 (the review of #463): A presses Add a folder…, B signs in while the picker is still open, and the folder then
/// picked never reaches the engine: nothing is added to B's backups. B signs in fully before the pick (the shell, the
/// window and Backups all show B), the case the review traced. The instrument can fail: a pick under the press's own
/// session reaches the engine, and B's own press afterwards reaches B (the dropped pick left no picker open).
#[test]
fn a_folder_picked_after_another_account_signed_in_is_dropped() {
    let fake = Fake::new();
    let (mut rt, shell, pending) = window(&fake);

    let (close, picker) = held_picker(PICKED);
    assert!(
        pick_folder_with(&rt.state, FolderFor::Backups, picker),
        "A's press opens the picker"
    );
    close_and_take(&mut rt, &close, &pending);
    assert_eq!(
        adds(&fake),
        vec![(SessionGeneration::for_test(1), PICKED.to_string())],
        "a folder picked under the press's own session reaches the engine"
    );

    let (close, picker) = held_picker(PICKED_LATE);
    assert!(pick_folder_with(&rt.state, FolderFor::Backups, picker));
    fake.sign_in(B);
    shell.set_account(B);
    settled(&mut rt, "the window and Backups show B while A's picker is open");
    rt.frame(0.0);
    assert!(a_folder_picker_is_open(), "the picker is still open");
    close_and_take(&mut rt, &close, &pending);
    assert_eq!(
        adds(&fake).len(),
        1,
        "the folder picked after B signed in is not asked of the engine"
    );

    let (close, picker) = held_picker(PICKED_BY_B);
    assert!(
        pick_folder_with(&rt.state, FolderFor::Backups, picker),
        "B's own press opens a picker"
    );
    close_and_take(&mut rt, &close, &pending);
    assert_eq!(
        adds(&fake),
        vec![
            (SessionGeneration::for_test(1), PICKED.to_string()),
            (SessionGeneration::for_test(2), PICKED_BY_B.to_string()),
        ],
        "only A's pick under A and B's own pick under B reached the engine"
    );
}

/// P2-1: a session that changed while the picker was open is not the press's even when it seems to be back. A presses
/// Add a folder…, signs out while the picker is open and signs in again (a new session of the same account): the folder
/// then picked is dropped. And while the app is locked, a press opens no picker at all.
#[test]
fn a_folder_picked_across_a_sign_out_and_in_is_dropped() {
    let fake = Fake::new();
    let (mut rt, shell, pending) = window(&fake);

    let (close, picker) = held_picker(PICKED_LATE);
    assert!(pick_folder_with(&rt.state, FolderFor::Backups, picker));
    *fake.account.lock().unwrap() = None;
    shell.set_gate(GateState::SignedOut);
    frame_until(&mut rt, "the window sees the sign-out", |rt| {
        rt.state.shell.app.gate() == GateState::SignedOut
    });
    fake.sign_in(A);
    shell.set_gate(GateState::Ready);
    settled(&mut rt, "A's new session shows while the picker is open");
    rt.frame(0.0);
    assert!(a_folder_picker_is_open(), "the picker is still open");
    close_and_take(&mut rt, &close, &pending);
    assert!(
        adds(&fake).is_empty(),
        "the folder picked across a sign-out and in is not asked of the engine: {:?}",
        adds(&fake)
    );

    // The instrument can fail: A's press in the new session reaches the engine under that session.
    let (close, picker) = held_picker(PICKED);
    assert!(pick_folder_with(&rt.state, FolderFor::Backups, picker));
    close_and_take(&mut rt, &close, &pending);
    assert_eq!(adds(&fake), vec![(SessionGeneration::for_test(2), PICKED.to_string())]);

    // Locked (the account is still the window's): the session gate is closed, and a press opens no picker.
    shell.set_gate(GateState::Locked);
    frame_until(&mut rt, "the window sees the lock", |rt| {
        rt.state.shell.app.gate() == GateState::Locked
    });
    let (_close, picker) = held_picker(PICKED);
    assert!(
        !pick_folder_with(&rt.state, FolderFor::Backups, picker),
        "locked, a press opens no picker"
    );
    assert!(!a_folder_picker_is_open());
}
