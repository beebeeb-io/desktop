//! w26-account-switch (v5 bug 6), at the engine: account A signed out of this PC (its local data stays, bound to it,
//! as a sign-out keeps it) and account B starts sync. B must start clean and never see A's names; whatever of A's
//! needs a person's answer (a queued change, a file of unknown content), or cannot be compared by user id, keeps the
//! refusal with every row and file as it was. The sync folder's items go through the port, here a fake. Every name is
//! invented.

use super::*;
use crate::config::DesktopConfig;

/// A name of account A's that account B must never see, in a row or on disk.
const A_NAME: &str = "Secret plans of A.docx";

/// What the fake does when the engine asks it to remove A's items from the sync folder.
#[derive(Clone, Copy, PartialEq)]
enum Removal {
    /// Delete every item under the folder (a fake of the Windows purge).
    Works,
    /// Refuse, deleting nothing.
    Fails,
}

/// Windows' answers as the account switch needs them: nobody's folder, a registration that refuses with [`CODE`] (so a
/// start that got past the binding is seen without an engine), and a removal that counts its asks.
struct SwitchPort {
    removal: Removal,
    removals: AtomicU32,
    registers: AtomicU32,
    /// w28: a file a person drops at `(path, bytes)` right after the engine was told nothing is there, before the
    /// removal runs (the window between the check and the purge).
    dropped_after_the_check: std::sync::Mutex<Option<(String, Vec<u8>)>>,
}

impl SwitchPort {
    fn new(removal: Removal) -> Self {
        Self {
            removal,
            removals: AtomicU32::new(0),
            registers: AtomicU32::new(0),
            dropped_after_the_check: std::sync::Mutex::new(None),
        }
    }
}

impl SyncRootPort for SwitchPort {
    fn registration(&self, _folder: &Path) -> Registration {
        Registration::Nobody
    }
    fn register(&self, _folder: &Path) -> Result<(), Option<i32>> {
        self.registers.fetch_add(1, Ordering::SeqCst);
        Err(Some(CODE))
    }
    fn engine_lock_held(&self, _state_dir: &Path) -> bool {
        false
    }
    /// A fake of Windows' answer: an item that is there with bytes in it.
    fn may_hold_bytes(&self, folder: &Path, path: &str) -> bool {
        let answer = std::fs::metadata(folder.join(path)).is_ok_and(|item| item.len() > 0);
        let mut dropped = self.dropped_after_the_check.lock().unwrap();
        if dropped.as_ref().is_some_and(|(at, _)| at == path) {
            let (at, bytes) = dropped.take().unwrap();
            std::fs::write(folder.join(at), bytes).unwrap();
        }
        answer
    }
    /// A fake of the Windows purge's contract (w28): it refuses, deleting nothing, when an item is at a path of
    /// `unasked` (the engine was told nothing was there, so whatever is now is not the first account's to delete);
    /// otherwise it deletes every item under the folder.
    fn remove_synced_items(&self, _db: &StateDb, folder: &Path, unasked: &[String]) -> Result<(), String> {
        self.removals.fetch_add(1, Ordering::SeqCst);
        if self.removal == Removal::Fails {
            return Err("refused".to_string());
        }
        if unasked.iter().any(|path| folder.join(path.trim_start_matches('/')).symlink_metadata().is_ok()) {
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

/// This PC after A synced and signed out: A's rows (the server named them), one of them on disk, A recorded as the
/// owner. Nothing is queued.
fn a_signed_out(pc: &Pc, owner: &Identity) {
    let db = pc.db();
    row(&db, IDS[0], A_NAME, false, 3);
    row(&db, IDS[1], "Taxes 2026", true, 3);
    db.set_owner(owner).unwrap();
    drop(db);
    // A row synced the usual way knows its content; a bare row is a file of unknown content (the guarded set).
    rusqlite::Connection::open(pc.state_db())
        .unwrap()
        .execute("UPDATE files SET local_hash = 'h', fetched_min_version = 0 WHERE item_kind = 'file'", [])
        .unwrap();
    std::fs::write(pc.root().join(A_NAME), b"ten bytes!").unwrap();
    std::fs::create_dir_all(pc.root().join("Taxes 2026")).unwrap();
}

fn start_with(pc: &Pc, port: &SwitchPort) -> Result<crate::app::SyncStart, SyncStartRefused> {
    pc.engine.start_sync_with(pc.root(), port)
}

fn nothing_of_a_left(pc: &Pc) {
    let rows = pc.contents().join("\n");
    assert!(!rows.contains(A_NAME) && !rows.contains("Taxes 2026"), "A's names in B's rows:\n{rows}");
    assert_eq!(listing(&pc.root()), Vec::<String>::new(), "A's items are gone from the folder");
}

/// The bug itself: B signs in over A's data and gets past the binding, to the registration, with A's rows and items
/// gone and B recorded as the owner.
#[test]
fn a_second_account_starts_clean_and_never_sees_the_first_ones_names() {
    let pc = Pc::new();
    a_signed_out(&pc, &owner());
    pc.sign_in(&other());
    let port = SwitchPort::new(Removal::Works);

    assert_eq!(start_with(&pc, &port), Err(SyncStartRefused::RegistrationFailed { code: Some(CODE) }));

    assert_eq!(port.removals.load(Ordering::SeqCst), 1);
    assert_eq!(port.registers.load(Ordering::SeqCst), 1, "the start went on to the registration");
    assert_eq!(pc.db().owner().unwrap(), Some(other()), "B owns the local data now");
    nothing_of_a_left(&pc);
}

/// A queued change of A's needs A's answer: nothing is removed, B stays refused, every row and file as it was.
#[test]
fn a_queued_change_of_the_first_account_keeps_the_refusal_and_deletes_nothing() {
    let pc = Pc::new();
    a_signed_out(&pc, &owner());
    pc.db()
        .enqueue_operation(&PendingOperation {
            base_version: Some(3),
            ..queued("op-edit", OperationKind::UploadVersion, IDS[0], None)
        })
        .unwrap();
    let (contents, files) = (pc.contents(), listing(&pc.root()));
    pc.sign_in(&other());
    let port = SwitchPort::new(Removal::Works);

    assert_eq!(start_with(&pc, &port), Err(SyncStartRefused::OtherAccountsData));

    assert_eq!(port.removals.load(Ordering::SeqCst), 0);
    assert_eq!(port.registers.load(Ordering::SeqCst), 0);
    assert_eq!(pc.db().owner().unwrap(), Some(owner()));
    assert_eq!(pc.contents(), contents);
    assert_eq!(listing(&pc.root()), files);
}

/// A file of unknown content is never deleted unasked (sign-out's D11 rule): the same refusal.
#[test]
fn a_file_of_unknown_content_keeps_the_refusal_and_deletes_nothing() {
    let pc = Pc::new();
    a_signed_out(&pc, &owner());
    pc.db().put_unknown_orphan(A_NAME, IDS[0], false).unwrap();
    let (contents, files) = (pc.contents(), listing(&pc.root()));
    pc.sign_in(&other());
    let port = SwitchPort::new(Removal::Works);

    assert_eq!(start_with(&pc, &port), Err(SyncStartRefused::OtherAccountsData));

    assert_eq!(port.removals.load(Ordering::SeqCst), 0);
    assert_eq!(pc.contents(), contents);
    assert_eq!(listing(&pc.root()), files);
}

/// A name of A's the database lists with no agreed content (a document made in the app: the row is there, the folder
/// holds no bytes of it).
const A_MADE_IN_THE_APP: &str = "Budget of A.docx";

/// w27 (the Windows test machine: a document A made in the app kept B out, "guarded file"): a file of unknown content with no bytes on
/// this PC holds nothing a person could lose, so it does not keep B out. It goes with A's other rows.
#[test]
fn a_file_of_unknown_content_with_no_bytes_on_this_pc_does_not_keep_the_second_account_out() {
    let pc = Pc::new();
    a_signed_out(&pc, &owner());
    row(&pc.db(), IDS[2], A_MADE_IN_THE_APP, false, 1);
    pc.sign_in(&other());
    let port = SwitchPort::new(Removal::Works);

    assert_eq!(start_with(&pc, &port), Err(SyncStartRefused::RegistrationFailed { code: Some(CODE) }));

    assert_eq!(port.removals.load(Ordering::SeqCst), 1);
    assert_eq!(pc.db().owner().unwrap(), Some(other()), "B owns the local data now");
    assert!(!pc.contents().join("\n").contains(A_MADE_IN_THE_APP), "A's name is in B's rows");
    nothing_of_a_left(&pc);
}

/// w28 (the window w27 left): a file made at a guarded path after the check said "no bytes" and before the removal
/// is not A's synced item. The removal refuses, B stays refused, and the file keeps every byte.
#[test]
fn a_file_made_between_the_check_and_the_removal_survives_and_the_second_account_is_refused() {
    let pc = Pc::new();
    a_signed_out(&pc, &owner());
    row(&pc.db(), IDS[2], A_MADE_IN_THE_APP, false, 1);
    let contents = pc.contents();
    let mut files = listing(&pc.root());
    pc.sign_in(&other());
    let port = SwitchPort::new(Removal::Works);
    *port.dropped_after_the_check.lock().unwrap() = Some((A_MADE_IN_THE_APP.to_string(), b"typed by B just now".to_vec()));

    assert_eq!(start_with(&pc, &port), Err(SyncStartRefused::OtherAccountsData));

    assert_eq!(port.removals.load(Ordering::SeqCst), 1, "the removal was asked, and refused");
    assert_eq!(port.registers.load(Ordering::SeqCst), 0);
    assert_eq!(
        std::fs::read(pc.root().join(A_MADE_IN_THE_APP)).unwrap(),
        b"typed by B just now",
        "the file made during the switch is untouched"
    );
    assert_eq!(pc.db().owner().unwrap(), Some(owner()), "no row was cleared");
    assert_eq!(pc.contents(), contents);
    files.push(A_MADE_IN_THE_APP.to_string());
    files.sort();
    let mut now = listing(&pc.root());
    now.sort();
    assert_eq!(now, files, "nothing else was removed either");
}

/// w29 (the Windows test machine: A's old CLI-replaced files, never opened here, kept B out 5/5): a file of unknown content whose item
/// is there but holds no bytes (a cloud placeholder nothing was fetched into) is a synced item like any other. The
/// removal is asked to delete it, and B starts.
#[test]
fn a_file_of_unknown_content_that_is_an_empty_placeholder_goes_with_the_rest() {
    let pc = Pc::new();
    a_signed_out(&pc, &owner());
    row(&pc.db(), IDS[2], A_MADE_IN_THE_APP, false, 1);
    std::fs::write(pc.root().join(A_MADE_IN_THE_APP), b"").unwrap();
    pc.sign_in(&other());
    let port = SwitchPort::new(Removal::Works);

    assert_eq!(start_with(&pc, &port), Err(SyncStartRefused::RegistrationFailed { code: Some(CODE) }));

    assert_eq!(pc.db().owner().unwrap(), Some(other()), "B owns the local data now");
    nothing_of_a_left(&pc);
}

/// The same row with bytes in the folder may be A's unsent edit: never deleted unasked (D11). B stays refused, every
/// row and file as it was.
#[test]
fn a_file_of_unknown_content_with_bytes_on_this_pc_keeps_the_refusal_and_deletes_nothing() {
    let pc = Pc::new();
    a_signed_out(&pc, &owner());
    row(&pc.db(), IDS[2], A_MADE_IN_THE_APP, false, 1);
    std::fs::write(pc.root().join(A_MADE_IN_THE_APP), b"typed by A, not sent").unwrap();
    let (contents, files) = (pc.contents(), listing(&pc.root()));
    pc.sign_in(&other());
    let port = SwitchPort::new(Removal::Works);

    assert_eq!(start_with(&pc, &port), Err(SyncStartRefused::UnsentFileOfAnotherAccount));

    assert_eq!(port.removals.load(Ordering::SeqCst), 0);
    assert_eq!(port.registers.load(Ordering::SeqCst), 0);
    assert_eq!(pc.db().owner().unwrap(), Some(owner()));
    assert_eq!(pc.contents(), contents);
    assert_eq!(listing(&pc.root()), files);
}

/// Only two user ids that differ prove another account: an owner recorded by an email alone is task 2267's case (the
/// proof, nothing deleted).
#[test]
fn an_owner_known_by_email_alone_is_never_removed() {
    let pc = Pc::new();
    a_signed_out(&pc, &Identity::new(None, Some("owner@example.eu")));
    let (contents, files) = (pc.contents(), listing(&pc.root()));
    pc.sign_in(&other());
    let port = SwitchPort::new(Removal::Works);

    assert_eq!(start_with(&pc, &port), Err(SyncStartRefused::OtherAccountsData));

    assert_eq!(port.removals.load(Ordering::SeqCst), 0);
    assert_eq!(pc.contents(), contents);
    assert_eq!(listing(&pc.root()), files);
}

/// A removal the folder refuses leaves A's owner in place: B stays refused, no row is cleared.
#[test]
fn a_removal_that_fails_keeps_the_refusal_and_the_owner() {
    let pc = Pc::new();
    a_signed_out(&pc, &owner());
    let (contents, files) = (pc.contents(), listing(&pc.root()));
    pc.sign_in(&other());
    let port = SwitchPort::new(Removal::Fails);

    assert_eq!(start_with(&pc, &port), Err(SyncStartRefused::OtherAccountsData));

    assert_eq!(port.removals.load(Ordering::SeqCst), 1);
    assert_eq!(port.registers.load(Ordering::SeqCst), 0);
    assert_eq!(pc.db().owner().unwrap(), Some(owner()));
    assert_eq!(pc.contents(), contents);
    assert_eq!(listing(&pc.root()), files);
}

/// A signs back in: the data is A's own, nothing is removed, the start goes on.
#[test]
fn the_same_account_signing_back_in_keeps_everything() {
    let pc = Pc::new();
    a_signed_out(&pc, &owner());
    let (contents, files) = (pc.contents(), listing(&pc.root()));
    pc.sign_in(&owner());
    let port = SwitchPort::new(Removal::Works);

    assert_eq!(start_with(&pc, &port), Err(SyncStartRefused::RegistrationFailed { code: Some(CODE) }));

    assert_eq!(port.removals.load(Ordering::SeqCst), 0);
    assert_eq!(pc.contents(), contents);
    assert_eq!(listing(&pc.root()), files);
}

/// A chosen backup folder with its tag, as `app::bindings::backup_folders` makes one.
fn chosen(name: &str, owner: &Identity) -> crate::backup::sources::ChosenFolder {
    crate::backup::sources::ChosenFolder {
        name: name.to_string(),
        path: PathBuf::from(format!("C:/Invented/{name}")),
        owner: crate::app::bindings::backup_folders::owner_tag(owner),
        folder_id: Some(format!("folder-of-{name}")),
    }
}

/// desktop.toml as a session leaves it: `who`'s chosen folder (with its switch), its selective-sync ids, and the address
/// the sign-in form prefills.
fn settings_of(who: &Identity, name: &str) {
    let path = DesktopConfig::path_for(&TEST).unwrap();
    DesktopConfig::update_at(&path, |config| {
        config.backup_chosen_folders.push(chosen(name, who));
        config.known_folder_backup.push(format!("custom:{name}"));
        config.excluded_folder_ids = Some(vec![format!("vault-folder-of-{name}")]);
        config.last_signed_in_email = who.email.clone();
        (true, ())
    })
    .unwrap();
}

fn settings_now() -> DesktopConfig {
    DesktopConfig::load_for(&TEST).unwrap()
}

/// The account's entries in desktop.toml (its chosen backup folders and their switches, its address) go with its data:
/// B's start leaves nothing of A's there, and every entry of B's own stays.
#[test]
fn the_first_accounts_entries_in_the_settings_file_go_with_its_data() {
    let a = Identity::new(Some("user-w27-settings-a"), Some("w27-settings-a@example.eu"));
    let b = Identity::new(Some("user-w27-settings-b"), Some("w27-settings-b@example.eu"));
    let pc = Pc::new();
    a_signed_out(&pc, &a);
    settings_of(&a, "w27-a-photos");
    let path = DesktopConfig::path_for(&TEST).unwrap();
    DesktopConfig::update_at(&path, |config| {
        config.backup_chosen_folders.push(chosen("w27-b-music", &b));
        config.known_folder_backup.push("custom:w27-b-music".to_string());
        (true, ())
    })
    .unwrap();
    pc.sign_in(&b);
    let port = SwitchPort::new(Removal::Works);

    assert_eq!(start_with(&pc, &port), Err(SyncStartRefused::RegistrationFailed { code: Some(CODE) }));

    let config = settings_now();
    let names: Vec<&str> = config.backup_chosen_folders.iter().map(|f| f.name.as_str()).collect();
    assert!(!names.contains(&"w27-a-photos"), "A's chosen folder is still in desktop.toml: {names:?}");
    assert!(names.contains(&"w27-b-music"), "B's own chosen folder stays: {names:?}");
    assert!(!config.known_folder_backup.iter().any(|key| key == "custom:w27-a-photos"), "A's switch stays");
    assert!(config.known_folder_backup.iter().any(|key| key == "custom:w27-b-music"), "B's switch stays");
    assert_ne!(config.last_signed_in_email.as_deref(), Some("w27-settings-a@example.eu"), "A's address stays");
}

/// An account whose work waits for a person keeps its entries in desktop.toml as it keeps everything else.
#[test]
fn a_refused_switch_keeps_the_first_accounts_entries_in_the_settings_file() {
    let a = Identity::new(Some("user-w27-kept-a"), Some("w27-kept-a@example.eu"));
    let pc = Pc::new();
    a_signed_out(&pc, &a);
    pc.db().put_unknown_orphan(A_NAME, IDS[0], false).unwrap();
    settings_of(&a, "w27-kept-photos");
    pc.sign_in(&other());
    let port = SwitchPort::new(Removal::Works);

    assert_eq!(start_with(&pc, &port), Err(SyncStartRefused::OtherAccountsData));

    let names: Vec<String> = settings_now().backup_chosen_folders.into_iter().map(|f| f.name).collect();
    assert!(names.iter().any(|name| name == "w27-kept-photos"), "{names:?}");
}

/// w28's the Windows test machine probe, at the engine with the REAL Windows port (the purge, the exclusive handles, NTFS): the same
/// switch as above, but the file a person makes lands inside the real window between the check and the purge, in a
/// folder the purge walks. Windows only (the unit run on the Windows test machine reaches it).
#[cfg(target_os = "windows")]
mod real_purge {
    use super::*;
    use crate::sync_root_port::OsSyncRoots;

    const FOLDER: &str = "Plans of A";
    const MADE_IN_THE_APP: &str = "Plans of A/Budget of A.docx";

    /// The OS's own answers (`OsSyncRoots`), but for a file dropped at `MADE_IN_THE_APP` the moment the check has
    /// answered "nothing is there": exactly what a person's save does between the check and the purge.
    struct DropsAfterTheCheck {
        os: OsSyncRoots,
        drop_bytes: &'static [u8],
        dropped: AtomicU32,
        registers: AtomicU32,
    }

    impl SyncRootPort for DropsAfterTheCheck {
        fn registration(&self, _folder: &Path) -> Registration {
            Registration::Nobody
        }
        fn register(&self, _folder: &Path) -> Result<(), Option<i32>> {
            self.registers.fetch_add(1, Ordering::SeqCst);
            Err(Some(CODE))
        }
        fn engine_lock_held(&self, _state_dir: &Path) -> bool {
            false
        }
        fn may_hold_bytes(&self, folder: &Path, path: &str) -> bool {
            let answer = self.os.may_hold_bytes(folder, path);
            if !answer && path.trim_start_matches('/') == MADE_IN_THE_APP && self.dropped.fetch_add(1, Ordering::SeqCst) == 0 {
                std::fs::write(folder.join(MADE_IN_THE_APP), self.drop_bytes).unwrap();
            }
            answer
        }
        fn remove_synced_items(&self, db: &StateDb, folder: &Path, unasked: &[String]) -> Result<(), String> {
            self.os.remove_synced_items(db, folder, unasked)
        }
    }

    #[test]
    fn a_file_dropped_in_the_folder_during_the_switch_survives_and_the_second_account_is_refused() {
        let pc = Pc::new();
        let db = pc.db();
        row(&db, IDS[1], FOLDER, true, 3);
        row(&db, IDS[2], MADE_IN_THE_APP, false, 1);
        db.set_owner(&owner()).unwrap();
        drop(db);
        std::fs::create_dir(pc.root().join(FOLDER)).unwrap();
        let contents = pc.contents();
        pc.sign_in(&other());
        let port = DropsAfterTheCheck {
            os: OsSyncRoots::new(&crate::identity::TEST),
            drop_bytes: b"made during the switch",
            dropped: AtomicU32::new(0),
            registers: AtomicU32::new(0),
        };

        let started = pc.engine.start_sync_with(pc.root(), &port);

        assert_eq!(port.dropped.load(Ordering::SeqCst), 1, "the file was dropped after the check");
        assert_eq!(
            std::fs::read(pc.root().join(MADE_IN_THE_APP)).unwrap(),
            b"made during the switch",
            "the file made during the switch keeps every byte"
        );
        assert!(pc.root().join(FOLDER).is_dir());
        assert_eq!(started, Err(SyncStartRefused::OtherAccountsData), "the second account is refused");
        assert_eq!(port.registers.load(Ordering::SeqCst), 0);
        assert_eq!(pc.db().owner().unwrap(), Some(owner()), "no row was cleared");
        assert_eq!(pc.contents(), contents);
    }
}
