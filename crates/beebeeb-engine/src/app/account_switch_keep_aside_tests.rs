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
const LEFT: [(&str, &str); 2] = [
    ("Drafts/Plan of A.txt", "typed by A, never sent"),
    ("Notes of A.txt", "A's own notes"),
];

/// Windows' answers for this switch: a file is a synced copy only when it is [`SYNCED`]; the removal refuses,
/// deleting nothing, at any other file and at a folder no row names (the Windows purge's rule), else it deletes
/// every item.
struct AsidePort {
    removals: AtomicU32,
}

impl AsidePort {
    fn new() -> Self {
        Self {
            removals: AtomicU32::new(0),
        }
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
        for path in listing(folder) {
            let path = path.replace('\\', "/");
            let on_the_list = db.get_file_by_path(&path).unwrap().is_some();
            let ours = if folder.join(&path).is_dir() {
                on_the_list
            } else {
                path == SYNCED
            };
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
        Ok(())
    }
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
        .execute(
            "UPDATE files SET local_hash = 'h', fetched_min_version = 0 WHERE item_kind = 'file'",
            [],
        )
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
    vault
        .store_wrapped_master_key(SecretBytes::new_master_key(&A_KEY))
        .unwrap();
    vault.bind_vault_key_owner(a().user_id.as_deref().unwrap()).unwrap();
    pc.sign_in(&other());
}

/// The folder "Keep them aside" makes for A today, beside the sync folder.
fn aside(pc: &Pc) -> PathBuf {
    let day = chrono::Local::now().date_naive().format("%Y-%m-%d");
    pc.temp
        .path()
        .join(format!("Beebeeb files from rc-b-a@example.eu ({day})"))
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

/// The red test (VM 401, RC1): files A left refuse B's start with a reason that has a fix, not the plain
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
    assert_eq!(
        listing(&aside(&pc)).len(),
        3,
        "the two files and A's folder, nothing else"
    );
    assert_eq!(
        done.start,
        Err(SyncStartRefused::RegistrationFailed { code: Some(CODE) }),
        "past the binding"
    );
    assert_eq!(pc.db().owner().unwrap(), Some(other()), "B owns the local data now");
    assert_eq!(
        listing(&pc.root()),
        Vec::<String>::new(),
        "nothing of A's is left in the sync folder"
    );
    assert_eq!(
        a_key_in_its_slot(&pc),
        Some(A_KEY.to_vec()),
        "R4: A's key is kept in A's slot"
    );
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
    assert_eq!(
        std::fs::read(aside(&pc).join("Notes of A.txt")).unwrap(),
        b"kept before"
    );
    assert_eq!(listing(&aside(&pc)).len(), 1);
    assert_eq!(
        std::fs::read_to_string(done.kept_in.join("Notes of A.txt")).unwrap(),
        "A's own notes"
    );
}

/// The discard of 111 takes the files A left too: a copy of each in the kept folder first, then the switch.
#[test]
fn the_discard_keeps_a_copy_of_each_left_file_and_switches() {
    let pc = Pc::new();
    a_signed_out(&pc, true);
    let port = AsidePort::new();
    let ask = pc.engine.discard_and_switch_ask_with(&pc.root(), &port).unwrap();
    assert_eq!(
        ask.names,
        vec!["Notes of A.txt".to_string(), "Plan of A.txt".to_string()],
        "names, never paths"
    );

    let done = pc.engine.discard_and_switch_with(&ask, pc.root(), &port).unwrap();

    assert_eq!(done.kept, 2);
    assert!(
        done.kept_in.starts_with(
            pc.engine
                .config
                .state_dir
                .join(crate::sign_out::kept_names::KEPT_FOLDER_NAME)
        )
    );
    let mut kept: Vec<String> = listing(&done.kept_in)
        .into_iter()
        .filter(|path| done.kept_in.join(path).is_file())
        .map(|path| std::fs::read_to_string(done.kept_in.join(path)).unwrap())
        .collect();
    kept.sort();
    assert_eq!(
        kept,
        vec!["A's own notes".to_string(), "typed by A, never sent".to_string()]
    );
    assert_eq!(
        done.start,
        Err(SyncStartRefused::RegistrationFailed { code: Some(CODE) })
    );
    assert_eq!(listing(&pc.root()), Vec::<String>::new());
    assert!(
        !aside(&pc).exists(),
        "the discard makes no folder beside the sync folder"
    );
}

/// With nothing of a person's in the folder there is nothing to keep aside: asked anyway, it changes nothing, and
/// the plain switch needs no answer.
#[test]
fn nothing_left_means_nothing_is_moved() {
    let pc = Pc::new();
    a_signed_out(&pc, false);
    let port = AsidePort::new();
    let (contents, files) = (pc.contents(), listing(&pc.root()));

    assert_eq!(
        pc.engine.keep_aside_and_switch_with(pc.root(), &port),
        Err(DiscardRefused::NothingToDiscard)
    );

    assert_eq!((pc.contents(), listing(&pc.root())), (contents, files));
    assert_eq!(port.removals.load(Ordering::SeqCst), 0);
    assert!(!aside(&pc).exists());
    let start = pc.engine.start_sync_with(pc.root(), &port);
    assert_eq!(start, Err(SyncStartRefused::RegistrationFailed { code: Some(CODE) }));
    assert_eq!(pc.db().owner().unwrap(), Some(other()));
}

/// Other work of A's (a queued change) needs A: the refusal stays the plain one and keep aside moves nothing.
#[test]
fn a_queued_change_of_the_previous_account_is_not_kept_aside() {
    let pc = Pc::new();
    a_signed_out(&pc, true);
    pc.db()
        .enqueue_operation(&PendingOperation {
            base_version: Some(3),
            ..queued("op-rc-b", OperationKind::UploadVersion, IDS[0], None)
        })
        .unwrap();
    let port = AsidePort::new();
    let files = listing(&pc.root());

    assert_eq!(
        pc.engine.start_sync_with(pc.root(), &port),
        Err(SyncStartRefused::OtherAccountsData)
    );
    assert_eq!(
        pc.engine.keep_aside_and_switch_with(pc.root(), &port),
        Err(DiscardRefused::OtherWork)
    );

    assert_eq!(listing(&pc.root()), files);
    assert!(!aside(&pc).exists());
}

/// dc-175 (VM 401): A left only empty folders no row names. They are nothing to keep: the start removes them and the
/// switch completes, with no refusal for a person to answer. A folder holding a synced copy is never removed this way.
#[test]
fn empty_folders_the_previous_account_left_do_not_stop_the_switch() {
    let pc = Pc::new();
    a_signed_out(&pc, false);
    std::fs::create_dir_all(pc.root().join("Empty of A/Deeper")).unwrap();
    std::fs::create_dir_all(pc.root().join("Also empty")).unwrap();
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
