//! Slice A7 on the adapter (`EngineBackupsSource`): an add asked of the engine for the session of its press and its
//! answer dropped when it lands under another account; an add the engine took read back from the engine, with a pass
//! asked for; a refusal said in the engine's words; "Stop and keep the backup" and "Stop and move the backup to Trash"
//! reaching the engine as asked, the move only with the account password, one at a time. Every name is invented.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::backup_devices_api::{BackupDevices, DevicesRefused};
use beebeeb_engine::app::backups_api::{BackupFolderRow, BackupsOverview};
use beebeeb_engine::backup_delete::DeleteBackupError;

use super::*;

const A: &str = "invented-a@beebeeb.io";
const B: &str = "invented-b@beebeeb.io";
const PROJECTS: &str = "custom:Projects";
const PICKED: &str = r"C:\Users\Invented\Projects";

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
