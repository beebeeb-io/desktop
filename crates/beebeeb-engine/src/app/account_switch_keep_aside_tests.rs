//! dc-RC-b, "Keep them aside", at the engine: account A signed out leaving files in the sync folder that are no
//! synced copies (ordinary files, one in a folder A made), so the switch's removal refuses and B's start with it.
//! The refusal names a reason a person can answer; "Keep them aside" moves each file, byte for byte, into a dated
//! folder beside the sync folder and the start goes on; the discard of 111 takes the same files. Nothing leaves the
//! sync folder before one of the two was asked. Every name is invented.

use super::*;
use crate::account_binding::DiscardRefused;
use crate::keychain::SecretBytes;

const SYNCED: &str = "Synced by A.txt";
const A_KEY: [u8; 32] = [0xA1; 32];
/// What A left, by path under the sync folder, with its bytes.
const LEFT: [(&str, &str); 2] =
    [("Drafts/Plan of A.txt", "typed by A, never sent"), ("Notes of A.txt", "A's own notes")];

/// Windows' answers for this switch: a file is a synced copy only when it is [`SYNCED`]; the removal refuses,
/// deleting nothing, at any other file and at a folder no row names (the Windows purge's rule), else it deletes
/// every item.
struct AsidePort {
    removals: AtomicU32,
}

impl AsidePort {
    fn new() -> Self {
        Self { removals: AtomicU32::new(0) }
    }
}

impl SyncRootPort for AsidePort {
    fn registration(&self, _folder: &Path) -> Registration {
        Registration::Nobody
    }
    fn register(&self, _folder: &Path) -> Result<(), Option<i32>> {
        Err(Some(CODE))
    }
    fn engine_lock_held(&self, _state_dir: &Path) -> bool {
        false
    }
    fn may_hold_bytes(&self, folder: &Path, path: &str) -> bool {
        std::fs::metadata(folder.join(path)).is_ok_and(|item| item.len() > 0)
    }
    fn left_by_a_person(&self, folder: &Path, path: &str) -> bool {
        folder.join(path).is_file() && path != SYNCED
    }
    fn remove_synced_items(&self, db: &StateDb, folder: &Path, _unasked: &[String]) -> Result<(), String> {
        self.removals.fetch_add(1, Ordering::SeqCst);
        // RC3-1: the Windows purge's own first gate (`windows_cf::signout::purge`): while the database holds a queued
        // change or a file that is not at rest it refuses before it looks at the folder.
        purge_gate(db)?;
        for path in listing(folder) {
            let path = path.replace('\\', "/");
            let on_the_list = db.get_file_by_path(&path).unwrap().is_some();
            let ours = if folder.join(&path).is_dir() { on_the_list } else { path == SYNCED };
            if !ours {
                return Err("an item that is not on the list".to_string());
            }
        }
        for entry in std::fs::read_dir(folder).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.is_dir() {
                std::fs::remove_dir_all(&path).map_err(|e| e.to_string())?;
            } else {
                std::fs::remove_file(&path).map_err(|e| e.to_string())?;
            }
        }
        purge_finish(db)
    }
}

/// RC3-1: what the Windows purge asks of the database before it removes anything (its preflight, the same
/// portable policy function): no queued change, no file that is not at rest.
fn purge_gate(db: &StateDb) -> Result<(), String> {
    db.windows_signout_preflight().map_err(|_| "unsent work is in the database".to_string())
}

/// RC3-1: how the Windows purge ends: the staged copies' rows go, then the account's rows in the one transaction
/// that refuses while a queued change is left.
fn purge_finish(db: &StateDb) -> Result<(), String> {
    for (path, _, _) in db.staged_payloads_for_signout().map_err(|e| e.to_string())? {
        db.forget_staged_payload(&path).map_err(|e| e.to_string())?;
    }
    db.finish_windows_signout().map_err(|_| "the rows stay".to_string())
}

fn a() -> Identity {
    Identity::new(Some("user-rc-b-a"), Some("rc-b-a@example.eu"))
}

/// A synced one file and signed out; `left` says whether it also left [`LEFT`]. A's key is the working key. B signs in.
fn a_signed_out(pc: &Pc, left: bool) {
    let db = pc.db();
    row(&db, IDS[0], SYNCED, false, 3);
    db.set_owner(&a()).unwrap();
    drop(db);
    rusqlite::Connection::open(pc.state_db())
        .unwrap()
        .execute("UPDATE files SET local_hash = 'h', fetched_min_version = 0 WHERE item_kind = 'file'", [])
        .unwrap();
    std::fs::write(pc.root().join(SYNCED), b"ten bytes!").unwrap();
    if left {
        for (path, bytes) in LEFT {
            let at = pc.root().join(path);
            std::fs::create_dir_all(at.parent().unwrap()).unwrap();
            std::fs::write(at, bytes).unwrap();
        }
    }
    let vault = pc.engine.vault();
    vault.store_wrapped_master_key(SecretBytes::new_master_key(&A_KEY)).unwrap();
    vault.bind_vault_key_owner(a().user_id.as_deref().unwrap()).unwrap();
    pc.sign_in(&other());
}

/// The folder "Keep them aside" makes for A today, beside the sync folder.
fn aside(pc: &Pc) -> PathBuf {
    let day = chrono::Local::now().date_naive().format("%Y-%m-%d");
    pc.temp.path().join(format!("Beebeeb files from rc-b-a@example.eu ({day})"))
}

fn a_key_in_its_slot(pc: &Pc) -> Option<Vec<u8>> {
    use crate::keychain::AuthSecretStore;
    pc.engine
        .stores
        .for_account(pc.engine.identity, &pc.engine.config.account_id)
        .load_account_key(a().user_id.as_deref().unwrap())
        .unwrap()
        .map(|key| key.expose_for_crypto().to_vec())
}

/// The red test (the Windows test machine, RC1): files A left refuse B's start with a reason that has a fix, not the plain
/// `OtherAccountsData`; nothing was moved, removed or recorded for it, and both answers are offered.
#[test]
fn files_the_previous_account_left_refuse_with_a_reason_a_person_can_answer() {
    let pc = Pc::new();
    a_signed_out(&pc, true);
    let port = AsidePort::new();
    let (contents, files) = (pc.contents(), listing(&pc.root()));

    assert_eq!(
        pc.engine.start_sync_with(pc.root(), &port),
        Err(SyncStartRefused::LeftoverFilesOfAnotherAccount)
    );

    assert_eq!(pc.contents(), contents);
    assert_eq!(listing(&pc.root()), files);
    assert_eq!(pc.db().owner().unwrap(), Some(a()), "the data is still A's");
    assert!(!aside(&pc).exists(), "nothing moves before the person chose");
    let ask = pc.engine.discard_and_switch_ask_with(&pc.root(), &port).unwrap();
    assert_eq!(ask.files, 2, "the discard is offered for the same files");
}

/// "Keep them aside": each file is in the dated folder beside the sync folder, at its own path, with A's bytes; the
/// sync folder holds nothing of A's; the start went past the binding (B owns the data); A's key is in A's own slot.
#[test]
fn keep_aside_moves_every_file_byte_for_byte_and_the_start_goes_on() {
    let pc = Pc::new();
    a_signed_out(&pc, true);
    let port = AsidePort::new();
    assert_eq!(
        pc.engine.start_sync_with(pc.root(), &port),
        Err(SyncStartRefused::LeftoverFilesOfAnotherAccount)
    );

    let done = pc.engine.keep_aside_and_switch_with(pc.root(), &port).unwrap();

    assert_eq!((done.kept, done.kept_in.clone()), (2, aside(&pc)));
    assert!(!done.kept_in.starts_with(pc.root()), "outside the sync folder");
    for (path, bytes) in LEFT {
        assert_eq!(std::fs::read_to_string(aside(&pc).join(path)).unwrap(), bytes, "{path}");
    }
    assert_eq!(listing(&aside(&pc)).len(), 3, "the two files and A's folder, nothing else");
    assert_eq!(done.start, Err(SyncStartRefused::RegistrationFailed { code: Some(CODE) }), "past the binding");
    assert_eq!(pc.db().owner().unwrap(), Some(other()), "B owns the local data now");
    assert_eq!(listing(&pc.root()), Vec::<String>::new(), "nothing of A's is left in the sync folder");
    assert_eq!(a_key_in_its_slot(&pc), Some(A_KEY.to_vec()), "R4: A's key is kept in A's slot");
}

/// Never into a folder that is already there: one with today's name keeps what it holds, and the files go to a new
/// one beside it.
#[test]
fn a_folder_of_that_name_is_never_written_into() {
    let pc = Pc::new();
    a_signed_out(&pc, true);
    let port = AsidePort::new();
    std::fs::create_dir_all(aside(&pc)).unwrap();
    std::fs::write(aside(&pc).join("Notes of A.txt"), b"kept before").unwrap();

    let done = pc.engine.keep_aside_and_switch_with(pc.root(), &port).unwrap();

    assert_ne!(done.kept_in, aside(&pc));
    assert_eq!(std::fs::read(aside(&pc).join("Notes of A.txt")).unwrap(), b"kept before");
    assert_eq!(listing(&aside(&pc)).len(), 1);
    assert_eq!(std::fs::read_to_string(done.kept_in.join("Notes of A.txt")).unwrap(), "A's own notes");
}

/// The discard of 111 takes the files A left too: a copy of each in the kept folder first, then the switch.
#[test]
fn the_discard_keeps_a_copy_of_each_left_file_and_switches() {
    let pc = Pc::new();
    a_signed_out(&pc, true);
    let port = AsidePort::new();
    let ask = pc.engine.discard_and_switch_ask_with(&pc.root(), &port).unwrap();
    assert_eq!(ask.names, vec!["Notes of A.txt".to_string(), "Plan of A.txt".to_string()], "names, never paths");

    let done = pc.engine.discard_and_switch_with(&ask, pc.root(), &port).unwrap();

    assert_eq!(done.kept, 2);
    // The kept writer resolves its root (macOS: /var is /private/var), so compare against the resolved state folder.
    let kept_root = std::fs::canonicalize(&pc.engine.config.state_dir)
        .unwrap()
        .join(crate::sign_out::kept_names::KEPT_FOLDER_NAME);
    // Windows: canonicalize gives the \\?\ form; resolve both sides the same way.
    let kept_in = std::fs::canonicalize(&done.kept_in).unwrap();
    assert!(kept_in.starts_with(&kept_root), "{kept_in:?} not under {kept_root:?}");
    let mut kept: Vec<String> = listing(&done.kept_in)
        .into_iter()
        .filter(|path| done.kept_in.join(path).is_file())
        .map(|path| std::fs::read_to_string(done.kept_in.join(path)).unwrap())
        .collect();
    kept.sort();
    assert_eq!(kept, vec!["A's own notes".to_string(), "typed by A, never sent".to_string()]);
    assert_eq!(done.start, Err(SyncStartRefused::RegistrationFailed { code: Some(CODE) }));
    assert_eq!(listing(&pc.root()), Vec::<String>::new());
    assert!(!aside(&pc).exists(), "the discard makes no folder beside the sync folder");
}

/// With nothing of a person's in the folder there is nothing to keep aside: asked anyway, it changes nothing, and
/// the plain switch needs no answer.
#[test]
fn nothing_left_means_nothing_is_moved() {
    let pc = Pc::new();
    a_signed_out(&pc, false);
    let port = AsidePort::new();
    let (contents, files) = (pc.contents(), listing(&pc.root()));

    assert_eq!(pc.engine.keep_aside_and_switch_with(pc.root(), &port), Err(DiscardRefused::NothingToDiscard));

    assert_eq!((pc.contents(), listing(&pc.root())), (contents, files));
    assert_eq!(port.removals.load(Ordering::SeqCst), 0);
    assert!(!aside(&pc).exists());
    let start = pc.engine.start_sync_with(pc.root(), &port);
    assert_eq!(start, Err(SyncStartRefused::RegistrationFailed { code: Some(CODE) }));
    assert_eq!(pc.db().owner().unwrap(), Some(other()));
}

/// Other work of A's (a queued rename: nothing of it is an item a person could keep) needs A: the refusal stays the
/// plain one and keep aside moves nothing.
#[test]
fn a_queued_change_of_the_previous_account_is_not_kept_aside() {
    let pc = Pc::new();
    a_signed_out(&pc, true);
    pc.db()
        .enqueue_operation(&PendingOperation {
            base_version: Some(3),
            ..queued("op-rc-b", OperationKind::RenameFile, IDS[0], None)
        })
        .unwrap();
    let port = AsidePort::new();
    let files = listing(&pc.root());

    assert_eq!(pc.engine.start_sync_with(pc.root(), &port), Err(SyncStartRefused::OtherAccountsData));
    assert_eq!(pc.engine.keep_aside_and_switch_with(pc.root(), &port), Err(DiscardRefused::OtherWork));

    assert_eq!(listing(&pc.root()), files);
    assert!(!aside(&pc).exists());
}

/// dc-175 (the Windows test machine): A left only empty folders no row names. They are nothing to keep: the start removes them and the
/// switch completes, with no refusal for a person to answer. A folder holding a synced copy is never removed this way.
#[test]
fn empty_folders_the_previous_account_left_do_not_stop_the_switch() {
    let pc = Pc::new();
    a_signed_out(&pc, false);
    std::fs::create_dir_all(pc.root().join("Empty of A/Deeper")).unwrap();
    std::fs::create_dir_all(pc.root().join("Also empty")).unwrap();
    let port = AsidePort::new();

    let start = pc.engine.start_sync_with(pc.root(), &port);

    assert_eq!(start, Err(SyncStartRefused::RegistrationFailed { code: Some(CODE) }), "past the binding");
    assert_eq!(pc.db().owner().unwrap(), Some(other()), "B owns the local data now");
    assert_eq!(listing(&pc.root()), Vec::<String>::new());
    assert!(!aside(&pc).exists(), "nothing was kept aside: there was nothing to keep");
}

// ---- RC2-5 (the Windows test machine, rc2/o3, o4): what the previous account made or changed while sync was paused ----

const NEW_OF_A: &str = "New of A.txt";

/// A made `path` while sync was paused: a row of its own (never on the server, `uploading`) and its queued change.
fn queued_while_paused(pc: &Pc, id: &str, path: &str, folder: bool) {
    let db = pc.db();
    row(&db, id, path, folder, 0);
    let kind = if folder {
        OperationKind::CreateFolder
    } else {
        OperationKind::UploadFile
    };
    db.enqueue_operation(&queued(&format!("op-{id}"), kind, id, None))
        .unwrap();
    drop(db);
    if !folder {
        rusqlite::Connection::open(pc.state_db())
            .unwrap()
            .execute("UPDATE files SET status = 'uploading' WHERE file_id = ?1", [id])
            .unwrap();
    }
    let at = pc.root().join(path);
    if folder {
        std::fs::create_dir_all(at).unwrap();
    } else {
        std::fs::write(at, b"made by A while paused").unwrap();
    }
}

/// (a) rc2/o3 RC-b: a new file A made while sync was paused (its upload queued), with an empty folder beside it, is
/// refused with the choice, not the plain `OtherAccountsData`; nothing moved before the answer; "Keep them aside"
/// then moves it byte for byte and the switch completes; asked again, the discard is offered for the same file.
#[test]
fn account_switch_rc2_a_new_file_made_while_paused_offers_keep_aside_and_discard() {
    let pc = Pc::new();
    a_signed_out(&pc, false);
    queued_while_paused(&pc, IDS[1], NEW_OF_A, false);
    std::fs::create_dir_all(pc.root().join("Empty of A")).unwrap();
    let port = AsidePort::new();
    let files = listing(&pc.root());

    assert_eq!(
        pc.engine.start_sync_with(pc.root(), &port),
        Err(SyncStartRefused::LeftoverFilesOfAnotherAccount)
    );
    assert_eq!(listing(&pc.root()), files, "nothing moves before the person chose");
    assert_eq!(pc.db().owner().unwrap(), Some(a()));
    let ask = pc.engine.discard_and_switch_ask_with(&pc.root(), &port).unwrap();
    assert_eq!(
        ask.names,
        vec![NEW_OF_A.to_string()],
        "the discard is offered for the same file"
    );

    let done = pc.engine.keep_aside_and_switch_with(pc.root(), &port).unwrap();

    assert_eq!(done.kept, 1);
    assert_eq!(
        std::fs::read(aside(&pc).join(NEW_OF_A)).unwrap(),
        b"made by A while paused"
    );
    assert_eq!(
        done.start,
        Err(SyncStartRefused::RegistrationFailed { code: Some(CODE) }),
        "past the binding"
    );
    assert_eq!(pc.db().owner().unwrap(), Some(other()), "B owns the local data now");
    assert_eq!(listing(&pc.root()), Vec::<String>::new());
}

/// (a), the other answer (and 111's way on): "Discard and switch" keeps a copy of the new file and of an edit A
/// queued while paused, then the switch completes.
#[test]
fn account_switch_rc2_discard_takes_a_new_file_and_a_queued_edit_and_switches() {
    let pc = Pc::new();
    a_signed_out(&pc, false);
    queued_while_paused(&pc, IDS[1], NEW_OF_A, false);
    std::fs::write(pc.root().join(SYNCED), b"edited by A").unwrap();
    pc.db()
        .enqueue_operation(&PendingOperation {
            base_version: Some(3),
            ..queued("op-edit", OperationKind::UploadVersion, IDS[0], None)
        })
        .unwrap();
    let port = AsidePort::new();
    assert_eq!(
        pc.engine.start_sync_with(pc.root(), &port),
        Err(SyncStartRefused::LeftoverFilesOfAnotherAccount)
    );
    let ask = pc.engine.discard_and_switch_ask_with(&pc.root(), &port).unwrap();
    assert_eq!(ask.files, 2);

    let done = pc.engine.discard_and_switch_with(&ask, pc.root(), &port).unwrap();

    assert_eq!(done.kept, 2);
    assert_eq!(
        done.start,
        Err(SyncStartRefused::RegistrationFailed { code: Some(CODE) })
    );
    assert_eq!(pc.db().owner().unwrap(), Some(other()));
    assert_eq!(listing(&pc.root()), Vec::<String>::new());
}

/// (b) rc2/o4 175: only empty folders A made while sync was paused (each queued, so a row names it) are left. They
/// are nothing to keep: the start removes them and the switch completes, with no refusal for a person to answer.
#[test]
fn account_switch_rc2_only_empty_folders_queued_while_paused_do_not_stop_the_switch() {
    let pc = Pc::new();
    a_signed_out(&pc, false);
    queued_while_paused(&pc, IDS[1], "Empty B", true);
    queued_while_paused(&pc, IDS[2], "Empty B two", true);
    std::fs::create_dir_all(pc.root().join("Empty B/Deeper")).unwrap();
    let port = AsidePort::new();

    let start = pc.engine.start_sync_with(pc.root(), &port);

    assert_eq!(
        start,
        Err(SyncStartRefused::RegistrationFailed { code: Some(CODE) }),
        "past the binding"
    );
    assert_eq!(pc.db().owner().unwrap(), Some(other()), "B owns the local data now");
    assert_eq!(listing(&pc.root()), Vec::<String>::new());
    assert!(
        !aside(&pc).exists(),
        "nothing was kept aside: there was nothing to keep"
    );
}

/// (c) rc2/o3-attempt1: after a rebuilt state.db every file of unknown content is held for review ("held edit").
/// Adding another account is then refused with the choice, not the plain refusal, and keeping them aside completes
/// the switch with every byte.
#[test]
fn account_switch_rc2_held_edits_of_a_rebuilt_state_db_offer_the_choice() {
    let pc = Pc::new();
    a_signed_out(&pc, false);
    rusqlite::Connection::open(pc.state_db())
        .unwrap()
        .execute(
            "INSERT INTO held_edits (file_id, reason, fingerprint, detected_at) VALUES (?1, 'content_unknown', 'f', 1)",
            [IDS[0]],
        )
        .unwrap();
    let port = AsidePort::new();

    assert_eq!(
        pc.engine.start_sync_with(pc.root(), &port),
        Err(SyncStartRefused::LeftoverFilesOfAnotherAccount)
    );
    assert_eq!(
        std::fs::read(pc.root().join(SYNCED)).unwrap(),
        b"ten bytes!",
        "nothing moved unasked"
    );

    let done = pc.engine.keep_aside_and_switch_with(pc.root(), &port).unwrap();

    assert_eq!(done.kept, 1);
    assert_eq!(std::fs::read(aside(&pc).join(SYNCED)).unwrap(), b"ten bytes!");
    assert_eq!(
        done.start,
        Err(SyncStartRefused::RegistrationFailed { code: Some(CODE) })
    );
    assert_eq!(pc.db().owner().unwrap(), Some(other()));
}

// ---- RC3-1 (the Windows test machine, rc3/f1 f2 f3b): the choice was offered, and neither answer finished the switch ----

const FOLDER_OF_A: &str = "Folder of A";
const NEW_IN_FOLDER: &str = "Folder of A/leftover of A.txt";

/// Every line the engine logs while `body` runs on this thread.
fn logged<T>(body: impl FnOnce() -> T) -> (T, String) {
    use std::sync::Mutex;
    #[derive(Clone)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Capture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let buffer: Arc<Mutex<Vec<u8>>> = Arc::default();
    let writer = Capture(buffer.clone());
    // A second live dispatcher keeps callsite interest from being cached as "never" by another test's thread.
    let _second_dispatcher = tracing::Dispatch::new(tracing_subscriber::registry());
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .with_env_filter(tracing_subscriber::EnvFilter::new("beebeeb_engine=trace"))
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    let out = body();
    drop(guard);
    let text = String::from_utf8_lossy(&buffer.lock().unwrap()).into_owned();
    (out, text)
}

/// The VM's folder: A synced a folder with a file in it, paused sync, and made a new file in that folder (its upload
/// queued, its row `uploading`). A signed out; B signs in.
fn a_left_a_new_file_in_a_synced_folder(pc: &Pc) {
    a_signed_out(pc, false);
    row(&pc.db(), IDS[2], FOLDER_OF_A, true, 3);
    std::fs::create_dir_all(pc.root().join(FOLDER_OF_A)).unwrap();
    queued_while_paused(pc, IDS[1], NEW_IN_FOLDER, false);
}

/// (a) rc3/f1. Red at a5b14cd with the log lines of the VM: `keep aside: ... the switch goes on kept=1`, then
/// `the previous account's local data stays why="the folder's items stay"` and the plain refusal. The file was moved
/// aside, its queued upload was still in the database, and the Windows purge refuses while one is.
#[test]
fn account_switch_rc3_keep_aside_finishes_the_switch_with_its_queued_upload_in_the_database() {
    let pc = Pc::new();
    a_left_a_new_file_in_a_synced_folder(&pc);
    let port = AsidePort::new();
    assert_eq!(
        pc.engine.start_sync_with(pc.root(), &port),
        Err(SyncStartRefused::LeftoverFilesOfAnotherAccount)
    );

    let (done, log) = logged(|| pc.engine.keep_aside_and_switch_with(pc.root(), &port));
    let done = done.unwrap();

    assert!(log.contains("keep aside: the previous account's files are beside the sync folder"), "{log}");
    assert!(!log.contains("the folder's items stay"), "the removal went through: {log}");
    assert_eq!(done.kept, 1);
    assert_eq!(std::fs::read(aside(&pc).join(NEW_IN_FOLDER)).unwrap(), b"made by A while paused");
    assert_eq!(done.start, Err(SyncStartRefused::RegistrationFailed { code: Some(CODE) }), "past the binding");
    assert_eq!(pc.db().owner().unwrap(), Some(other()), "B owns the local data now");
    assert_eq!(listing(&pc.root()), Vec::<String>::new(), "nothing of A's is left in the sync folder");
    assert_eq!(a_key_in_its_slot(&pc), Some(A_KEY.to_vec()), "R4: A's key is kept in A's slot");
}

/// (b) rc3/f2, the engine's half: the confirmed discard keeps a copy, takes the file out and finishes the switch.
/// (On Windows it ended before that, at the kept folder's record: see `kept_folder_record_tests`.)
#[test]
fn account_switch_rc3_discard_finishes_the_switch_with_its_queued_upload_in_the_database() {
    let pc = Pc::new();
    a_left_a_new_file_in_a_synced_folder(&pc);
    let port = AsidePort::new();
    assert_eq!(
        pc.engine.start_sync_with(pc.root(), &port),
        Err(SyncStartRefused::LeftoverFilesOfAnotherAccount)
    );
    let ask = pc.engine.discard_and_switch_ask_with(&pc.root(), &port).unwrap();
    assert_eq!(ask.names, vec!["leftover of A.txt".to_string()]);

    let (done, log) = logged(|| pc.engine.discard_and_switch_with(&ask, pc.root(), &port));
    let done = done.unwrap();

    assert!(!log.contains("nothing was removed"), "{log}");
    assert!(!log.contains("the previous account's local data stays"), "{log}");
    assert_eq!(done.kept, 1);
    assert!(done.kept_in.is_dir(), "the copy is in Changes not sent");
    assert_eq!(done.start, Err(SyncStartRefused::RegistrationFailed { code: Some(CODE) }), "past the binding");
    assert_eq!(pc.db().owner().unwrap(), Some(other()), "B owns the local data now");
    assert_eq!(listing(&pc.root()), Vec::<String>::new(), "the file is out of the sync folder");
}

/// (c) rc3/f3b, the engine's half (the click itself never reached the engine: `frames/account/switcher.rs`): two
/// empty folders made while sync was paused, each queued. The start removes them and goes on, with no refusal.
#[test]
fn account_switch_rc3_only_empty_folders_finish_the_switch_with_their_queued_creates_in_the_database() {
    let pc = Pc::new();
    a_signed_out(&pc, false);
    queued_while_paused(&pc, IDS[1], "Empty B", true);
    queued_while_paused(&pc, IDS[2], "Empty B two", true);
    let port = AsidePort::new();

    let (start, log) = logged(|| pc.engine.start_sync_with(pc.root(), &port));

    assert!(!log.contains("the previous account's local data stays"), "{log}");
    assert_eq!(start, Err(SyncStartRefused::RegistrationFailed { code: Some(CODE) }), "past the binding");
    assert_eq!(pc.db().owner().unwrap(), Some(other()), "B owns the local data now");
    assert_eq!(listing(&pc.root()), Vec::<String>::new());
}

/// A removal the OS refuses after the answered work was forgotten leaves A's data A's: its synced rows and its owner
/// record stay, nothing of the queue comes back, and B is refused.
#[test]
fn account_switch_rc3_a_removal_refused_after_the_answer_leaves_the_synced_rows_with_their_account() {
    struct Refuses(AsidePort);
    impl SyncRootPort for Refuses {
        fn registration(&self, folder: &Path) -> Registration {
            self.0.registration(folder)
        }
        fn register(&self, folder: &Path) -> Result<(), Option<i32>> {
            self.0.register(folder)
        }
        fn engine_lock_held(&self, state_dir: &Path) -> bool {
            self.0.engine_lock_held(state_dir)
        }
        fn may_hold_bytes(&self, folder: &Path, path: &str) -> bool {
            self.0.may_hold_bytes(folder, path)
        }
        fn left_by_a_person(&self, folder: &Path, path: &str) -> bool {
            self.0.left_by_a_person(folder, path)
        }
        fn remove_synced_items(&self, db: &StateDb, _folder: &Path, _unasked: &[String]) -> Result<(), String> {
            purge_gate(db)?;
            Err("an item is open in another app".to_string())
        }
    }
    let pc = Pc::new();
    a_left_a_new_file_in_a_synced_folder(&pc);
    let port = Refuses(AsidePort::new());

    let done = pc.engine.keep_aside_and_switch_with(pc.root(), &port).unwrap();

    assert_eq!(done.start, Err(SyncStartRefused::OtherAccountsData), "B is refused");
    let db = pc.db();
    assert_eq!(db.owner().unwrap(), Some(a()), "the data is still A's");
    assert!(db.get_file_by_path(SYNCED).unwrap().is_some(), "A's synced file keeps its row");
    assert!(db.get_file_by_path(FOLDER_OF_A).unwrap().is_some(), "and its synced folder");
    assert!(db.get_file_by_path(NEW_IN_FOLDER).unwrap().is_none(), "the file kept aside has no row left");
    assert_eq!(db.work_waiting_for_a_person().unwrap(), None, "and no queued upload");
    assert!(pc.root().join(SYNCED).is_file(), "nothing synced was removed");
    assert_eq!(std::fs::read(aside(&pc).join(NEW_IN_FOLDER)).unwrap(), b"made by A while paused");
}

/// The same answers through the real Windows port (the real purge, the real file-system steps): what the Windows test machine ran.
#[cfg(target_os = "windows")]
mod on_windows {
    use super::*;
    use crate::sync_root_port::OsSyncRoots;

    /// Windows' own answers; only the registration is a test's.
    struct RealPort(OsSyncRoots);

    impl RealPort {
        fn new() -> Self {
            Self(OsSyncRoots::new(&crate::identity::TEST))
        }
    }

    impl SyncRootPort for RealPort {
        fn registration(&self, _folder: &Path) -> Registration {
            Registration::Nobody
        }
        fn register(&self, _folder: &Path) -> Result<(), Option<i32>> {
            Err(Some(CODE))
        }
        fn engine_lock_held(&self, _state_dir: &Path) -> bool {
            false
        }
        fn may_hold_bytes(&self, folder: &Path, path: &str) -> bool {
            self.0.may_hold_bytes(folder, path)
        }
        fn left_by_a_person(&self, folder: &Path, path: &str) -> bool {
            self.0.left_by_a_person(folder, path)
        }
        fn remove_synced_items(&self, db: &StateDb, folder: &Path, unasked: &[String]) -> Result<(), String> {
            self.0.remove_synced_items(db, folder, unasked)
        }
    }

    /// A synced a folder (an ordinary directory, as a folder made on this PC is left), paused sync and made a new
    /// file in it. A signed out; B signs in.
    fn a_left_a_new_file(pc: &Pc) {
        let db = pc.db();
        row(&db, IDS[2], FOLDER_OF_A, true, 3);
        db.set_owner(&a()).unwrap();
        drop(db);
        std::fs::create_dir_all(pc.root().join(FOLDER_OF_A)).unwrap();
        queued_while_paused(pc, IDS[1], NEW_IN_FOLDER, false);
        let vault = pc.engine.vault();
        vault.store_wrapped_master_key(SecretBytes::new_master_key(&A_KEY)).unwrap();
        vault.bind_vault_key_owner(a().user_id.as_deref().unwrap()).unwrap();
        pc.sign_in(&other());
    }

    #[test]
    fn account_switch_rc3_windows_keep_aside_finishes_the_switch() {
        let pc = Pc::new();
        a_left_a_new_file(&pc);
        let port = RealPort::new();
        assert_eq!(
            pc.engine.start_sync_with(pc.root(), &port),
            Err(SyncStartRefused::LeftoverFilesOfAnotherAccount)
        );

        let (done, log) = logged(|| pc.engine.keep_aside_and_switch_with(pc.root(), &port));
        let done = done.unwrap();

        assert!(!log.contains("the folder's items stay"), "{log}");
        assert_eq!(done.start, Err(SyncStartRefused::RegistrationFailed { code: Some(CODE) }), "{log}");
        assert_eq!(pc.db().owner().unwrap(), Some(other()));
        assert_eq!(listing(&pc.root()), Vec::<String>::new());
        assert_eq!(std::fs::read(aside(&pc).join(NEW_IN_FOLDER)).unwrap(), b"made by A while paused");
    }

    #[test]
    fn account_switch_rc3_windows_discard_finishes_the_switch() {
        let pc = Pc::new();
        a_left_a_new_file(&pc);
        let port = RealPort::new();
        assert_eq!(
            pc.engine.start_sync_with(pc.root(), &port),
            Err(SyncStartRefused::LeftoverFilesOfAnotherAccount)
        );
        let ask = pc.engine.discard_and_switch_ask_with(&pc.root(), &port).unwrap();

        let (done, log) = logged(|| pc.engine.discard_and_switch_with(&ask, pc.root(), &port));

        let done = done.unwrap_or_else(|why| panic!("discard and switch: nothing was removed why={why:?}\n{log}"));
        assert_eq!(done.start, Err(SyncStartRefused::RegistrationFailed { code: Some(CODE) }), "{log}");
        assert_eq!(pc.db().owner().unwrap(), Some(other()));
        assert_eq!(listing(&pc.root()), Vec::<String>::new());
    }

    #[test]
    fn account_switch_rc3_windows_only_empty_folders_finish_the_switch() {
        let pc = Pc::new();
        pc.db().set_owner(&a()).unwrap();
        queued_while_paused(&pc, IDS[1], "Empty B", true);
        queued_while_paused(&pc, IDS[2], "Empty B two", true);
        pc.sign_in(&other());
        let port = RealPort::new();

        let (start, log) = logged(|| pc.engine.start_sync_with(pc.root(), &port));

        assert_eq!(start, Err(SyncStartRefused::RegistrationFailed { code: Some(CODE) }), "{log}");
        assert_eq!(pc.db().owner().unwrap(), Some(other()));
        assert_eq!(listing(&pc.root()), Vec::<String>::new());
    }
}
