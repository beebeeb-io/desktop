//! LD-0113 D1 (spec §6.3), the Windows-native rungs: the pre-operation delete and rename callbacks on a
//! real Cloud Files root. Every test registers its OWN temporary sync root under the QA provider (never
//! the app's provider, never the person's Beebeeb folder) with a scratch state DB, and unregisters it
//! when it ends. Deletes and renames come from a CHILD process (`cmd.exe`), as Explorer's would: this
//! process's own are never refused. Nothing here talks to a server. `--nocapture` shows the numbers.

use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use beebeeb_core::backup::{MSG_DELETE, MSG_RENAME, own_backup_refusal};
use windows::Win32::Storage::CloudFilters::*;
use windows::core::PCWSTR;

use crate::backup_protect::{OwnBackupFolder, OwnBackups};
use crate::state_db::{FileEntry, FileStatus, ItemKind, StateDb};
use crate::windows_cf::callbacks::{ack_pre_operation, delete_refusal, rename_refusal};
use crate::windows_cf::placeholders;

const ROOT_ID: &str = "0b5ec0de-0000-4000-8000-0000000000c1";

/// What the test callbacks answer from, and what they answered: the time each answer took (callback
/// entry to the `CfExecute` ACK returned) and the refusal words, if any.
struct Fixture {
    db: StateDb,
    own: OwnBackups,
    root: PathBuf,
    answers: Mutex<Vec<(Duration, Option<String>)>>,
    /// The production gate, one per test (the process-wide one would mix tests that run at once).
    gate: Mutex<crate::backup_protect::NoticeGate>,
    /// The notices the person would have been shown.
    shown: Mutex<Vec<String>>,
}

impl Fixture {
    /// Every refused answer's words (Windows may ask several times for one delete).
    fn refusals(&self) -> Vec<String> {
        self.answers.lock().unwrap().iter().filter_map(|(_, words)| words.clone()).collect()
    }

    fn shown(&self) -> Vec<String> {
        self.shown.lock().unwrap().clone()
    }

    /// Every callback Windows made for the change was answered with the refusal `words`, and the
    /// person is told once (the a80dcd4 rung saw 4 callbacks for one `del`).
    fn assert_refused_once(&self, words: &str) {
        let answers: Vec<Option<String>> = self.answers.lock().unwrap().iter().map(|(_, w)| w.clone()).collect();
        assert!(
            !answers.is_empty() && answers.iter().all(|w| w.as_deref() == Some(words)),
            "every answer refused with {words:?}, got {answers:?}"
        );
        assert_eq!(self.shown(), vec![words.to_string()], "one notice for one change");
        println!("ld0113 refused: callbacks={} notices={}", answers.len(), self.shown().len());
    }

    /// After the answer, as `note_refusal` does after the production callbacks answer.
    fn record(&self, started: Instant, refusal: Option<String>) {
        // The answer time is measured up to the ACK, before the notice gate (Windows waits for the ACK only).
        let answered = started.elapsed();
        if let Some(words) = refusal.as_deref()
            && self.gate.lock().unwrap().due(words, Instant::now())
        {
            self.shown.lock().unwrap().push(words.to_string());
        }
        self.answers.lock().unwrap().push((answered, refusal));
    }
}

/// The production callbacks' decision and answer, reached through the connection's context.
unsafe extern "system" fn test_delete(info: *const CF_CALLBACK_INFO, params: *const CF_CALLBACK_PARAMETERS) {
    let started = Instant::now();
    // SAFETY: valid for the duration of the callback; the context is the live `Fixture` (see `connect`).
    let info = unsafe { &*info };
    let fx = unsafe { &*(info.CallbackContext as *const Fixture) };
    let params = (!params.is_null()).then(|| unsafe { &*params });
    let refusal = delete_refusal(&fx.db, &fx.own, Some(&fx.root), info, params);
    unsafe { ack_pre_operation(info, CF_OPERATION_TYPE_ACK_DELETE, refusal.is_some()) };
    fx.record(started, refusal);
}

unsafe extern "system" fn test_rename(info: *const CF_CALLBACK_INFO, params: *const CF_CALLBACK_PARAMETERS) {
    let started = Instant::now();
    // SAFETY: as `test_delete`.
    let info = unsafe { &*info };
    let fx = unsafe { &*(info.CallbackContext as *const Fixture) };
    let params = (!params.is_null()).then(|| unsafe { &*params });
    let refusal = rename_refusal(&fx.db, &fx.own, Some(&fx.root), info, params);
    unsafe { ack_pre_operation(info, CF_OPERATION_TYPE_ACK_RENAME, refusal.is_some()) };
    fx.record(started, refusal);
}

/// A registered QA root with a flagged Backups root and the fixture its callbacks answer from;
/// disconnected and unregistered on drop, whatever the test did.
struct QaRoot {
    _dir: tempfile::TempDir,
    root: PathBuf,
    key: Option<CF_CONNECTION_KEY>,
    fx: Arc<Fixture>,
}

impl QaRoot {
    fn new(own: OwnBackups) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("qa-root");
        std::fs::create_dir(&root).unwrap();
        crate::windows_cf::register_sync_root_with(&root, &crate::windows_cf::QA_PROVIDER, crate::windows_cf::HYDRATION_POLICY)
            .unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let fx = Arc::new(Fixture {
            db,
            own,
            root: root.clone(),
            answers: Mutex::new(Vec::new()),
            gate: Mutex::new(crate::backup_protect::NoticeGate::default()),
            shown: Mutex::new(Vec::new()),
        });
        let qa = QaRoot { _dir: dir, root, key: None, fx };
        qa.item(ROOT_ID, "Backups", true);
        assert!(qa.fx.db.set_system_role(ROOT_ID, Some(beebeeb_types::backup::SystemRole::BackupsRoot)).unwrap());
        qa
    }

    /// A cloud-only row at `rel` and its placeholder, as the pull and `populate_placeholders` leave them.
    fn item(&self, id: &str, rel: &str, is_dir: bool) {
        self.fx
            .db
            .upsert_file(&FileEntry {
                file_id: id.into(),
                path: rel.into(),
                status: FileStatus::CloudOnly,
                size_bytes: if is_dir { 0 } else { 5 },
                modified_at: 1_700_000_000,
                content_hash: None,
                remote_updated_at: 1_700_000_000,
                parent_id: None,
                item_kind: if is_dir { ItemKind::Folder } else { ItemKind::File },
            })
            .unwrap();
        // `upsert_file` never writes `item_kind` (the contract owns it); the pull writes it through the
        // contract, so a folder row is a folder for `populate_placeholders`. Without this the a80dcd4 rung
        // re-created `Backups` as a FILE placeholder and its children failed `ensure_directory` (1 of 3).
        let mut contract = self.fx.db.get_file_contract_state(id).unwrap().unwrap();
        contract.item_kind = if is_dir { ItemKind::Folder } else { ItemKind::File };
        self.fx.db.set_file_contract_state(&contract).unwrap();
        let path = self.path(rel);
        let (parent, name) = (path.parent().unwrap(), path.file_name().unwrap().to_string_lossy().into_owned());
        assert!(placeholders::create_placeholder(parent, id, &name, if is_dir { 0 } else { 5 }, 1_700_000_000, is_dir).unwrap());
    }

    fn connect(&mut self) {
        let table = [
            CF_CALLBACK_REGISTRATION {
                Type: CF_CALLBACK_TYPE_NOTIFY_DELETE,
                Callback: Some(test_delete),
            },
            CF_CALLBACK_REGISTRATION {
                Type: CF_CALLBACK_TYPE_NOTIFY_RENAME,
                Callback: Some(test_rename),
            },
            CF_CALLBACK_REGISTRATION {
                Type: CF_CALLBACK_TYPE_NONE,
                Callback: None,
            },
        ];
        let wide: Vec<u16> = self.root.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: the path and the table outlive the call; the context is the fixture, which this root
        // holds until after it disconnects (Drop).
        let key = unsafe {
            CfConnectSyncRoot(
                PCWSTR(wide.as_ptr()),
                table.as_ptr(),
                Some(Arc::as_ptr(&self.fx) as *const std::ffi::c_void),
                CF_CONNECT_FLAG_REQUIRE_PROCESS_INFO,
            )
        }
        .unwrap();
        self.key = Some(key);
    }

    fn path(&self, rel: &str) -> PathBuf {
        rel.split('/').fold(self.root.clone(), |path, segment| path.join(segment))
    }
}

impl Drop for QaRoot {
    fn drop(&mut self) {
        if let Some(key) = self.key.take() {
            // SAFETY: the key of the connection made in `connect`, disconnected once.
            let _ = unsafe { CfDisconnectSyncRoot(key) };
        }
        let _ = crate::windows_cf::unregister_sync_root(&self.root);
    }
}

/// Run `line` through `cmd.exe`, a process other than this one (as Explorer is).
fn cmd(line: &str) -> std::process::Output {
    std::process::Command::new("cmd").args(["/D", "/C"]).raw_arg(line).output().unwrap()
}

fn quoted(path: &Path) -> String {
    format!("\"{}\"", path.display())
}

fn pictures_on() -> OwnBackups {
    OwnBackups {
        device_folder: Some("GLegion".into()),
        folders: vec![OwnBackupFolder {
            display_name: "Pictures".into(),
            source_path: PathBuf::from(r"C:\Users\qa\Pictures"),
        }],
    }
}

/// This user's Recycle Bin folder on the volume of `path` (`X:\$Recycle.Bin\<SID>`), made if missing.
fn recycle_bin_for(path: &Path) -> PathBuf {
    let out = std::process::Command::new("whoami").args(["/user", "/fo", "csv", "/nh"]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let sid = text.trim().rsplit(',').next().unwrap().trim_matches('"').to_string();
    assert!(sid.starts_with("S-1-"), "whoami gave a SID: {text}");
    let volume = path.components().next().unwrap().as_os_str().to_string_lossy().into_owned();
    let bin = PathBuf::from(format!(r"{volume}\$Recycle.Bin")).join(sid);
    std::fs::create_dir_all(&bin).unwrap();
    bin
}

#[test]
fn explorer_delete_of_backups_refused() {
    let mut qa = QaRoot::new(OwnBackups::default());
    qa.connect();
    // A permanent delete of the whole folder (Shift+Delete, `rmdir /S`): its children are ordinary
    // (spec §4.2 rule 7), the root itself is refused.
    qa.item("0b5ec0de-0000-4000-8000-0000000000c2", "Backups/OtherPC", true);
    let out = cmd(&format!("rmdir /S /Q {}", quoted(&qa.path("Backups"))));
    assert!(qa.path("Backups").is_dir(), "the Backups root is still there");
    assert!(qa.fx.refusals().contains(&MSG_DELETE.to_string()), "refused with core's words, got {:?}", qa.fx.refusals());
    println!(
        "ld0113 explorer_delete: refusals={} children_left={} cmd_stderr={:?}",
        qa.fx.refusals().len(),
        std::fs::read_dir(qa.path("Backups")).unwrap().count(),
        String::from_utf8_lossy(&out.stderr).trim()
    );
}

#[test]
fn recycle_bin_move_of_backups_refused() {
    let mut qa = QaRoot::new(OwnBackups::default());
    qa.item("0b5ec0de-0000-4000-8000-0000000000c3", "Backups/OtherPC", true);
    qa.connect();
    // Explorer's Delete: a rename of the folder into this user's Recycle Bin on the same volume.
    let target = recycle_bin_for(&qa.root).join(format!("$RLD0113{}", std::process::id()));
    cmd(&format!("move {} {}", quoted(&qa.path("Backups")), quoted(&target)));
    let landed = target.exists();
    if landed {
        // A red run must not leave the folder in the person's Recycle Bin.
        let _ = std::fs::rename(&target, qa.path("Backups"));
    }
    assert!(!landed, "nothing reached the Recycle Bin");
    assert!(qa.path("Backups/OtherPC").is_dir(), "the root and what is in it stay");
    qa.fx.assert_refused_once(MSG_DELETE); // a move into the Recycle Bin is a delete
}

#[test]
fn rename_of_backups_refused() {
    let mut qa = QaRoot::new(OwnBackups::default());
    qa.connect();
    cmd(&format!("ren {} \"Old backups\"", quoted(&qa.path("Backups"))));
    assert!(qa.path("Backups").is_dir() && !qa.path("Old backups").exists(), "the rename did not happen");
    qa.fx.assert_refused_once(MSG_RENAME);
}

#[test]
fn own_enabled_subtree_delete_refused() {
    let mut qa = QaRoot::new(pictures_on());
    qa.item("0b5ec0de-0000-4000-8000-0000000000c4", "Backups/GLegion", true);
    qa.item("0b5ec0de-0000-4000-8000-0000000000c5", "Backups/GLegion/Pictures", true);
    qa.item("0b5ec0de-0000-4000-8000-0000000000c6", "Backups/GLegion/Pictures/a.jpg", false);
    qa.connect();
    cmd(&format!("del /F /Q {}", quoted(&qa.path("Backups/GLegion/Pictures/a.jpg"))));
    assert!(qa.path("Backups/GLegion/Pictures/a.jpg").exists(), "the backed-up file stays");
    qa.fx.assert_refused_once(&own_backup_refusal(r"C:\Users\qa\Pictures\a.jpg"));
}

#[test]
fn own_disabled_subtree_delete_allowed() {
    let mut qa = QaRoot::new(OwnBackups {
        device_folder: Some("GLegion".into()),
        folders: Vec::new(),
    });
    qa.item("0b5ec0de-0000-4000-8000-0000000000c7", "Backups/GLegion", true);
    qa.item("0b5ec0de-0000-4000-8000-0000000000c8", "Backups/GLegion/Pictures", true);
    qa.item("0b5ec0de-0000-4000-8000-0000000000c9", "Backups/GLegion/Pictures/a.jpg", false);
    qa.connect();
    cmd(&format!("del /F /Q {}", quoted(&qa.path("Backups/GLegion/Pictures/a.jpg"))));
    assert!(!qa.path("Backups/GLegion/Pictures/a.jpg").exists(), "a turned-off backup is an ordinary archive");
    assert!(qa.fx.refusals().is_empty());
    assert_eq!(qa.fx.answers.lock().unwrap().len(), 1, "the callback answered the delete");
}

#[test]
fn ordinary_folder_delete_unchanged() {
    let mut qa = QaRoot::new(pictures_on());
    qa.item("0b5ec0de-0000-4000-8000-0000000000ca", "Documents", true);
    qa.item("0b5ec0de-0000-4000-8000-0000000000cb", "Documents/notes.txt", false);
    qa.connect();
    cmd(&format!("rmdir /S /Q {}", quoted(&qa.path("Documents"))));
    assert!(!qa.path("Documents").exists(), "an ordinary folder deletes as before");
    assert!(qa.fx.refusals().is_empty());
}

#[tokio::test]
async fn delete_while_app_closed_restored_on_start_zero_server_calls() {
    let qa = QaRoot::new(pictures_on());
    qa.item("0b5ec0de-0000-4000-8000-0000000000cc", "Backups/OtherPC", true);
    qa.item("0b5ec0de-0000-4000-8000-0000000000cd", "Backups/OtherPC/a.txt", false);
    // The app is not running: no connection, so Windows asks nobody and the delete goes through.
    std::fs::remove_dir_all(qa.path("Backups")).unwrap();
    assert!(!qa.path("Backups").exists());

    // The next start: the placeholder pass mints what the DB says is there.
    let restored = crate::windows_cf::restore_placeholders(&qa.fx.db, &qa.root);
    assert_eq!(restored, 3, "the root, its folder and its file are back");
    assert!(qa.path("Backups/OtherPC/a.txt").exists());

    // A late delete completion for the root, then a transfer pass, against an API that counts
    // connections: nothing is queued and nothing is sent.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let api = crate::api_client::ApiClient::new(format!("http://{}", listener.local_addr().unwrap()), "token".into(), [7u8; 32]);
    let db = Arc::new(StateDb::open(qa._dir.path().join("state.db")).unwrap());
    let bridge = crate::engine_bridge::EngineBridge::new(db.clone(), Arc::new(api));
    crate::watcher::handle_delete(&bridge, &qa.root, &qa.path("Backups")).await;
    assert!(db.list_operations_for_file(ROOT_ID).unwrap().is_empty(), "no trash is queued");
    let outcome = bridge.process_due_operations(&qa.root, i64::MAX / 2).await.unwrap();
    assert!(outcome.completed_op_ids.is_empty() && outcome.retried_op_ids.is_empty());
    let calls = match listener.accept() {
        Ok(_) => 1,
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => 0,
        Err(error) => panic!("{error}"),
    };
    assert_eq!(calls, 0, "zero server calls");
    println!("ld0113 delete_while_closed: restored={restored} server_calls={calls}");
}

/// Risk §15: every delete in the sync root now waits for this answer. 1,000 deletes from another
/// process, each answered from the DB; the p99 is recorded (and bounded far below Explorer's patience).
#[test]
fn pre_operation_answer_p99_over_1000_deletes() {
    let mut qa = QaRoot::new(pictures_on());
    qa.item("0b5ec0de-0000-4000-8000-0000000000ce", "Documents", true);
    for i in 0..1_000 {
        qa.item(&format!("0b5ec0de-0000-4000-8000-{:012x}", 0x1000 + i), &format!("Documents/f{i:04}.txt"), false);
    }
    qa.connect();
    let started = Instant::now();
    cmd(&format!("rmdir /S /Q {}", quoted(&qa.path("Documents"))));
    let wall = started.elapsed();
    assert!(!qa.path("Documents").exists(), "every ordinary delete went through");
    let mut times: Vec<Duration> = qa.fx.answers.lock().unwrap().iter().map(|(time, _)| *time).collect();
    assert!(times.len() >= 1_000, "1,000 file deletes reached the callback, got {}", times.len());
    times.sort();
    let at = |q: f64| times[((times.len() as f64 * q).ceil() as usize).saturating_sub(1)];
    let (p50, p99, max) = (at(0.50), at(0.99), *times.last().unwrap());
    println!(
        "ld0113 p99: deletes={} p50_us={} p99_us={} max_us={} wall_ms={}",
        times.len(),
        p50.as_micros(),
        p99.as_micros(),
        max.as_micros(),
        wall.as_millis()
    );
    assert!(p99 < Duration::from_millis(50), "p99 {p99:?}");
}
