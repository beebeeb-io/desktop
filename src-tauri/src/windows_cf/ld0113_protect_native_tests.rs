//! LD-0113 D1 (spec §6.3, Amendment 6), the Windows-native rungs: the pre-operation delete and rename
//! callbacks on a real Cloud Files root. Every test registers its OWN temporary sync root under the QA
//! provider (never the app's provider, never the person's Beebeeb folder) with a scratch state DB, and
//! unregisters it when it ends. Deletes and renames come from a CHILD process (`cmd.exe`), as Explorer's
//! would: this process's own are never refused. Nothing here talks to a server: where an API is
//! needed it is a localhost listener that counts connections. `--nocapture` shows the numbers.

use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use beebeeb_core::backup::{MANIFEST_NAME, MSG_DELETE, MSG_MANAGED_IN_APP, MSG_READ_ONLY, MSG_RENAME, own_backup_refusal};
use windows::Win32::Foundation::{HANDLE, STATUS_SUCCESS};
use windows::Win32::Storage::CloudFilters::*;
use windows::Win32::Storage::FileSystem::{DELETE, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES};
use windows::core::PCWSTR;

use crate::backup_protect::{OwnBackupFolder, OwnBackups};
use crate::state_db::{FileEntry, FileStatus, ItemKind, StateDb};
use crate::windows_cf::callbacks::{ack_pre_operation, delete_refusal, rename_refusal, volume_relative_to_full};
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
    /// Each rename callback's target as Windows gave it (`TargetPath` joined with the volume).
    targets: Mutex<Vec<String>>,
    /// Callbacks that started (`answers` counts those that answered).
    entered: AtomicUsize,
    /// FETCH_DATA callbacks (LD-0113 Amendment 7's rungs).
    fetches: AtomicUsize,
}

impl Fixture {
    /// Every refused answer's words (Windows may ask several times for one delete).
    fn refusals(&self) -> Vec<String> {
        self.answers.lock().unwrap().iter().filter_map(|(_, words)| words.clone()).collect()
    }

    fn shown(&self) -> Vec<String> {
        self.shown.lock().unwrap().clone()
    }

    /// What the callbacks saw, for an `ld0113` line. Each lock is taken and dropped on its own: the e100f78
    /// rung hung on a print that held `answers` while `refusals()` locked it again.
    fn diagnostics(&self) -> String {
        let entered = self.entered.load(Ordering::SeqCst);
        let answered = self.answers.lock().unwrap().len();
        let refusals = self.refusals().len();
        let notices = self.shown().len();
        let targets = self.targets.lock().unwrap().clone();
        format!(
            "callbacks_entered={entered} answered={answered} refusals={refusals} notices={notices} targets={targets:?} root={:?}",
            self.root.display().to_string()
        )
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
    fx.entered.fetch_add(1, Ordering::SeqCst);
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
    fx.entered.fetch_add(1, Ordering::SeqCst);
    let params = (!params.is_null()).then(|| unsafe { &*params });
    // SAFETY: the rename arm, read while the callback is live (before the answer).
    let target = params.and_then(|p| volume_relative_to_full(info, unsafe { p.Anonymous.Rename }.TargetPath.0));
    fx.targets.lock().unwrap().push(target.map_or_else(|| "<none>".into(), |t| t.display().to_string()));
    let refusal = rename_refusal(&fx.db, &fx.own, Some(&fx.root), info, params);
    unsafe { ack_pre_operation(info, CF_OPERATION_TYPE_ACK_RENAME, refusal.is_some()) };
    fx.record(started, refusal);
}

/// The bytes the QA fetch callback serves (every fixture file is 5 bytes); past them, `x`.
const CONTENT: &[u8] = b"hello";

/// FETCH_DATA for the read-only rungs (Amendment 7): the required range, up to the placeholder's
/// size, in one transfer.
unsafe extern "system" fn test_fetch(info: *const CF_CALLBACK_INFO, params: *const CF_CALLBACK_PARAMETERS) {
    // SAFETY: valid for the duration of the callback; the context is the live `Fixture` (see `connect`).
    let info = unsafe { &*info };
    let fx = unsafe { &*(info.CallbackContext as *const Fixture) };
    fx.fetches.fetch_add(1, Ordering::SeqCst);
    // SAFETY: the FETCH_DATA arm, copied out while the callback is live.
    let fetch = unsafe { (*params).Anonymous.FetchData };
    let start = fetch.RequiredFileOffset.clamp(0, info.FileSize);
    let end = fetch.RequiredFileOffset.saturating_add(fetch.RequiredLength).clamp(start, info.FileSize);
    let bytes: Vec<u8> = (start..end).map(|at| CONTENT.get(at as usize).copied().unwrap_or(b'x')).collect();
    // SAFETY: the keys of this live fetch; `bytes` outlives the call.
    let _ = unsafe {
        super::callbacks::transfer_one(
            info.ConnectionKey,
            info.TransferKey,
            info.RequestKey,
            bytes.as_ptr().cast(),
            start,
            end - start,
            STATUS_SUCCESS,
        )
    };
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
        // The root as Windows names it back in every callback: the final, long path (8.3 short names,
        // junctions and the drive letter's case resolved), as the app's own root is. `TEMP` need not
        // be. On the 906ac78 rung a move into Backups went through, and that is the first test that
        // matches a callback's path against this root; `move_into_backups_refused` prints both.
        let root = long_path(&root);
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
            targets: Mutex::new(Vec::new()),
            entered: AtomicUsize::new(0),
            fetches: AtomicUsize::new(0),
        });
        let qa = QaRoot { _dir: dir, root, key: None, fx };
        qa.item(ROOT_ID, "Backups", true);
        assert!(qa.fx.db.set_system_role(ROOT_ID, Some(beebeeb_types::backup::SystemRole::BackupsRoot)).unwrap());
        qa
    }

    /// A cloud-only row at `rel` and its placeholder, as the pull and `populate_placeholders` leave them.
    fn item(&self, id: &str, rel: &str, is_dir: bool) {
        self.row(id, rel, is_dir);
        let path = self.path(rel);
        let (parent, name) = (path.parent().unwrap(), path.file_name().unwrap().to_string_lossy().into_owned());
        assert!(placeholders::create_placeholder(parent, id, &name, if is_dir { 0 } else { 5 }, 1_700_000_000, is_dir).unwrap());
    }

    /// The cloud-only row alone, as the pull leaves it for the placeholder pass to mint.
    fn row(&self, id: &str, rel: &str, is_dir: bool) {
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
            // LD-0113 Amendment 7: the read-only rungs read and hydrate files.
            CF_CALLBACK_REGISTRATION {
                Type: CF_CALLBACK_TYPE_FETCH_DATA,
                Callback: Some(test_fetch),
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

/// `path` as `GetFinalPathNameByHandle` gives it, without the `\\?\` verbatim prefix.
fn long_path(path: &Path) -> PathBuf {
    let full = std::fs::canonicalize(path).unwrap().display().to_string();
    PathBuf::from(full.strip_prefix(r"\\?\").unwrap_or(&full))
}

/// How long one delete, rename or move may take before the test stops it and fails.
const OS_OP_LIMIT: Duration = Duration::from_secs(30);

/// Run `line` through `cmd.exe`, a process other than this one (as Explorer is), bounded: past
/// [`OS_OP_LIMIT`] the child is stopped through its own handle and the test FAILS with an
/// `ld0113 <label> timeout` line saying what the callbacks saw. Never a hang (the e100f78 rung hung 33 min).
fn cmd(qa: &QaRoot, label: &str, line: &str) -> std::process::Output {
    use std::io::Read;
    use std::process::Stdio;
    let mut child = std::process::Command::new("cmd")
        .args(["/D", "/C"])
        .raw_arg(line)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let drain = |mut pipe: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = pipe.read_to_end(&mut bytes);
            bytes
        })
    };
    let (stdout, stderr) = (drain(Box::new(child.stdout.take().unwrap())), drain(Box::new(child.stderr.take().unwrap())));
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if started.elapsed() >= OS_OP_LIMIT {
            let _ = child.kill();
            let _ = child.wait();
            println!("ld0113 {label} timeout: after_s={} {}", OS_OP_LIMIT.as_secs(), qa.fx.diagnostics());
            panic!("{label}: `{line}` did not finish within {OS_OP_LIMIT:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    std::process::Output { status, stdout: stdout.join().unwrap(), stderr: stderr.join().unwrap() }
}

/// Run `op`, an OS operation of this process, under a watchdog: past [`OS_OP_LIMIT`] the watchdog prints
/// an `ld0113 <label> timeout` line saying what the callbacks saw and ends the test process (exit 101),
/// so the rung FAILS instead of hanging (an operation in this process cannot be stopped from outside).
fn bounded<T>(qa: &QaRoot, label: &'static str, op: impl FnOnce() -> T) -> T {
    let done = Arc::new(AtomicBool::new(false));
    let watchdog = {
        let (done, fx) = (done.clone(), qa.fx.clone());
        std::thread::spawn(move || {
            let started = Instant::now();
            while !done.load(Ordering::SeqCst) {
                if started.elapsed() >= OS_OP_LIMIT {
                    println!("ld0113 {label} timeout: after_s={} {}", OS_OP_LIMIT.as_secs(), fx.diagnostics());
                    std::process::exit(101);
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        })
    };
    let out = op();
    done.store(true, Ordering::SeqCst);
    let _ = watchdog.join();
    out
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

/// The fixture's state DB behind a bridge whose API is a localhost listener that counts connections.
fn counting_bridge(qa: &QaRoot) -> (crate::engine_bridge::EngineBridge, std::net::TcpListener) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let api = crate::api_client::ApiClient::new(format!("http://{}", listener.local_addr().unwrap()), "token".into(), [7u8; 32]);
    let db = Arc::new(StateDb::open(qa._dir.path().join("state.db")).unwrap());
    (crate::engine_bridge::EngineBridge::new(db, Arc::new(api)), listener)
}

/// What reaches the server after a change: each of `rels` gone from disk is handed to the watcher, as
/// its delete completion would be, then a transfer pass runs. The connections the API saw.
async fn server_calls_after_deletes(qa: &QaRoot, rels: &[&str]) -> usize {
    let (bridge, listener) = counting_bridge(qa);
    for rel in rels {
        if !qa.path(rel).exists() {
            crate::watcher::handle_delete(&bridge, &qa.root, &qa.path(rel)).await;
        }
    }
    bridge.process_due_operations(&qa.root, i64::MAX / 2).await.unwrap();
    std::iter::from_fn(|| match listener.accept() {
        Ok(_) => Some(()),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => None,
        Err(error) => panic!("{error}"),
    })
    .count()
}

/// A permanent delete of the whole folder (Shift+Delete, `rmdir /S`) goes bottom-up: Guus's ruling
/// (2026-11-06 17:49, "Protect the whole tree") refuses each item under the root, so nothing goes, and
/// nothing reaches the server. The 7a0def6 rung, which protected the root only, left 0 children.
#[tokio::test]
async fn explorer_delete_of_backups_refused() {
    let mut qa = QaRoot::new(OwnBackups::default());
    let fixture = ["Backups/OtherPC", "Backups/OtherPC/a.txt", "Backups/GLegion"];
    qa.item("0b5ec0de-0000-4000-8000-0000000000c2", fixture[0], true);
    qa.item("0b5ec0de-0000-4000-8000-0000000000e4", fixture[1], false);
    qa.item("0b5ec0de-0000-4000-8000-0000000000e5", fixture[2], true);
    qa.connect();
    let out = cmd(&qa, "explorer_delete_of_backups_refused", &format!("rmdir /S /Q {}", quoted(&qa.path("Backups"))));
    let children_left = std::fs::read_dir(qa.path("Backups")).unwrap().count();
    let refusals = qa.fx.refusals();
    let calls = server_calls_after_deletes(&qa, &fixture).await;
    println!(
        "ld0113 explorer_delete: refusals={} managed={} root={} notices={} children_left={children_left} server_calls={calls} cmd_stderr={:?}",
        refusals.len(),
        refusals.iter().filter(|w| *w == MSG_MANAGED_IN_APP).count(),
        refusals.iter().filter(|w| *w == MSG_DELETE).count(),
        qa.fx.shown().len(),
        String::from_utf8_lossy(&out.stderr).trim()
    );
    assert_eq!(children_left, 2, "both device folders are still in Backups");
    assert!(fixture.iter().all(|rel| qa.path(rel).exists()), "every item is still there");
    assert!(
        !refusals.is_empty() && refusals.iter().all(|w| w == MSG_MANAGED_IN_APP || w == MSG_DELETE),
        "refused with core's words, got {refusals:?}"
    );
    assert_eq!(calls, 0, "zero server calls");
}

/// Shift+Delete of a device folder: everything in it is refused with the managed-in-the-app words, the
/// person is told once, and nothing reaches the server.
#[tokio::test]
async fn shift_delete_of_a_device_folder_refused() {
    let mut qa = QaRoot::new(pictures_on());
    let fixture = ["Backups/OtherPC", "Backups/OtherPC/Pictures", "Backups/OtherPC/Pictures/a.jpg"];
    qa.item("0b5ec0de-0000-4000-8000-0000000000e1", fixture[0], true);
    qa.item("0b5ec0de-0000-4000-8000-0000000000e2", fixture[1], true);
    qa.item("0b5ec0de-0000-4000-8000-0000000000e3", fixture[2], false);
    qa.connect();
    cmd(&qa, "shift_delete_of_a_device_folder_refused", &format!("rmdir /S /Q {}", quoted(&qa.path("Backups/OtherPC"))));
    let left = fixture.iter().filter(|rel| qa.path(rel).exists()).count();
    let calls = server_calls_after_deletes(&qa, &fixture).await;
    println!("ld0113 shift_delete: items_left={left} server_calls={calls}");
    assert_eq!(left, fixture.len(), "the device folder and everything in it stay");
    qa.fx.assert_refused_once(MSG_MANAGED_IN_APP);
    assert_eq!(calls, 0, "zero server calls");
}

#[test]
fn recycle_bin_move_of_backups_refused() {
    let mut qa = QaRoot::new(OwnBackups::default());
    qa.item("0b5ec0de-0000-4000-8000-0000000000c3", "Backups/OtherPC", true);
    qa.connect();
    // Explorer's Delete: a rename of the folder into this user's Recycle Bin on the same volume.
    let target = recycle_bin_for(&qa.root).join(format!("$RLD0113{}", std::process::id()));
    cmd(&qa, "recycle_bin_move_of_backups_refused", &format!("move {} {}", quoted(&qa.path("Backups")), quoted(&target)));
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
    cmd(&qa, "rename_of_backups_refused", &format!("ren {} \"Old backups\"", quoted(&qa.path("Backups"))));
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
    cmd(&qa, "own_enabled_subtree_delete_refused", &format!("del /F /Q {}", quoted(&qa.path("Backups/GLegion/Pictures/a.jpg"))));
    assert!(qa.path("Backups/GLegion/Pictures/a.jpg").exists(), "the backed-up file stays");
    qa.fx.assert_refused_once(&own_backup_refusal(r"C:\Users\qa\Pictures\a.jpg"));
}

/// A turned-off folder's backup is managed in the app too (Amendment 6; it was deletable before).
#[test]
fn own_disabled_subtree_delete_refused() {
    let mut qa = QaRoot::new(OwnBackups {
        device_folder: Some("GLegion".into()),
        folders: Vec::new(),
    });
    qa.item("0b5ec0de-0000-4000-8000-0000000000c7", "Backups/GLegion", true);
    qa.item("0b5ec0de-0000-4000-8000-0000000000c8", "Backups/GLegion/Pictures", true);
    qa.item("0b5ec0de-0000-4000-8000-0000000000c9", "Backups/GLegion/Pictures/a.jpg", false);
    qa.connect();
    cmd(&qa, "own_disabled_subtree_delete_refused", &format!("del /F /Q {}", quoted(&qa.path("Backups/GLegion/Pictures/a.jpg"))));
    assert!(qa.path("Backups/GLegion/Pictures/a.jpg").exists(), "a turned-off backup stays in the drive");
    qa.fx.assert_refused_once(MSG_MANAGED_IN_APP);
}

/// A move from elsewhere in the drive into Backups is refused (Amendment 6); the item stays put.
#[test]
fn move_into_backups_refused() {
    let mut qa = QaRoot::new(pictures_on());
    qa.item("0b5ec0de-0000-4000-8000-0000000000e6", "Backups/OtherPC", true);
    qa.item("0b5ec0de-0000-4000-8000-0000000000e7", "Documents", true);
    qa.item("0b5ec0de-0000-4000-8000-0000000000e8", "Documents/report.txt", false);
    qa.connect();
    let out = cmd(&qa, "move_into_backups_refused", &format!("move {} {}", quoted(&qa.path("Documents/report.txt")), quoted(&qa.path("Backups/OtherPC/report.txt"))));
    println!("ld0113 move_into: {} cmd_stderr={:?}", qa.fx.diagnostics(), String::from_utf8_lossy(&out.stderr).trim());
    assert!(qa.path("Documents/report.txt").exists(), "the item stays where it was");
    assert!(!qa.path("Backups/OtherPC/report.txt").exists(), "nothing is moved into Backups");
    qa.fx.assert_refused_once(MSG_READ_ONLY); // core v0.2.1: Create on the folder it goes into
}

/// A move out of Backups is refused; the backup stays put.
#[test]
fn move_out_of_backups_refused() {
    let mut qa = QaRoot::new(pictures_on());
    qa.item("0b5ec0de-0000-4000-8000-0000000000e9", "Backups/OtherPC", true);
    qa.item("0b5ec0de-0000-4000-8000-0000000000ea", "Backups/OtherPC/a.txt", false);
    qa.item("0b5ec0de-0000-4000-8000-0000000000eb", "Documents", true);
    qa.connect();
    cmd(&qa, "move_out_of_backups_refused", &format!("move {} {}", quoted(&qa.path("Backups/OtherPC/a.txt")), quoted(&qa.path("Documents/a.txt"))));
    assert!(qa.path("Backups/OtherPC/a.txt").exists(), "the backup stays in Backups");
    assert!(!qa.path("Documents/a.txt").exists(), "nothing is moved out");
    qa.fx.assert_refused_once(MSG_MANAGED_IN_APP);
}

#[test]
fn ordinary_folder_delete_unchanged() {
    let mut qa = QaRoot::new(pictures_on());
    qa.item("0b5ec0de-0000-4000-8000-0000000000ca", "Documents", true);
    qa.item("0b5ec0de-0000-4000-8000-0000000000cb", "Documents/notes.txt", false);
    qa.connect();
    cmd(&qa, "ordinary_folder_delete_unchanged", &format!("rmdir /S /Q {}", quoted(&qa.path("Documents"))));
    assert!(!qa.path("Documents").exists(), "an ordinary folder deletes as before");
    assert!(qa.fx.refusals().is_empty());
}

#[tokio::test]
async fn delete_while_app_closed_restored_on_start_zero_server_calls() {
    let qa = QaRoot::new(pictures_on());
    qa.item("0b5ec0de-0000-4000-8000-0000000000cc", "Backups/OtherPC", true);
    qa.item("0b5ec0de-0000-4000-8000-0000000000cd", "Backups/OtherPC/a.txt", false);
    // a.txt was downloaded: its row is `local`, which the placeholder pass alone never mints again.
    qa.fx.db.set_status("0b5ec0de-0000-4000-8000-0000000000cd", FileStatus::Local).unwrap();
    // The app is not running: no connection, so Windows asks nobody and the delete goes through.
    std::fs::remove_dir_all(qa.path("Backups")).unwrap();
    assert!(!qa.path("Backups").exists());

    // The next start: the delete reaches the watcher, which queues nothing and makes every row under the
    // root whose bytes went cloud-only again; the placeholder pass then mints what the DB says is there.
    // A transfer pass runs against an API that counts connections.
    let calls = server_calls_after_deletes(&qa, &["Backups"]).await;
    crate::windows_cf::restore_placeholders(&qa.fx.db, &qa.root);
    // The watcher's own pass runs off its task (`spawn_blocking`, review #9): wait for whichever pass
    // mints last, bounded.
    let fixture = ["Backups", "Backups/OtherPC", "Backups/OtherPC/a.txt"];
    let deadline = Instant::now() + Duration::from_secs(5);
    let restored = loop {
        let restored = fixture.iter().filter(|rel| qa.path(rel).exists()).count();
        if restored == fixture.len() || Instant::now() >= deadline {
            break restored;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    println!("ld0113 delete_while_closed: restored={restored} server_calls={calls}");
    assert_eq!(restored, 3, "the root, its folder and its downloaded file are back");
    assert_eq!(qa.fx.db.get_file("0b5ec0de-0000-4000-8000-0000000000cd").unwrap().unwrap().status, FileStatus::CloudOnly);
    for id in [ROOT_ID, "0b5ec0de-0000-4000-8000-0000000000cc", "0b5ec0de-0000-4000-8000-0000000000cd"] {
        assert!(qa.fx.db.list_operations_for_file(id).unwrap().is_empty(), "no trash is queued for {id}");
    }
    assert_eq!(calls, 0, "zero server calls");
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
    cmd(&qa, "pre_operation_answer_p99_over_1000_deletes", &format!("rmdir /S /Q {}", quoted(&qa.path("Documents"))));
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

fn read_only(path: &Path) -> Option<bool> {
    crate::windows_cf::read_only::is_read_only(path)
}

/// Amendment 7 (Guus, 2026-11-06 19:30): the placeholder pass (the pull's path) mints a file under
/// Backups read-only and in sync; a file outside and a folder are not read-only. A placeholder an older
/// build minted takes the attribute when the tree's state is applied (at start, or when the role is
/// flagged later) and stays in sync; a cleared role takes the attribute away again.
#[test]
fn backups_file_is_read_only() {
    let mut qa = QaRoot::new(OwnBackups::default());
    qa.connect();
    // An older build's placeholder, minted before files under Backups were read-only.
    qa.item("0b5ec0de-0000-4000-8000-0000000000f1", "Backups/Old", true);
    qa.item("0b5ec0de-0000-4000-8000-0000000000f2", "Backups/Old/c.txt", false);
    // What the pull leaves for the placeholder pass.
    qa.row("0b5ec0de-0000-4000-8000-0000000000f3", "Backups/OtherPC", true);
    qa.row("0b5ec0de-0000-4000-8000-0000000000f4", "Backups/OtherPC/a.txt", false);
    qa.row("0b5ec0de-0000-4000-8000-0000000000f5", "Documents", true);
    qa.row("0b5ec0de-0000-4000-8000-0000000000f6", "Documents/b.txt", false);
    let created = bounded(&qa, "backups_file_is_read_only pass", || crate::windows_cf::restore_placeholders(&qa.fx.db, &qa.root));
    let (a, b, c) = (qa.path("Backups/OtherPC/a.txt"), qa.path("Documents/b.txt"), qa.path("Backups/Old/c.txt"));
    println!(
        "ld0113 read_only minted: created={created} a={:?} a_in_sync={:?} b={:?} folder={:?} old_c={:?}",
        read_only(&a),
        placeholders::placeholder_in_sync(&a),
        read_only(&b),
        read_only(&qa.path("Backups/OtherPC")),
        read_only(&c)
    );
    assert_eq!(created, 4, "the folder and file under Backups, and the two outside");
    assert_eq!(read_only(&a), Some(true), "a file minted under Backups is read-only");
    assert_eq!(placeholders::placeholder_in_sync(&a), Some(true), "and in sync");
    assert_eq!(read_only(&b), Some(false), "a file outside Backups is not");
    assert_eq!(read_only(&qa.path("Backups/OtherPC")), Some(false), "a folder is left alone");
    assert_eq!(read_only(&c), Some(false), "the older placeholder, until the tree's state is applied");

    let changed = bounded(&qa, "backups_file_is_read_only apply", || {
        crate::backup_protect::apply_tree_read_only(&qa.fx.db, &qa.root, "Backups", true)
    });
    println!("ld0113 read_only applied: changed={changed} old_c={:?} old_c_in_sync={:?}", read_only(&c), placeholders::placeholder_in_sync(&c));
    assert_eq!(changed, 1, "only the older placeholder changed");
    assert_eq!(read_only(&c), Some(true));
    assert_eq!(placeholders::placeholder_in_sync(&c), Some(true), "still in sync after the attribute changed");

    // An admin cleared a wrong flag (spec §5.5): the files are writable again.
    assert!(qa.fx.db.set_system_role(ROOT_ID, None).unwrap());
    let cleared = bounded(&qa, "backups_file_is_read_only clear", || {
        crate::backup_protect::apply_tree_read_only(&qa.fx.db, &qa.root, "Backups", false)
    });
    println!("ld0113 read_only cleared: cleared={cleared} a={:?} old_c={:?}", read_only(&a), read_only(&c));
    assert_eq!(cleared, 2);
    assert_eq!((read_only(&a), read_only(&c)), (Some(false), Some(false)));
}

/// A read-only placeholder hydrates when another process reads it, and the engine's own hydrate
/// ("Always keep on this device"), dehydrate ("Free up space") and hydrate again go through; the file is
/// read-only after each.
#[test]
fn read_only_placeholder_hydrates() {
    let mut qa = QaRoot::new(OwnBackups::default());
    qa.connect();
    qa.row("0b5ec0de-0000-4000-8000-0000000000f7", "Backups/OtherPC", true);
    qa.row("0b5ec0de-0000-4000-8000-0000000000f8", "Backups/OtherPC/a.txt", false);
    qa.row("0b5ec0de-0000-4000-8000-0000000000f9", "Backups/OtherPC/b.txt", false);
    bounded(&qa, "read_only_placeholder_hydrates pass", || crate::windows_cf::restore_placeholders(&qa.fx.db, &qa.root));
    let (a, b) = (qa.path("Backups/OtherPC/a.txt"), qa.path("Backups/OtherPC/b.txt"));
    assert_eq!((read_only(&a), read_only(&b)), (Some(true), Some(true)));

    // Another process opens it to read, as an editor does.
    let out = cmd(&qa, "read_only_placeholder_hydrates", &format!("type {}", quoted(&a)));
    let read = String::from_utf8_lossy(&out.stdout).into_owned();
    let a_on_disk = placeholders::fully_on_disk(&a);
    // The engine's own changes.
    let hydrated = bounded(&qa, "read_only_placeholder_hydrates hydrate", || placeholders::hydrate_placeholder(&b).map_err(|e| e.to_string()));
    let (b_on_disk, b_read_only_hydrated) = (placeholders::fully_on_disk(&b), read_only(&b));
    let freed = bounded(&qa, "read_only_placeholder_hydrates dehydrate", || placeholders::dehydrate_placeholder(&b).map_err(|e| e.to_string()));
    let (b_freed_on_disk, b_read_only_freed) = (placeholders::fully_on_disk(&b), read_only(&b));
    let again = bounded(&qa, "read_only_placeholder_hydrates again", || placeholders::hydrate_placeholder(&b).map_err(|e| e.to_string()));
    println!(
        "ld0113 read_only_hydrates: type_ok={} type_stdout={read:?} a_on_disk={a_on_disk} a_ro={:?} hydrated={hydrated:?} b_on_disk={b_on_disk} \
         b_ro={b_read_only_hydrated:?} freed={freed:?} b_freed_on_disk={b_freed_on_disk} b_ro_freed={b_read_only_freed:?} again={again:?} \
         b_ro_end={:?} fetches={}",
        out.status.success(),
        read_only(&a),
        read_only(&b),
        qa.fx.fetches.load(Ordering::SeqCst)
    );
    assert!(out.status.success() && read == "hello", "another process reads the bytes: {read:?}");
    assert!(a_on_disk, "the read hydrated it");
    assert_eq!(read_only(&a), Some(true), "and it is still read-only");
    assert!(hydrated.is_ok() && b_on_disk, "the engine's hydrate went through");
    assert_eq!(b_read_only_hydrated, Some(true));
    assert!(freed.is_ok() && !b_freed_on_disk, "the engine's dehydrate went through");
    assert_eq!(b_read_only_freed, Some(true));
    assert!(again.is_ok() && placeholders::fully_on_disk(&b), "and a hydrate again");
    assert_eq!(read_only(&b), Some(true));
    assert!(qa.fx.fetches.load(Ordering::SeqCst) >= 3, "three fetches through the provider");
}

/// A new version of a read-only file: the engine empties the old version's bytes (S6) and gives the
/// placeholder the new size (`resize_placeholder`, which writes `FILE_ATTRIBUTE_NORMAL`); it is
/// read-only after both.
#[test]
fn new_version_updates_read_only_placeholder() {
    let mut qa = QaRoot::new(OwnBackups::default());
    qa.connect();
    qa.row("0b5ec0de-0000-4000-8000-0000000000fa", "Backups/OtherPC", true);
    qa.row("0b5ec0de-0000-4000-8000-0000000000fb", "Backups/OtherPC/a.txt", false);
    bounded(&qa, "new_version_updates_read_only_placeholder pass", || crate::windows_cf::restore_placeholders(&qa.fx.db, &qa.root));
    let a = qa.path("Backups/OtherPC/a.txt");
    cmd(&qa, "new_version_updates_read_only_placeholder", &format!("type {}", quoted(&a)));
    let hydrated = placeholders::fully_on_disk(&a);

    let emptied = bounded(&qa, "new_version_updates_read_only_placeholder empty", || {
        crate::windows_cf::poison::empty_poisoned(&a, crate::windows_cf::poison::Check::Discard)
    });
    let (emptied_on_disk, read_only_emptied) = (placeholders::fully_on_disk(&a), read_only(&a));
    let resized = bounded(&qa, "new_version_updates_read_only_placeholder resize", || placeholders::resize_placeholder(&a, 11).map_err(|e| e.to_string()));
    let size = std::fs::symlink_metadata(&a).map(|metadata| metadata.len()).ok();
    println!(
        "ld0113 new_version: hydrated={hydrated} emptied={emptied:?} on_disk_after={emptied_on_disk} ro_emptied={read_only_emptied:?} \
         resized={resized:?} size={size:?} ro_end={:?}",
        read_only(&a)
    );
    assert!(hydrated, "the old version's bytes were on disk");
    assert_eq!(emptied, crate::windows_cf::poison::Emptied::Emptied, "the old bytes are emptied");
    assert!(!emptied_on_disk);
    assert_eq!(read_only_emptied, Some(true), "read-only after the emptying");
    assert!(resized.is_ok(), "the new version's size is applied");
    assert_eq!(size, Some(11));
    assert_eq!(read_only(&a), Some(true), "read-only after the resize");
}

/// A write open of a file under Backups is refused by the file system, from another process and from
/// this one, and nothing is fetched for it; the same append outside Backups works.
#[test]
fn write_open_of_backups_file_denied() {
    let mut qa = QaRoot::new(OwnBackups::default());
    qa.connect();
    qa.row("0b5ec0de-0000-4000-8000-0000000000fc", "Backups/OtherPC", true);
    qa.row("0b5ec0de-0000-4000-8000-0000000000fd", "Backups/OtherPC/a.txt", false);
    qa.row("0b5ec0de-0000-4000-8000-0000000000fe", "Documents", true);
    qa.row("0b5ec0de-0000-4000-8000-0000000000ff", "Documents/b.txt", false);
    bounded(&qa, "write_open_of_backups_file_denied pass", || crate::windows_cf::restore_placeholders(&qa.fx.db, &qa.root));
    let (a, b) = (qa.path("Backups/OtherPC/a.txt"), qa.path("Documents/b.txt"));

    let refused = cmd(&qa, "write_open_of_backups_file_denied", &format!("echo changed>> {}", quoted(&a)));
    let own_open = bounded(&qa, "write_open_of_backups_file_denied open", || {
        std::fs::OpenOptions::new().write(true).open(&a).map(|_| ()).map_err(|e| e.raw_os_error())
    });
    let fetches_for_refused = qa.fx.fetches.load(Ordering::SeqCst);
    let size_a = std::fs::symlink_metadata(&a).map(|metadata| metadata.len()).ok();
    let outside = cmd(&qa, "write_open_of_backups_file_denied outside", &format!("echo changed>> {}", quoted(&b)));
    let size_b = std::fs::symlink_metadata(&b).map(|metadata| metadata.len()).ok();
    println!(
        "ld0113 write_open: cmd_ok={} cmd_stderr={:?} own_open={own_open:?} fetches_for_refused={fetches_for_refused} size_a={size_a:?} \
         outside_ok={} size_b={size_b:?}",
        refused.status.success(),
        String::from_utf8_lossy(&refused.stderr).trim(),
        outside.status.success()
    );
    assert!(!refused.status.success(), "the append from another process is refused");
    assert_eq!(own_open, Err(Some(5)), "an open for writing here is ERROR_ACCESS_DENIED");
    assert_eq!(fetches_for_refused, 0, "nothing is fetched for a refused write");
    assert_eq!(size_a, Some(5), "the backup is unchanged");
    assert!(outside.status.success() && size_b.is_some_and(|size| size > 5), "a file outside Backups is still writable");
}

/// The manifest of a device folder (core's `is_hidden_backup_item`) is never minted in the drive; the
/// rest of the tree is.
#[test]
fn backups_manifest_is_never_minted() {
    let qa = QaRoot::new(OwnBackups::default());
    qa.row("0b5ec0de-0000-4000-8000-0000000001a1", "Backups/OtherPC", true);
    qa.row("0b5ec0de-0000-4000-8000-0000000001a2", &format!("Backups/OtherPC/{MANIFEST_NAME}"), false);
    qa.row("0b5ec0de-0000-4000-8000-0000000001a3", "Backups/OtherPC/Pictures", true);
    let created = bounded(&qa, "backups_manifest_is_never_minted pass", || crate::windows_cf::restore_placeholders(&qa.fx.db, &qa.root));
    let manifest = qa.path(&format!("Backups/OtherPC/{MANIFEST_NAME}"));
    println!("ld0113 manifest: created={created} manifest_on_disk={}", manifest.exists());
    assert_eq!(created, 2, "the device folder and its category folder");
    assert!(!manifest.exists(), "the manifest is not in the drive");
    assert!(qa.path("Backups/OtherPC/Pictures").is_dir());
}

/// A read-only file under Backups is removed by the engine (a backup deleted in the app) and by the
/// sign-out purge, each the way production removes it.
#[test]
fn engine_and_sign_out_remove_read_only_placeholders() {
    let qa = QaRoot::new(OwnBackups::default());
    qa.row("0b5ec0de-0000-4000-8000-0000000001a4", "Backups/OtherPC", true);
    qa.row("0b5ec0de-0000-4000-8000-0000000001a5", "Backups/OtherPC/a.txt", false);
    qa.row("0b5ec0de-0000-4000-8000-0000000001a6", "Backups/OtherPC/b.txt", false);
    bounded(&qa, "engine_and_sign_out_remove pass", || crate::windows_cf::restore_placeholders(&qa.fx.db, &qa.root));
    let (a, b) = (qa.path("Backups/OtherPC/a.txt"), qa.path("Backups/OtherPC/b.txt"));
    assert_eq!((read_only(&a), read_only(&b)), (Some(true), Some(true)));

    let removed = bounded(&qa, "engine_and_sign_out_remove engine", || {
        crate::engine_bridge::engine_remove(&a, crate::engine_bridge::RemovedKind::File).map_err(|e| e.raw_os_error())
    });
    // The sign-out purge's handle: attributes and DELETE, the reparse point itself, exclusive.
    let marked = bounded(&qa, "engine_and_sign_out_remove sign_out", || {
        let file = std::fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES.0 | DELETE.0)
            .share_mode(0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0 | FILE_FLAG_BACKUP_SEMANTICS.0)
            .open(&b)
            .map_err(|e| e.to_string())?;
        crate::windows_cf::read_only::mark_for_deletion(HANDLE(file.as_raw_handle())).map_err(|e| e.to_string())
    });
    println!("ld0113 remove_read_only: engine={removed:?} a_left={} sign_out={marked:?} b_left={}", a.exists(), b.exists());
    assert_eq!(removed, Ok(()), "the engine's own remove");
    assert!(!a.exists());
    assert_eq!(marked, Ok(()), "the sign-out purge's delete");
    assert!(!b.exists());
}
