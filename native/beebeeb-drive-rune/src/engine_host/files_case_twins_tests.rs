//! Two server items at one place on disk (`drive_v2::case_place`) on the real engine's Files presses: Open (and Open in
//! another app…), Show in File Explorer and Open in File Explorer hand the system a row's place only when the entry
//! there is that row's. Linux tells case apart, so a test plays a volume that does not (`with_rule_for_test`): the sync
//! folder holds the ONE entry such a volume would hold (`Invented Case <tag>.txt`), the rows say `Invented Case …` and
//! `invented case …`. Two rows with the very same name are one entry on every volume: refused under any rule. Nothing
//! here starts a program (the starts are recording stand-ins). Every name is invented; each test runs in a sync
//! folder of its own (`Engine::use_running_root_for_test`), never the person's home.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;

use beebeeb_engine::account_binding::Identity;
use beebeeb_engine::app::{Engine, EngineConfig};
use beebeeb_engine::drive_v2::case_place::{CaseRule, with_rule_for_test};
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::runner::EngineRunner;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::state_db::{FileEntry, FileStatus, ItemKind, StateDb};
use beebeeb_engine::sync_view::Start;
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, start_fake};
use zeroize::Zeroizing;

use super::*;
use crate::engine_host::EngineFilesSource;
use crate::frames::files::source::FilesSource;

const PASSWORD: &str = "an invented password for case twins";
/// `Invented Case <tag>.txt`: its spelling is the entry's (its placeholder was made first).
const UPPER: &str = "a0000000-0000-4000-8000-000000000701";
/// `invented case <tag>.txt`: the same name but for case; the entry is not its own.
const LOWER: &str = "a0000000-0000-4000-8000-000000000702";
/// Two server items with the very same name `Invented same <tag>.txt`.
const SAME_A: &str = "a0000000-0000-4000-8000-000000000703";
const SAME_B: &str = "a0000000-0000-4000-8000-000000000704";
/// Twin folders: `Invented Dir <tag>` (the entry's) and `invented dir <tag>`.
const DIR_UPPER: &str = "a0000000-0000-4000-8000-000000000705";
const DIR_LOWER: &str = "a0000000-0000-4000-8000-000000000706";
/// A row alone at its place.
const ALONE: &str = "a0000000-0000-4000-8000-000000000707";

type Source = EngineFilesSource<MemoryCredentials>;

struct Set {
    engine: Arc<Engine<MemoryCredentials>>,
    source: Source,
    root: PathBuf,
    tag: String,
    _dir: tempfile::TempDir,
    _sync: tempfile::TempDir,
    _fake: beebeeb_engine::test_api::FakeApi,
    _running: tokio::runtime::Runtime,
}

impl Set {
    /// The name of a row of this set: `Invented Case` + the tag, and so on.
    fn name(&self, stem: &str, ext: &str) -> String {
        format!("{stem} {}{ext}", self.tag)
    }
}

fn put(db: &StateDb, id: &str, name: &str, kind: ItemKind) {
    let entry = FileEntry {
        file_id: id.to_string(),
        path: format!("/{name}"),
        status: FileStatus::Local,
        size_bytes: 10,
        modified_at: 1_794_000_000,
        content_hash: None,
        remote_updated_at: 1_794_000_000,
        parent_id: None,
        item_kind: kind,
    };
    db.upsert_file_with_contract(&entry, |contract| contract.item_kind = entry.item_kind.clone())
        .unwrap();
}

/// A signed-in engine with sync running whose local state holds the twins above, and the sync folder with the ONE
/// entry each pair has on a volume that compares names without case.
fn set(tag: &str) -> Set {
    let (account, phrase) = FakeAccount::register(&format!("case-twins-{tag}@example.eu"), PASSWORD, TEST_PHRASE);
    let account = Arc::new(account);
    let fake = start_fake(account.clone());
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: format!("case-twins-slot-{tag}"),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: fake.base.clone(),
    };
    let engine = Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens");
    engine.restore();
    let engine = Arc::new(engine);
    engine
        .sign_in_password(&account.email, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(Zeroizing::new(phrase))
        .expect("the phrase unlocks");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let db = StateDb::open(&path).unwrap();
    let named = |stem: &str, ext: &str| format!("{stem} {tag}{ext}");
    put(&db, UPPER, &named("Invented Case", ".txt"), ItemKind::File);
    put(&db, LOWER, &named("invented case", ".txt"), ItemKind::File);
    put(&db, SAME_A, &named("Invented same", ".txt"), ItemKind::File);
    put(&db, SAME_B, &named("Invented same", ".txt"), ItemKind::File);
    put(&db, DIR_UPPER, &named("Invented Dir", ""), ItemKind::Folder);
    put(&db, DIR_LOWER, &named("invented dir", ""), ItemKind::Folder);
    put(&db, ALONE, &named("Invented alone", ".txt"), ItemKind::File);
    db.set_owner(&Identity::new(Some(&account.user_id), Some(&account.email)))
        .unwrap();
    drop(db);
    engine.use_local_state_for_test(path);
    let sync = tempfile::tempdir().unwrap();
    engine.use_running_root_for_test(sync.path().to_path_buf());
    let running = hold_running(&engine);
    let root = engine.sync_root().expect("a sync folder");
    assert_eq!(root, sync.path(), "the test's own sync folder");
    for file in ["Invented Case", "Invented same", "Invented alone"] {
        std::fs::write(root.join(named(file, ".txt")), b"one entry").unwrap();
    }
    std::fs::create_dir_all(root.join(named("Invented Dir", ""))).unwrap();
    let mut source = EngineFilesSource::new(engine.clone());
    source.starts = Starts {
        open: record_open,
        reveal: record_reveal,
    };
    Set {
        engine,
        source,
        root,
        tag: tag.to_string(),
        _dir: dir,
        _sync: sync,
        _fake: fake,
        _running: running,
    }
}

/// The engine's slot holds a task that never runs and the view says sync runs (as the tray's tests do).
fn hold_running(engine: &Engine<MemoryCredentials>) -> tokio::runtime::Runtime {
    let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
    let task = runtime.spawn(std::future::pending::<()>());
    let account = engine.accounts().active_account().expect("the account");
    *account.engine.try_lock().expect("the engine slot is free") = Some(EngineRunner::for_test_with_task(task));
    engine.set_view_for_test(|view| view.start = Start::Running);
    runtime
}

/// What the stand-in starts were handed, every test's (each filters by its own tag).
static STARTED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

fn record_open(path: &Path) -> Result<(), ActionError> {
    STARTED.lock().unwrap().push(path.to_path_buf());
    Ok(())
}

fn record_reveal(path: &Path) -> Result<(), ActionError> {
    STARTED.lock().unwrap().push(path.to_path_buf());
    Ok(())
}

fn row(id: &str, name: &str, is_folder: bool) -> DriveItem {
    DriveItem {
        id: id.to_string(),
        parent_id: None,
        name: name.to_string(),
        is_folder,
        size: Some(10),
        child_count: None,
        modified_at: String::new(),
        mime: None,
        has_thumbnail: false,
        shared_type: None,
        shared_badges: Vec::new(),
        pc_status: None,
        is_protected: false,
    }
}

/// Asks `open_with_by` (the path every hand-over of a file takes, `path_the_system_opens`) with a recording chooser.
fn ask(set: &Set, id: &str) -> (Result<(), ActionError>, Vec<PathBuf>) {
    let mut asked = Vec::new();
    let answer = set.source.open_with_by(&row(id, "", false), |path| {
        asked.push(path.to_path_buf());
        Ok(())
    });
    (answer, asked)
}

/// The refusal a press on the case twin answers, and on either of two items with the very same name.
const CASE_TWIN: Result<(), ActionError> = Err(ActionError::NameSharedButCase);
const SAME_NAME: Result<(), ActionError> = Err(ActionError::NameShared);

/// Open and Open in another app… (`path_the_system_opens`): the twin and the two same-name items are refused and no
/// path is handed over; the row whose spelling the entry has, and a row alone, get their own entry.
#[test]
fn open_of_a_twin_never_hands_over_the_other_rows_entry() {
    let set = set("t1");
    with_rule_for_test(CaseRule::Insensitive, || {
        assert_eq!(ask(&set, LOWER), (CASE_TWIN, vec![]), "the case twin");
        assert_eq!(ask(&set, SAME_A), (SAME_NAME, vec![]), "a same-name twin");
        assert_eq!(ask(&set, SAME_B), (SAME_NAME, vec![]), "the other same-name twin");
        assert_eq!(
            ask(&set, UPPER),
            (Ok(()), vec![set.root.join(set.name("Invented Case", ".txt"))]),
            "the row whose spelling the entry has"
        );
        assert_eq!(
            ask(&set, ALONE),
            (Ok(()), vec![set.root.join(set.name("Invented alone", ".txt"))])
        );
    });
}

/// Show in File Explorer: the twin is refused before anything starts; the owner row's entry is shown.
#[test]
fn show_in_file_explorer_of_a_twin_is_refused() {
    let mut set = set("t2");
    let mut shown = |set: &mut Set, id: &str| set.source.show_in_file_explorer(&[row(id, "", false)]);
    with_rule_for_test(CaseRule::Insensitive, || {
        assert_eq!(shown(&mut set, LOWER), CASE_TWIN, "the case twin");
        assert_eq!(shown(&mut set, SAME_A), SAME_NAME, "a same-name twin");
        assert_eq!(shown(&mut set, UPPER), Ok(()));
        assert_eq!(shown(&mut set, ALONE), Ok(()));
    });
    let mine = |path: &PathBuf| path.to_string_lossy().contains(" t2");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while STARTED.lock().unwrap().iter().filter(|p| mine(p)).count() < 2 && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    std::thread::sleep(std::time::Duration::from_millis(50));
    let mut started: Vec<PathBuf> = STARTED.lock().unwrap().iter().filter(|p| mine(p)).cloned().collect();
    started.sort();
    assert_eq!(
        started,
        [
            set.root.join(set.name("Invented Case", ".txt")),
            set.root.join(set.name("Invented alone", ".txt")),
        ],
        "only the rows' own entries were shown"
    );
}

/// Open in File Explorer on a twin FOLDER (the toolbar's button while it is shown) is refused; the folder whose
/// spelling the entry has opens.
#[test]
fn open_in_file_explorer_of_a_twin_folder_is_refused() {
    let mut set = set("t3");
    with_rule_for_test(CaseRule::Insensitive, || {
        assert_eq!(set.source.open_in_file_explorer(Some(DIR_LOWER)), CASE_TWIN);
        assert_eq!(set.source.open_in_file_explorer(Some(DIR_UPPER)), Ok(()));
    });
    // A volume that tells case apart: each spelling is its own entry, nothing is refused for case.
    with_rule_for_test(CaseRule::Sensitive, || {
        assert_eq!(set.source.open_in_file_explorer(Some(DIR_LOWER)), Ok(()));
        assert_eq!(ask(&set, LOWER).0, Ok(()));
        assert_eq!(ask(&set, SAME_A).0, SAME_NAME, "one name is one entry on every volume");
    });
    let _ = &set.engine;
}

/// The Files list marks each row whose presses on disk are refused, in the PC column's words, and only those; on a
/// volume that tells case apart only the two items with the very same name.
#[test]
fn the_files_list_marks_each_twin_and_only_the_twins() {
    let set = set("t4");
    let marks = |rule| {
        with_rule_for_test(rule, || {
            let listing = crate::engine_host::EngineDriveSource::new(set.engine.clone())
                .list(None)
                .expect("the listing");
            let mut marks: Vec<(String, Option<String>)> = listing
                .items
                .into_iter()
                .map(|item| (item.id, item.pc_status))
                .collect();
            marks.sort();
            marks
        })
    };
    let all = |lower: &str| {
        let on = Some("on_this_pc".to_string());
        vec![
            (UPPER.to_string(), on.clone()),
            (LOWER.to_string(), Some(lower.to_string())),
            (SAME_A.to_string(), Some("name_shared".to_string())),
            (SAME_B.to_string(), Some("name_shared".to_string())),
            (DIR_UPPER.to_string(), None),
            (DIR_LOWER.to_string(), None),
            (ALONE.to_string(), on),
        ]
    };
    let insensitive = marks(CaseRule::Insensitive);
    let mut lower_folder_marked = all("name_shared_but_case");
    lower_folder_marked[5].1 = Some("name_shared_but_case".to_string());
    assert_eq!(
        insensitive, lower_folder_marked,
        "the case twin file and the case twin folder"
    );
    assert_eq!(marks(CaseRule::Sensitive), all("on_this_pc"));
    assert_eq!(
        crate::frames::files::list::name_shared(Some("name_shared_but_case")),
        Some("Same name as another item here, except for capital letters")
    );
    assert_eq!(crate::frames::files::list::name_shared(Some("on_this_pc")), None);
}

/// What the message says: each twin refusal says why and what fixes it, and offers no Try again (asking again changes
/// nothing). Every other Conflict (a folder moved into itself, an item that cannot go to Trash yet) keeps the plain
/// failure: the twin's words are never said of something else.
#[test]
fn a_twin_refusal_says_why_and_a_plain_conflict_does_not() {
    use crate::frames::files::Toast;
    use crate::frames::files::source::Action;
    assert_eq!(
        Toast::failure(Action::Open, ActionError::NameSharedButCase),
        "Couldn't open. Another item here has the same name except for capital letters. Rename one on the web to \
         use it on this PC."
    );
    assert_eq!(
        Toast::failure(Action::Rename, ActionError::NameShared),
        "Couldn't rename. Another item here has the same name. Rename one on the web to use it on this PC."
    );
    assert_eq!(map_drive_err(DriveError::Conflict), ActionError::Failed);
    assert_eq!(
        Toast::failure(Action::Move, map_drive_err(DriveError::Conflict)),
        "Couldn't move."
    );
    assert_eq!(
        map_drive_err(DriveError::SharedPlace(Clash::Case)),
        ActionError::NameSharedButCase
    );
    assert_eq!(
        map_drive_err(DriveError::SharedPlace(Clash::SameName)),
        ActionError::NameShared
    );
    // Keep on this PC's refusal, from the engine's fixed words.
    use beebeeb_engine::app::kept_api::{SHARED_PLACE, SHARED_PLACE_BUT_CASE};
    assert_eq!(
        crate::engine_host::files::kept_refusal(SHARED_PLACE_BUT_CASE),
        ActionError::NameSharedButCase
    );
    assert_eq!(
        crate::engine_host::files::kept_refusal(SHARED_PLACE),
        ActionError::NameShared
    );
    assert_eq!(
        crate::engine_host::files::kept_refusal("could not change what is kept on this computer"),
        ActionError::Failed
    );
}
