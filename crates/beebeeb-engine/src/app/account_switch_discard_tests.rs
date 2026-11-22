//! dc-111, "Discard and switch", at the engine: account A signed out leaving files that may hold changes it never
//! sent, so B's start is refused (`UnsentFileOfAnotherAccount`). The ask only reads (a Cancel changes nothing); the
//! discard keeps a copy of each file in the kept folder BEFORE anything is purged, then switches in 014's order and
//! forgets neither account's key slot (R4). Every name is invented.

use super::*;
use crate::keychain::{AuthSecretStore, SecretBytes};

const UNSENT: [&str; 6] = ["One.txt", "Two.txt", "Three.txt", "Four.txt", "Five.txt", "Six.txt"];
const A_KEY: [u8; 32] = [0xA1; 32];
const B_KEY: [u8; 32] = [0xB2; 32];

/// Windows' answers for a switch: nobody's folder, a registration that refuses with [`CODE`], a byte answer by size,
/// and a purge that, when it runs, notes whether every unsent file's copy was in the kept folder already.
struct DiscardPort {
    kept_root: PathBuf,
    removals: AtomicU32,
    copies_at_purge: std::sync::Mutex<Option<Vec<String>>>,
}

impl DiscardPort {
    fn new(pc: &Pc) -> Self {
        Self {
            kept_root: kept_root(pc),
            removals: AtomicU32::new(0),
            copies_at_purge: std::sync::Mutex::new(None),
        }
    }
}

impl SyncRootPort for DiscardPort {
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
    fn remove_synced_items(&self, _db: &StateDb, folder: &Path, unasked: &[String]) -> Result<(), String> {
        self.removals.fetch_add(1, Ordering::SeqCst);
        *self.copies_at_purge.lock().unwrap() = Some(kept_files(&self.kept_root));
        if unasked.iter().any(|path| folder.join(path).symlink_metadata().is_ok()) {
            return Err("an item that was not there".to_string());
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

fn kept_root(pc: &Pc) -> PathBuf {
    pc.engine
        .config
        .state_dir
        .join(crate::sign_out::kept_names::KEPT_FOLDER_NAME)
}

/// Every file under the kept folder, as `name=bytes`, sorted.
fn kept_files(root: &Path) -> Vec<String> {
    fn walk(at: &Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(at) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else {
                let bytes = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).into_owned();
                out.push(format!("{}={bytes}", path.file_name().unwrap().to_string_lossy()));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out.sort();
    out
}

fn expected_copies(count: usize) -> Vec<String> {
    let mut want: Vec<String> = UNSENT[..count]
        .iter()
        .map(|name| format!("{name}=typed by A: {name}"))
        .collect();
    want.sort();
    want
}

/// A synced and signed out: a synced row, then `count` files of unknown content with A's typing in the folder; A's
/// key in the working slot and in A's own slot. B signs in, with B's key in B's own slot.
fn a_left_unsent(pc: &Pc, a: &Identity, count: usize) {
    let db = pc.db();
    row(&db, IDS[0], "Synced.txt", false, 3);
    db.set_owner(a).unwrap();
    drop(db);
    rusqlite::Connection::open(pc.state_db())
        .unwrap()
        .execute(
            "UPDATE files SET local_hash = 'h', fetched_min_version = 0 WHERE item_kind = 'file'",
            [],
        )
        .unwrap();
    std::fs::write(pc.root().join("Synced.txt"), b"ten bytes!").unwrap();
    for (n, name) in UNSENT[..count].iter().enumerate() {
        row(
            &pc.db(),
            &format!("0b000000-0000-4000-8000-00000000000{n}"),
            name,
            false,
            1,
        );
        std::fs::write(pc.root().join(name), format!("typed by A: {name}")).unwrap();
    }
    let vault = pc.engine.vault();
    vault
        .store_wrapped_master_key(SecretBytes::new_master_key(&A_KEY))
        .unwrap();
    vault.bind_vault_key_owner(a.user_id.as_deref().unwrap()).unwrap();
    pc.store()
        .save_account_key(other().user_id.as_deref().unwrap(), SecretBytes::new_master_key(&B_KEY))
        .unwrap();
    pc.sign_in(&other());
}

impl Pc {
    fn store(&self) -> <MemoryCredentials as CredentialStores>::Store {
        self.engine
            .stores
            .for_account(self.engine.identity, &self.engine.config.account_id)
    }

    fn account_key(&self, who: &Identity) -> Option<Vec<u8>> {
        self.store()
            .load_account_key(who.user_id.as_deref().unwrap())
            .unwrap()
            .map(|key| key.expose_for_crypto().to_vec())
    }
}

fn a_with(tag: &str) -> Identity {
    Identity::new(
        Some(&format!("user-dc111-{tag}")),
        Some(&format!("dc111-{tag}@example.eu")),
    )
}

/// The refusal this task starts from, so the tests below are about the state a person really meets.
fn refused_for_unsent(pc: &Pc, port: &DiscardPort) {
    assert_eq!(
        pc.engine.start_sync_with(pc.root(), port),
        Err(SyncStartRefused::UnsentFileOfAnotherAccount)
    );
}

/// The confirmation names what is lost: every file counted, the first five by name (never a path).
#[test]
fn the_ask_counts_every_unsent_file_and_names_the_first_five() {
    let pc = Pc::new();
    a_left_unsent(&pc, &a_with("names"), 6);
    let port = DiscardPort::new(&pc);
    refused_for_unsent(&pc, &port);

    let ask = pc.engine.discard_and_switch_ask_with(&pc.root(), &port).unwrap();

    assert_eq!(ask.files, 6);
    assert_eq!(ask.names, UNSENT[..5].iter().map(|n| n.to_string()).collect::<Vec<_>>());
    assert_eq!(ask.more(), 1);
}

/// Cancel is a no-op: asking and then dropping the answer leaves every row, item, key, setting and the kept folder
/// as they were, and B is still refused.
#[test]
fn cancel_changes_nothing() {
    let pc = Pc::new();
    let a = a_with("cancel");
    a_left_unsent(&pc, &a, 2);
    let port = DiscardPort::new(&pc);
    refused_for_unsent(&pc, &port);
    let (contents, files) = (pc.contents(), listing(&pc.root()));

    let ask = pc.engine.discard_and_switch_ask_with(&pc.root(), &port).unwrap();
    drop(ask);

    assert_eq!(pc.contents(), contents);
    assert_eq!(listing(&pc.root()), files);
    assert_eq!(kept_files(&kept_root(&pc)), Vec::<String>::new(), "nothing kept");
    assert_eq!(port.removals.load(Ordering::SeqCst), 0);
    assert_eq!(
        pc.engine.vault().vault_key_owner().unwrap(),
        a.user_id.clone(),
        "A's key as it was"
    );
    refused_for_unsent(&pc, &port);
}

/// The red test of the order: every unsent file's copy is in the kept folder, with A's bytes, before the purge
/// starts; the switch then goes on (B owns the data, A's names gone) and the kept folder is recorded.
#[test]
fn every_copy_exists_before_the_purge_and_the_switch_goes_on() {
    let pc = Pc::new();
    a_left_unsent(&pc, &a_with("order"), 3);
    let port = DiscardPort::new(&pc);
    refused_for_unsent(&pc, &port);
    let ask = pc.engine.discard_and_switch_ask_with(&pc.root(), &port).unwrap();

    let done = pc.engine.discard_and_switch_with(&ask, pc.root(), &port).unwrap();

    assert_eq!(
        port.copies_at_purge.lock().unwrap().clone(),
        Some(expected_copies(3)),
        "the copies were there when the purge began"
    );
    assert_eq!(done.kept, 3);
    assert_eq!(
        done.start,
        Err(SyncStartRefused::RegistrationFailed { code: Some(CODE) })
    );
    assert_eq!(pc.db().owner().unwrap(), Some(other()), "B owns the local data now");
    assert_eq!(listing(&pc.root()), Vec::<String>::new());
    assert_eq!(kept_files(&kept_root(&pc)), expected_copies(3), "the copies stay");
    // desktop.toml is one file for every test, so only that a folder was recorded is asserted here.
    assert!(
        crate::config::DesktopConfig::load_for(&TEST)
            .unwrap()
            .kept_unsent_folder
            .is_some()
    );
}

/// R4: a discard-and-switch is a switch. A's key stays in A's own slot and B's in B's.
#[test]
fn neither_accounts_key_slot_is_forgotten() {
    let pc = Pc::new();
    let a = a_with("slots");
    a_left_unsent(&pc, &a, 1);
    // A's slot empty, as on a PC from before R4: the working key is A's only copy, so it is kept there first.
    pc.store().delete_account_key(a.user_id.as_deref().unwrap()).unwrap();
    let port = DiscardPort::new(&pc);
    let ask = pc.engine.discard_and_switch_ask_with(&pc.root(), &port).unwrap();

    pc.engine.discard_and_switch_with(&ask, pc.root(), &port).unwrap();

    assert_eq!(pc.account_key(&a), Some(A_KEY.to_vec()), "A's slot holds A's key");
    assert_eq!(pc.account_key(&other()), Some(B_KEY.to_vec()), "B's slot holds B's key");
}

/// The discard does what was confirmed or nothing: a file added since the ask refuses, nothing kept or removed.
#[test]
fn a_file_added_since_the_ask_refuses_and_changes_nothing() {
    let pc = Pc::new();
    a_left_unsent(&pc, &a_with("changed"), 2);
    let port = DiscardPort::new(&pc);
    let ask = pc.engine.discard_and_switch_ask_with(&pc.root(), &port).unwrap();
    row(&pc.db(), IDS[2], "Late.txt", false, 1);
    std::fs::write(pc.root().join("Late.txt"), b"typed later").unwrap();
    let (contents, files) = (pc.contents(), listing(&pc.root()));

    assert_eq!(
        pc.engine.discard_and_switch_with(&ask, pc.root(), &port),
        Err(crate::account_binding::DiscardRefused::Changed)
    );

    assert_eq!(pc.contents(), contents);
    assert_eq!(listing(&pc.root()), files);
    assert_eq!(kept_files(&kept_root(&pc)), Vec::<String>::new());
    assert_eq!(port.removals.load(Ordering::SeqCst), 0);
}

/// Never over a name in the kept folder: a file already there keeps its bytes, and the copy takes `name (2)`.
#[test]
fn a_name_already_in_the_kept_folder_is_never_overwritten() {
    let pc = Pc::new();
    a_left_unsent(&pc, &a_with("never-over"), 1);
    let port = DiscardPort::new(&pc);
    let ask = pc.engine.discard_and_switch_ask_with(&pc.root(), &port).unwrap();
    let event = kept_root(&pc).join(crate::sign_out::kept_names::event_folder_name(chrono::Local::now()));
    std::fs::create_dir_all(&event).unwrap();
    std::fs::write(event.join(UNSENT[0]), b"kept before").unwrap();

    let done = pc.engine.discard_and_switch_with(&ask, pc.root(), &port).unwrap();

    assert_eq!(std::fs::read(event.join(UNSENT[0])).unwrap(), b"kept before");
    assert_ne!(done.kept_in, event, "a new event folder");
    let mut want = expected_copies(1);
    want.push(format!("{}=kept before", UNSENT[0]));
    want.sort();
    assert_eq!(kept_files(&kept_root(&pc)), want);
}

/// 079b: A's cached thumbnails are gone once the switch to B went through.
#[test]
fn a_switch_removes_the_previous_accounts_thumb_cache() {
    let pc = Pc::new();
    a_left_unsent(&pc, &a_with("thumb"), 1);
    let port = DiscardPort::new(&pc);
    let master = beebeeb_core::kdf::MasterKey::from_bytes([7; 32]);
    let cache = crate::thumb_cache::ThumbCache::new(
        &pc.engine.config.state_dir,
        &pc.engine.config.account_id,
        &master,
        crate::thumb_cache::DEFAULT_CAP_MB,
    );
    cache
        .put("0190d8a6-1b2c-7d3e-8f40-0123456789ab", 1, 2, 2, &[1u8; 16])
        .unwrap();
    assert!(cache.get("0190d8a6-1b2c-7d3e-8f40-0123456789ab").is_some());
    let ask = pc.engine.discard_and_switch_ask_with(&pc.root(), &port).unwrap();
    pc.engine.discard_and_switch_with(&ask, pc.root(), &port).unwrap();
    assert!(
        cache.get("0190d8a6-1b2c-7d3e-8f40-0123456789ab").is_none(),
        "A's thumbnail is gone"
    );
}
