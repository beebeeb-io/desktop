//! LD-0105 part 1, the Windows-native rungs (S0, W1, S6). Every test registers its OWN sync root:
//! a temporary folder, [`crate::windows_cf::QA_PROVIDER`]'s name and GUID, never the app's provider,
//! never the person's Beebeeb root, and unregisters it when it ends. Nothing here talks to a server.
//! Run with `--nocapture` to see the counts.

use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::Win32::Storage::CloudFilters::*;
use windows::core::PCWSTR;

use crate::unknown_check::{OnDisk, PlatformView};
use crate::windows_cf::callbacks::{fail_transfer, transfer_range};
use crate::windows_cf::fetch_progress::keep_fetch_alive;
use crate::windows_cf::placeholders;

type Callback = unsafe extern "system" fn(*const CF_CALLBACK_INFO, *const CF_CALLBACK_PARAMETERS);

/// A registered (and optionally connected) QA root; unregistered on drop, whatever the test did.
struct QaRoot {
    _dir: tempfile::TempDir,
    root: PathBuf,
    key: Option<CF_CONNECTION_KEY>,
}

impl QaRoot {
    fn new(hydration: CF_HYDRATION_POLICY_PRIMARY) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("qa-root");
        std::fs::create_dir(&root).unwrap();
        crate::windows_cf::register_sync_root_with(&root, &crate::windows_cf::QA_PROVIDER, hydration).unwrap();
        QaRoot {
            _dir: dir,
            root,
            key: None,
        }
    }

    fn connect(&mut self, fetch: Callback) {
        self.connect_with(fetch, None);
    }

    fn connect_with(&mut self, fetch: Callback, cancel: Option<Callback>) {
        let none = || CF_CALLBACK_REGISTRATION {
            Type: CF_CALLBACK_TYPE_NONE,
            Callback: None,
        };
        let table = [
            CF_CALLBACK_REGISTRATION {
                Type: CF_CALLBACK_TYPE_FETCH_DATA,
                Callback: Some(fetch),
            },
            match cancel {
                Some(cancel) => CF_CALLBACK_REGISTRATION {
                    Type: CF_CALLBACK_TYPE_CANCEL_FETCH_DATA,
                    Callback: Some(cancel),
                },
                None => none(),
            },
            none(),
        ];
        let wide: Vec<u16> = self.root.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: the path and the table outlive the call; the callback is a static function.
        let key = unsafe {
            CfConnectSyncRoot(
                PCWSTR(wide.as_ptr()),
                table.as_ptr(),
                None,
                CF_CONNECT_FLAG_REQUIRE_PROCESS_INFO,
            )
        }
        .unwrap();
        self.key = Some(key);
    }

    /// What an app restart does to the connection: it ends.
    fn disconnect(&mut self) {
        if let Some(key) = self.key.take() {
            // SAFETY: the key of the connection made in `connect`, disconnected once.
            let _ = unsafe { CfDisconnectSyncRoot(key) };
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
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

/// The bytes every test file holds: deterministic, so a read can be checked.
fn content(len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u32).wrapping_mul(2_654_435_761).to_le_bytes()[1])
        .collect()
}

/// The keys and range of one FETCH_DATA request.
#[derive(Clone, Copy)]
struct Request {
    connection: CF_CONNECTION_KEY,
    transfer: i64,
    request: i64,
    offset: i64,
    length: i64,
    file_size: i64,
}

/// SAFETY: both pointers come from a live callback.
unsafe fn request_of(info: *const CF_CALLBACK_INFO, params: *const CF_CALLBACK_PARAMETERS) -> Request {
    let info = unsafe { &*info };
    let fetch = unsafe { (*params).Anonymous.FetchData };
    Request {
        connection: info.ConnectionKey,
        transfer: info.TransferKey,
        request: info.RequestKey,
        offset: fetch.RequiredFileOffset,
        length: fetch.RequiredLength,
        file_size: info.FileSize,
    }
}

/// A fetch callback that serves at most the first 64 KiB of what was asked and fails the rest: it
/// leaves a PARTLY hydrated placeholder behind.
unsafe extern "system" fn serve_first_64k(info: *const CF_CALLBACK_INFO, params: *const CF_CALLBACK_PARAMETERS) {
    if info.is_null() || params.is_null() {
        return;
    }
    // SAFETY: valid for the duration of the callback.
    let r = unsafe { request_of(info, params) };
    let data = content(r.file_size as usize);
    let served = r.length.min(64 * 1024);
    // SAFETY: the keys of this live request.
    unsafe {
        let _ = transfer_range(r.connection, r.transfer, r.request, &data, || false, r.offset, served);
        if served < r.length {
            fail_transfer(
                r.connection,
                r.transfer,
                r.request,
                r.offset + served,
                r.length - served,
            );
        }
    }
}

/// `(on disk, in sync, pinned)` of a placeholder, read without fetching anything.
fn census_of(path: &Path) -> (OnDisk, bool, bool) {
    const FILE_ATTRIBUTE_PINNED: u32 = 0x0008_0000;
    let pinned = std::fs::symlink_metadata(path)
        .map(|m| m.file_attributes() & FILE_ATTRIBUTE_PINNED != 0)
        .unwrap_or(false);
    match placeholders::platform_view(path) {
        PlatformView::Placeholder { on_disk, in_sync, .. } => (on_disk, in_sync, pinned),
        other => panic!("{} is not a placeholder: {other:?}", path.display()),
    }
}

fn census(root: &QaRoot, names: &[&str]) -> Vec<(String, OnDisk, bool, bool)> {
    names
        .iter()
        .map(|n| {
            let (on_disk, in_sync, pinned) = census_of(&root.path(n));
            (n.to_string(), on_disk, in_sync, pinned)
        })
        .collect()
}

/// The five kinds of placeholder the W1 rung counts, minted on `root` (connected with
/// [`serve_first_64k`] for the partly hydrated one).
fn mint_five(root: &QaRoot) -> Vec<&'static str> {
    const MIB: i64 = 1 << 20;
    for (id, name) in [
        ("qa-cloud", "cloud.bin"),
        ("qa-cloud-pinned", "cloud-pinned.bin"),
        ("qa-partial", "partial.bin"),
    ] {
        assert!(placeholders::create_placeholder(&root.root, id, name, MIB, 1_700_000_000, false).unwrap());
    }
    placeholders::set_pin_state(&root.path("cloud-pinned.bin"), true, false).unwrap();
    // One read of the first byte: 64 KiB arrive, the rest is refused, the read may fail.
    let mut one = [0u8; 1];
    let _ = std::fs::File::open(root.path("partial.bin")).and_then(|mut f| std::io::Read::read(&mut f, &mut one));
    for (id, name) in [("qa-local", "local.bin"), ("qa-local-pinned", "local-pinned.bin")] {
        std::fs::write(root.path(name), content(MIB as usize)).unwrap();
        placeholders::convert_to_in_sync_placeholder(&root.path(name), id).unwrap();
    }
    placeholders::set_pin_state(&root.path("local-pinned.bin"), true, false).unwrap();
    vec![
        "cloud.bin",
        "cloud-pinned.bin",
        "partial.bin",
        "local.bin",
        "local-pinned.bin",
    ]
}

// ── S0: a fetch longer than Windows' 60 s limit survives on progress reports alone ──────────────

static S0_IMAGES: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// 90 s of "download" (the meter grows every 2 s, as body bytes would), with the production
/// [`keep_fetch_alive`] reporting it, then the whole range transferred at once, as today's whole-file
/// path does after its download.
unsafe extern "system" fn slow_fetch(info: *const CF_CALLBACK_INFO, params: *const CF_CALLBACK_PARAMETERS) {
    if info.is_null() || params.is_null() {
        return;
    }
    // SAFETY: valid for the duration of the callback.
    let r = unsafe { request_of(info, params) };
    // SAFETY: as above; the root is connected with REQUIRE_PROCESS_INFO.
    let process = unsafe { (*info).ProcessInfo };
    if !process.is_null() {
        let image = unsafe { (*process).ImagePath };
        // SAFETY: a NUL-terminated wide string valid for the callback.
        let text = unsafe { image.to_string() }.unwrap_or_default();
        S0_IMAGES.lock().unwrap_or_else(|p| p.into_inner()).push(text);
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    let meter = crate::transfer_progress::ReceiveMeter::default();
    let step = (r.file_size / 45).max(1) as u64;
    runtime.block_on(async {
        tokio::select! {
            never = keep_fetch_alive(r.connection, r.transfer, r.file_size, &meter) => match never {},
            () = async {
                for _ in 0..45 {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    meter.add(step);
                }
            } => {}
        }
    });
    let data = content(r.file_size as usize);
    // SAFETY: the keys of this live request.
    if unsafe { transfer_range(r.connection, r.transfer, r.request, &data, || false, r.offset, r.length) }.is_err() {
        unsafe { fail_transfer(r.connection, r.transfer, r.request, r.offset, r.length) };
    }
}

#[test]
fn ld0105_s0_native_a_90_s_fetch_kept_alive_by_progress_reports_completes() {
    let mut qa = QaRoot::new(crate::windows_cf::HYDRATION_POLICY);
    qa.connect(slow_fetch);
    let size = 4 << 20;
    assert!(placeholders::create_placeholder(&qa.root, "qa-slow", "slow.mp4", size, 1_700_000_000, false).unwrap());
    let started = Instant::now();
    let read = std::fs::read(qa.path("slow.mp4"));
    let took = started.elapsed();
    println!(
        "ld0105-s0-native: read took {took:?}, result {:?}",
        read.as_ref().map(Vec::len).map_err(|e| e.to_string())
    );
    for image in S0_IMAGES.lock().unwrap().iter() {
        // W3 needs the format Windows uses here (DOS or NT path); record it.
        println!(
            "ld0105-s0-native: requester ImagePath = {image}; this test = {}",
            std::env::current_exe().unwrap().display()
        );
    }
    let bytes = read.unwrap_or_else(|e| panic!("the open failed after {took:?}: {e} (0x800701AA = request timeout)"));
    // The discriminating count (measured 2026-11-05): under PROGRESSIVE a silent fetch that times out at
    // 60 s no longer fails the open; Windows issues a SECOND fetch instead, which in the app is a second
    // whole-file download (LD-0105 H7). With the reports there is exactly one.
    let fetches = S0_IMAGES.lock().unwrap().len();
    assert_eq!(fetches, 1, "one fetch for one open; {fetches} means the 60 s timer fired");
    assert_eq!(bytes, content(size as usize), "the right bytes");
    assert!(
        took >= Duration::from_secs(85),
        "the fetch really took longer than Windows' 60 s limit: {took:?}"
    );
}

// ── W1: the 0.8.21 → 0.8.22 registration keeps every placeholder ────────────────────────────────

#[test]
fn ld0105_w1_native_the_upgrade_from_partial_to_progressive_keeps_every_placeholder() {
    let mut qa = QaRoot::new(CF_HYDRATION_POLICY_PARTIAL); // a 0.8.21 install
    qa.connect(serve_first_64k);
    let names = mint_five(&qa);
    let before = census(&qa, &names);
    println!("ld0105-w1-native: before {before:?}");
    assert_eq!(
        crate::windows_cf::hydration_policy_of(&qa.root).unwrap().0,
        CF_HYDRATION_POLICY_PARTIAL.0
    );
    // Every on-disk state and both pin states are there before the upgrade. (Measured: pinning a
    // cloud-only placeholder on a connected PARTIAL root makes Windows fetch it, so `cloud-pinned.bin`
    // is partly hydrated here, not cloud-only.)
    for on_disk in [OnDisk::Nothing, OnDisk::Partly, OnDisk::Wholly] {
        assert!(
            before.iter().any(|(_, d, _, _)| *d == on_disk),
            "a {on_disk:?} placeholder before the upgrade: {before:?}"
        );
    }
    assert!(
        before.iter().any(|(_, _, _, p)| *p) && before.iter().any(|(_, _, _, p)| !*p),
        "pinned and unpinned: {before:?}"
    );
    // The 0.8.22 engine start: the old process's connection is gone, then the same registration call
    // the app makes before it connects, with UPDATE, no unregister.
    qa.disconnect();
    crate::windows_cf::register_sync_root_with(
        &qa.root,
        &crate::windows_cf::QA_PROVIDER,
        crate::windows_cf::HYDRATION_POLICY,
    )
    .unwrap();
    let after = census(&qa, &names);
    println!("ld0105-w1-native: after  {after:?}");
    assert_eq!(after, before, "0 placeholders lost or changed");
    let policy = crate::windows_cf::hydration_policy_of(&qa.root).unwrap();
    println!("ld0105-w1-native: policy read back {policy:?}");
    assert!(
        crate::windows_cf::hydration_policy_is_ours(Some(policy)),
        "PROGRESSIVE, no modifier: {policy:?}"
    );
}

#[test]
fn ld0105_w1_native_a_fresh_registration_is_progressive_and_d6_is_recorded() {
    use windows::Storage::Provider::StorageProviderSyncRootManager;
    use windows::core::HSTRING;
    let mut qa = QaRoot::new(crate::windows_cf::HYDRATION_POLICY);
    let id = crate::windows_cf::shell_sync_root_id_as(&qa.root, crate::windows_cf::QA_PROVIDER.name);
    struct ShellCleanup(String);
    impl Drop for ShellCleanup {
        fn drop(&mut self) {
            let _ = StorageProviderSyncRootManager::Unregister(&HSTRING::from(self.0.as_str()));
            let _ = crate::windows_cf::remove_owned_shell_key(&self.0);
        }
    }
    // First install / sign-in after a sign-out: Win32 first, then the Explorer registration.
    crate::windows_cf::register_shell_sync_root_as(&qa.root, &crate::windows_cf::QA_PROVIDER).unwrap();
    let _cleanup = ShellCleanup(id.clone());
    let policy = crate::windows_cf::hydration_policy_of(&qa.root).unwrap();
    println!("ld0105-w1-native: fresh registration, policy read back {policy:?}");
    assert!(
        crate::windows_cf::hydration_policy_is_ours(Some(policy)),
        "the WinRT values must not override PROGRESSIVE/NONE: {policy:?}"
    );
    qa.connect(serve_first_64k);
    let names = mint_five(&qa);
    let before = census(&qa, &names);
    // The next engine start: both registrations again. The Explorer one is kept, not unregistered.
    crate::windows_cf::register_sync_root_with(
        &qa.root,
        &crate::windows_cf::QA_PROVIDER,
        crate::windows_cf::HYDRATION_POLICY,
    )
    .unwrap();
    crate::windows_cf::register_shell_sync_root_as(&qa.root, &crate::windows_cf::QA_PROVIDER).unwrap();
    let after = census(&qa, &names);
    assert_eq!(after, before, "a second start keeps every placeholder");
    // The record (spec §5.1 D-6, plan W1): what a WinRT Unregister → Register does to this root.
    // Not asserted: this is the measurement that says why nothing in the app may do it. Disconnected
    // first, as at sign-out (a connected root refuses it: 0x8007017C, measured).
    qa.disconnect();
    let unregistered =
        StorageProviderSyncRootManager::Unregister(&HSTRING::from(id.as_str())).map_err(|e| e.to_string());
    let left: Vec<(String, bool)> = names.iter().map(|n| (n.to_string(), qa.path(n).exists())).collect();
    let views: Vec<String> = names
        .iter()
        .map(|n| format!("{n}={:?}", placeholders::platform_view(&qa.path(n))))
        .collect();
    println!("ld0105-w1-native: D-6 record, WinRT Unregister {unregistered:?}: present {left:?}");
    println!("ld0105-w1-native: D-6 record, views {views:?}");
}

/// Set once the S6 fixture is poisoned: from then on its provider refuses every fetch, as the
/// production callback refuses a poisoned file (I5). Without it the platform's re-issued fetch for the
/// rest of a partly hydrated file is served after the poison, which production never does.
static S6_POISONED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

unsafe extern "system" fn s6_fetch(info: *const CF_CALLBACK_INFO, params: *const CF_CALLBACK_PARAMETERS) {
    if !S6_POISONED.load(std::sync::atomic::Ordering::SeqCst) {
        // SAFETY: forwarded as received.
        return unsafe { serve_first_64k(info, params) };
    }
    if info.is_null() || params.is_null() {
        return;
    }
    // SAFETY: valid for the duration of the callback.
    let r = unsafe { request_of(info, params) };
    unsafe { fail_transfer(r.connection, r.transfer, r.request, r.offset, r.length) };
}

// ── S6: the empty step, against hand-poisoned placeholders ───────────────────────────────────────

fn qa_db(dir: &Path, rows: &[(&str, &str)]) -> crate::state_db::StateDb {
    let db = crate::state_db::StateDb::open(dir.join("state.db")).unwrap();
    for (id, rel) in rows {
        db.upsert_file(&crate::state_db::FileEntry {
            file_id: (*id).into(),
            path: (*rel).into(),
            status: crate::state_db::FileStatus::Error,
            size_bytes: 1 << 20,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: crate::state_db::ItemKind::File,
        })
        .unwrap();
    }
    db
}

/// Poison `name` as the W2 fetch path does (`poison_now`): not in sync, then the content key, then the
/// record.
fn poison(db: &crate::state_db::StateDb, qa: &QaRoot, id: &str, name: &str) {
    let key_before = crate::windows_cf::poison::content_key(&qa.path(name));
    crate::windows_cf::poison::poison_now(db, id, &qa.path(name), "corrupt", key_before);
    assert!(db.poison(id).unwrap().unwrap().key.is_some(), "the key of {name} is readable");
}

#[test]
fn ld0105_s6_native_the_empty_step_dehydrates_a_poisoned_placeholder_in_the_measured_order() {
    use crate::windows_cf::poison::{Check, Emptied, SweepKind, empty_poisoned, sweep_poisoned};
    let mut qa = QaRoot::new(crate::windows_cf::HYDRATION_POLICY);
    qa.connect(s6_fetch); // this process is the connected provider: only it may set IN_SYNC
    let names = mint_five(&qa);
    assert_eq!(names.len(), 5);
    S6_POISONED.store(true, std::sync::atomic::Ordering::SeqCst);
    let state = tempfile::tempdir().unwrap();
    let db_path = state.path().to_path_buf();
    {
        let db = qa_db(
            &db_path,
            &[
                ("qa-local-pinned", "local-pinned.bin"),
                ("qa-partial", "partial.bin"),
                ("qa-local", "local.bin"),
            ],
        );
        // (a) whole, pinned, not in sync: the P1b case.
        poison(&db, &qa, "qa-local-pinned", "local-pinned.bin");
        assert_eq!(census_of(&qa.path("local-pinned.bin")), (OnDisk::Wholly, false, true));
        // While someone has it open: nothing is done (a sharing violation), the poison stays.
        let open = std::fs::File::open(qa.path("local-pinned.bin")).unwrap();
        let stamp = db.poison("qa-local-pinned").unwrap().unwrap();
        assert_eq!(
            empty_poisoned(&qa.path("local-pinned.bin"), Check::Key(stamp.key)),
            Emptied::Busy
        );
        assert_eq!(
            census_of(&qa.path("local-pinned.bin")),
            (OnDisk::Wholly, false, true),
            "untouched while open"
        );
        drop(open);
        // (b) partly hydrated, not in sync.
        assert_eq!(census_of(&qa.path("partial.bin")).0, OnDisk::Partly, "64 KiB of 1 MiB");
        poison(&db, &qa, "qa-partial", "partial.bin");
        // (c) whole, not in sync, then written to by the person: never thrown away.
        poison(&db, &qa, "qa-local", "local.bin");
        std::thread::sleep(Duration::from_millis(50));
        std::fs::OpenOptions::new()
            .write(true)
            .open(qa.path("local.bin"))
            .and_then(|mut f| std::io::Write::write_all(&mut f, b"mine"))
            .unwrap();
    }
    // (d) the app restarted while poisoned: a new database handle, and the sweep that runs at start.
    let db = crate::state_db::StateDb::open(db_path.join("state.db")).unwrap();
    assert_eq!(db.poisoned().unwrap().len(), 3, "the poisons outlived the restart");
    // The sweep, again at the next "tick" for a file someone else had open (`Busy`), as the engine
    // does, at most 10 times. On its own thread: a step that never returned would hang the suite.
    let sweeps = {
        let (db_file, root) = (db_path.join("state.db"), qa.root.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let db = crate::state_db::StateDb::open(db_file).unwrap();
            let mut sweeps = vec![sweep_poisoned(&db, &root, SweepKind::ProcessStart)];
            while sweeps.len() < 10 && sweeps.last().unwrap().iter().any(|(_, o)| *o == Emptied::Busy) {
                std::thread::sleep(Duration::from_millis(250));
                sweeps.push(sweep_poisoned(&db, &root, SweepKind::Tick));
            }
            let _ = tx.send(sweeps);
        });
        rx.recv_timeout(Duration::from_secs(60))
            .expect("the sweep returns (a step waiting on its own oplock would not)")
    };
    println!("ld0105-s6-native: {} sweep(s), outcomes {sweeps:?}", sweeps.len());
    // Each file's last outcome (an emptied file has no poison left for a later sweep).
    let mut last = std::collections::BTreeMap::new();
    for (id, outcome) in sweeps.into_iter().flatten() {
        last.insert(id, outcome);
    }
    let emptied = last.values().filter(|o| **o == Emptied::Emptied).count();
    let pinned_after = census_of(&qa.path("local-pinned.bin"));
    let partial_after = census_of(&qa.path("partial.bin"));
    let changed_after = census_of(&qa.path("local.bin"));
    println!(
        "ld0105-s6-native: emptied={emptied} pinned={pinned_after:?} partial={partial_after:?} changed={changed_after:?}"
    );
    assert_eq!(emptied, 2);
    assert_eq!(
        pinned_after,
        (OnDisk::Nothing, true, true),
        "emptied, in sync, its pin restored"
    );
    assert_eq!(partial_after, (OnDisk::Nothing, true, false), "emptied, in sync");
    assert_eq!(changed_after.0, OnDisk::Wholly, "the person's write is kept");
    assert!(!changed_after.1, "and stays not in sync");
    assert_eq!(db.poison("qa-local-pinned").unwrap(), None);
    assert_eq!(db.poison("qa-partial").unwrap(), None);
    // Amendment 4: a content write since the poison: never emptied, never cleared silently.
    assert_eq!(last.get("qa-local"), Some(&Emptied::NeedsAttention), "{last:?}");
    let changed = db.poison("qa-local").unwrap().expect("the poison stays");
    assert!(changed.needs_attention, "the row needs attention");
    assert_eq!(
        sweep_poisoned(&db, &qa.root, SweepKind::Tick),
        vec![],
        "a needs-attention row is not opened on a tick"
    );
    // A poisoned file refuses every fetch until it is emptied (I5) is the fetch callback's first check;
    // the emptied ones fetch again: a read of the partial file asks for bytes once more.
    S6_POISONED.store(false, std::sync::atomic::Ordering::SeqCst); // the poison is cleared: production serves again
    let mut one = [0u8; 1];
    let reread = std::fs::File::open(qa.path("partial.bin")).and_then(|mut f| std::io::Read::read(&mut f, &mut one));
    assert!(reread.is_ok(), "an emptied placeholder fetches again: {reread:?}");
}

/// A provider that refuses every fetch, as production refuses every fetch of a poisoned file (I5).
unsafe extern "system" fn refuse_every_fetch(info: *const CF_CALLBACK_INFO, params: *const CF_CALLBACK_PARAMETERS) {
    if info.is_null() || params.is_null() {
        return;
    }
    // SAFETY: valid for the duration of the callback.
    let r = unsafe { request_of(info, params) };
    // SAFETY: the keys of this live request.
    unsafe { fail_transfer(r.connection, r.transfer, r.request, r.offset, r.length) };
}

/// A whole, pinned, poisoned placeholder at `name` (the P1b case), and its poison's stamp.
fn poisoned_whole_pinned(qa: &QaRoot, name: &str) -> Option<(i64, i64)> {
    let path = qa.path(name);
    std::fs::write(&path, content(1 << 20)).unwrap();
    placeholders::convert_to_in_sync_placeholder(&path, name).unwrap();
    placeholders::set_pin_state(&path, true, false).unwrap();
    placeholders::mark_not_in_sync_path(&path).unwrap();
    assert_eq!(census_of(&path), (OnDisk::Wholly, false, true), "{name} poisoned as a failed fetch leaves it");
    crate::windows_cf::poison::content_key(&path)
}

/// The windows-latest red (desktop PR 224): between two of the empty step's calls the step's protected
/// handle stopped being referenceable (an oplock break), and every later call on it failed with
/// `0x80070006`, the restore of the pin included: `Failed("in_sync")`, `Failed("dehydrate")` or
/// `Failed("still_on_disk")`, NOT_IN_SYNC set again by path, the pin lost. Here another open comes
/// from a thread of this process at each point between two calls, which breaks the oplock the same
/// way. The step holds a reference and works on the Win32 handle, so that open waits until the step
/// ends, and the step completes.
#[test]
fn ld0105_s6_native_an_open_between_two_steps_waits_until_the_empty_step_ends() {
    use crate::windows_cf::poison::{Check, Emptied, Step, empty_poisoned_with};
    let mut qa = QaRoot::new(crate::windows_cf::HYDRATION_POLICY);
    qa.connect(refuse_every_fetch); // this process is the connected provider: only it may set IN_SYNC
    let mut seen = Vec::new();
    for at in [Step::Checked, Step::InSync, Step::Dehydrated] {
        let name = format!("open-at-{at:?}.bin");
        let path = qa.path(&name);
        let stamp = poisoned_whole_pinned(&qa, &name);
        let (tx, rx) = std::sync::mpsc::channel();
        let mut opener = None;
        let mut done_early = None;
        let started = Instant::now();
        let outcome = empty_poisoned_with(&path, Check::Key(stamp), &mut |step, _| {
            if step == at {
                let (path, tx) = (path.clone(), tx.clone());
                opener = Some(std::thread::spawn(move || {
                    let opened = std::fs::File::open(&path).map(drop).map_err(|e| e.raw_os_error());
                    let _ = tx.send((opened, started.elapsed()));
                }));
                // Time for that open to reach the oplock: on an unreferenced handle the break
                // invalidates it well within this, and the open returns.
                std::thread::sleep(Duration::from_millis(300));
                done_early = rx.try_recv().ok();
            }
        });
        let step_took = started.elapsed();
        let (opened, open_took) = match done_early {
            Some(early) => early,
            None => rx
                .recv_timeout(Duration::from_secs(30))
                .expect("the other open returns once the step's handle is closed"),
        };
        opener.expect("the step reached the point").join().unwrap();
        let after = census_of(&path);
        println!(
            "ld0105-s6-native: open at {at:?}: outcome {outcome:?}, step {step_took:?}, other open {opened:?} after {open_took:?}, returned during the step: {}, census {after:?}",
            done_early.is_some()
        );
        seen.push((at, outcome, after, done_early.is_some(), opened));
    }
    // Every point: emptied, in sync, its pin restored; the other open waited for the step, then opened.
    let want = |at| (at, Emptied::Emptied, (OnDisk::Nothing, true, true), false, Ok(()));
    let wanted: Vec<_> = [Step::Checked, Step::InSync, Step::Dehydrated].into_iter().map(want).collect();
    assert_eq!(seen, wanted, "(point, outcome, census, the other open returned during the step, its result)");
}

/// The failure path (spec S6 step 6), then the next sweep. A step that fails after the pin was cleared
/// restores NOT_IN_SYNC and the pin before the handle is released. The failure is made, not hoped for:
/// at `Step::InSync` the file is set not in sync again through the step's own handle, and a placeholder
/// that is not in sync refuses to dehydrate (`0x80070179`, P1b). Contract Amendment 4's measurement:
/// those restore writes (through the handle) leave the content key (`LastWriteTime` + size) as it was,
/// so the poison needs no new stamp and the next sweep empties the file. (Their ChangeTime moves: the
/// part-1 key, printed.)
#[test]
fn ld0105_s6_native_a_failed_step_restores_not_in_sync_and_the_pin() {
    use crate::windows_cf::poison::{Emptied, Step, SweepKind, change_time, content_key, sweep_poisoned, sweep_poisoned_with};
    let mut qa = QaRoot::new(crate::windows_cf::HYDRATION_POLICY);
    qa.connect(refuse_every_fetch);
    let path = qa.path("failed.bin");
    let key = poisoned_whole_pinned(&qa, "failed.bin");
    let change = change_time(&path);
    let state = tempfile::tempdir().unwrap();
    let db = qa_db(state.path(), &[("qa-failed", "failed.bin")]);
    db.record_poison("qa-failed", "corrupt", key, 1).unwrap();
    // Time for a moved file time to show (file times are not finer than the clock that sets them).
    std::thread::sleep(Duration::from_millis(50));
    let mut pin_when_it_failed = None;
    let first = sweep_poisoned_with(&db, &qa.root, SweepKind::ProcessStart, None, &mut |step, handle| {
        if step == Step::InSync {
            // SAFETY: the step's own open handle, for the duration of this call.
            unsafe {
                CfSetInSyncState(handle, CF_IN_SYNC_STATE_NOT_IN_SYNC, CF_SET_IN_SYNC_FLAG_NONE, None).unwrap();
            }
            // Read through the same handle: another open here would wait on the step's own oplock.
            use windows::Win32::Storage::FileSystem::{FILE_BASIC_INFO, FileBasicInfo, GetFileInformationByHandleEx};
            let mut basic = FILE_BASIC_INFO::default();
            // SAFETY: as above; the call writes only into `basic`, whose size it is given.
            unsafe {
                GetFileInformationByHandleEx(
                    handle,
                    FileBasicInfo,
                    (&mut basic as *mut FILE_BASIC_INFO).cast(),
                    std::mem::size_of::<FILE_BASIC_INFO>() as u32,
                )
                .unwrap();
            }
            const FILE_ATTRIBUTE_PINNED: u32 = 0x0008_0000;
            pin_when_it_failed = Some(basic.FileAttributes & FILE_ATTRIBUTE_PINNED != 0);
        }
    });
    let after = census_of(&path);
    let (key_after, change_after) = (content_key(&path), change_time(&path));
    println!(
        "ld0105-a4-native: failed step: {first:?}, pinned when it failed {pin_when_it_failed:?}, census {after:?}, key {key:?} -> {key_after:?}, ChangeTime {change:?} -> {change_after:?}"
    );
    assert_eq!(first, vec![("qa-failed".to_string(), Emptied::Failed("dehydrate"))], "a not-in-sync placeholder refuses to dehydrate");
    assert_eq!(pin_when_it_failed, Some(false), "the step had cleared the pin");
    assert_eq!(after, (OnDisk::Wholly, false, true), "untouched, not in sync, pinned again");
    assert_eq!(key_after, key, "Amendment 4: our own restore writes leave LastWriteTime and the size");
    // The next tick: the key is unchanged, so the file is emptied.
    let second = sweep_poisoned(&db, &qa.root, SweepKind::Tick);
    println!("ld0105-a4-native: the next sweep: {second:?}, census {:?}", census_of(&path));
    assert_eq!(second, vec![("qa-failed".to_string(), Emptied::Emptied)]);
    assert_eq!(census_of(&path), (OnDisk::Nothing, true, true));
    assert_eq!(db.poison("qa-failed").unwrap(), None);
}

/// Contract Amendment 4's measurement, by path: every restore write we make (not in sync, the pin set
/// and cleared, in sync) and a hydration's own TRANSFER_DATA leave the content key; a content write
/// of the same size moves it. If this ever fails, the key is not the content and Amendment 4's
/// fallback applies (every key change needs attention): stop and report (honesty trigger 3).
#[test]
fn ld0105_a4_native_our_own_writes_leave_the_content_key_and_a_content_write_moves_it() {
    use crate::windows_cf::poison::{change_time, content_key};
    let mut qa = QaRoot::new(crate::windows_cf::HYDRATION_POLICY);
    qa.connect(serve_first_64k);
    let path = qa.path("keyed.bin");
    std::fs::write(&path, content(1 << 20)).unwrap();
    placeholders::convert_to_in_sync_placeholder(&path, "qa-keyed").unwrap();
    let key = content_key(&path);
    let change = change_time(&path);
    assert!(key.is_some_and(|k| k.1 == 1 << 20), "{key:?}");
    let mut seen = Vec::new();
    let mut step = |what: &'static str| {
        std::thread::sleep(Duration::from_millis(30));
        seen.push((what, content_key(&path), change_time(&path)));
    };
    placeholders::mark_not_in_sync_path(&path).unwrap();
    step("not in sync, by path");
    placeholders::set_pin_state(&path, true, false).unwrap();
    step("pinned, by path");
    placeholders::set_pin_state(&path, false, false).unwrap();
    step("unpinned, by path");
    {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: NUL-terminated, alive for the call.
        unsafe { crate::windows_cf::callbacks::mark_in_sync(&wide) };
    }
    step("in sync, by path");
    // A hydration's TRANSFER_DATA (the 64 KiB a read of the first byte brings) on a cloud-only one.
    assert!(placeholders::create_placeholder(&qa.root, "qa-hydrated", "hydrated.bin", 1 << 20, 1_700_000_000, false).unwrap());
    let hydrated = qa.path("hydrated.bin");
    let before_transfer = content_key(&hydrated);
    let mut one = [0u8; 1];
    let _ = std::fs::File::open(&hydrated).and_then(|mut f| std::io::Read::read(&mut f, &mut one));
    let after_transfer = content_key(&hydrated);
    // A content write of the same size.
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, b"mine"))
        .unwrap();
    step("a content write");
    println!("ld0105-a4-native: start key {key:?} ChangeTime {change:?}; steps {seen:?}; transfer {before_transfer:?} -> {after_transfer:?}");
    for (what, k, _) in &seen[..4] {
        assert_eq!(*k, key, "{what}: our own write leaves the content key");
    }
    assert!(seen[..4].iter().any(|(_, _, c)| *c != change), "ChangeTime moves on our own writes (why part 1's key was wrong)");
    assert_eq!(after_transfer, before_transfer, "a hydration's TRANSFER_DATA leaves the content key");
    assert_ne!(seen[4].1, key, "a content write moves the content key");
}

/// Amendment 4: a pin the step cannot put back through its handle is set again by path. Red on the
/// mutant "no by-path retry" (the pin is lost).
#[test]
fn ld0105_a4_native_a_failed_pin_restore_through_the_handle_is_retried_by_path() {
    use crate::windows_cf::poison::{Emptied, FAIL_PIN_RESTORE_BY_HANDLE, SweepKind, sweep_poisoned};
    let mut qa = QaRoot::new(crate::windows_cf::HYDRATION_POLICY);
    qa.connect(refuse_every_fetch);
    let key = poisoned_whole_pinned(&qa, "pin.bin");
    let state = tempfile::tempdir().unwrap();
    let db = qa_db(state.path(), &[("qa-pin", "pin.bin")]);
    db.record_poison("qa-pin", "corrupt", key, 1).unwrap();
    FAIL_PIN_RESTORE_BY_HANDLE.set(true);
    let outcome = sweep_poisoned(&db, &qa.root, SweepKind::Tick);
    FAIL_PIN_RESTORE_BY_HANDLE.set(false);
    let after = census_of(&qa.path("pin.bin"));
    println!("ld0105-a4-native: pin restore by handle refused: {outcome:?}, census {after:?}");
    assert_eq!(outcome, vec![("qa-pin".to_string(), Emptied::Emptied)]);
    assert_eq!(after, (OnDisk::Nothing, true, true), "emptied, and pinned again by path");
}

/// Amendment 4: "Discard and download again" empties a needs-attention file whatever was written to
/// it, clears the poison, then fetches it. Red on the mutant "discard checks the key" (it is refused).
#[test]
fn ld0105_a4_native_discard_and_download_again_empties_then_fetches() {
    use crate::windows_cf::poison::{Emptied, SweepKind, discard_now, sweep_poisoned};
    let mut qa = QaRoot::new(crate::windows_cf::HYDRATION_POLICY);
    qa.connect(refuse_every_fetch);
    let key = poisoned_whole_pinned(&qa, "edited.bin");
    let state = tempfile::tempdir().unwrap();
    let db = qa_db(state.path(), &[("qa-edited", "edited.bin")]);
    db.record_poison("qa-edited", "corrupt", key, 1).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    std::fs::OpenOptions::new()
        .write(true)
        .open(qa.path("edited.bin"))
        .and_then(|mut f| std::io::Write::write_all(&mut f, b"mine"))
        .unwrap();
    assert_eq!(sweep_poisoned(&db, &qa.root, SweepKind::Close), vec![("qa-edited".to_string(), Emptied::NeedsAttention)]);
    assert_eq!(census_of(&qa.path("edited.bin")).0, OnDisk::Wholly, "kept as it is");
    let mut fetched = None;
    let result = discard_now(&db, &qa.root, "qa-edited", |path| fetched = Some(path));
    println!("ld0105-a4-native: discard {result:?}, census {:?}", census_of(&qa.path("edited.bin")));
    assert_eq!(result, Ok(()));
    assert_eq!(census_of(&qa.path("edited.bin")), (OnDisk::Nothing, true, true), "emptied, its pin kept");
    assert_eq!(db.poison("qa-edited").unwrap(), None, "the row ends");
    assert_eq!(fetched, Some(qa.path("edited.bin")), "then it is fetched again");
}

/// Amendment 4: a poison with no key needs attention (never cleared); a `Missing` row is cleared only
/// at the first sweep of a new process, never at a later tick.
#[test]
fn ld0105_s6_native_a_keyless_poison_needs_attention_and_missing_clears_only_at_process_start() {
    use crate::windows_cf::poison::{Emptied, SweepKind, sweep_poisoned};
    let mut qa = QaRoot::new(crate::windows_cf::HYDRATION_POLICY);
    qa.connect(refuse_every_fetch);
    let _ = poisoned_whole_pinned(&qa, "keyless.bin");
    let state = tempfile::tempdir().unwrap();
    let db = qa_db(state.path(), &[("qa-keyless", "keyless.bin"), ("qa-missing", "missing.bin")]);
    db.record_poison("qa-keyless", "identity_mismatch", None, 1).unwrap();
    db.record_poison("qa-missing", "corrupt", Some((1, 2)), 1).unwrap();
    let tick = sweep_poisoned(&db, &qa.root, SweepKind::Tick);
    let kept_on_tick = db.poison("qa-missing").unwrap().is_some();
    let start = sweep_poisoned(&db, &qa.root, SweepKind::ProcessStart);
    println!("ld0105-s6-native: tick {tick:?}, start {start:?}");
    assert!(tick.contains(&("qa-keyless".to_string(), Emptied::NeedsAttention)), "{tick:?}");
    assert!(db.poison("qa-keyless").unwrap().unwrap().needs_attention, "no key: needs attention, never cleared");
    assert_eq!(census_of(&qa.path("keyless.bin")), (OnDisk::Wholly, false, true), "and the file left as it is");
    assert!(tick.contains(&("qa-missing".to_string(), Emptied::Missing)) && kept_on_tick, "kept at a tick");
    assert!(start.contains(&("qa-missing".to_string(), Emptied::Missing)), "{start:?}");
    assert_eq!(db.poison("qa-missing").unwrap(), None, "missing at the first sweep of a process: cleared");
}

/// Self-review (desktop PR 224): a database error while the sweep reads a poisoned file's row is not
/// "the row is gone". The poison stays (I5), as the fetch check fails closed on the same error.
#[test]
fn ld0105_s6_native_an_unreadable_row_keeps_its_poison() {
    use crate::windows_cf::poison::sweep_poisoned;
    let state = tempfile::tempdir().unwrap();
    let db = qa_db(state.path(), &[("qa-unreadable", "unreadable.bin")]);
    db.record_poison("qa-unreadable", "corrupt", Some((1, 2)), 1).unwrap();
    db.make_file_rows_unreadable_for_test();
    assert!(db.get_file("qa-unreadable").is_err(), "the row read fails");
    let outcomes = sweep_poisoned(&db, state.path(), crate::windows_cf::poison::SweepKind::Tick);
    println!("ld0105-s6-native: unreadable row: outcomes {outcomes:?}");
    assert!(outcomes.is_empty(), "nothing is emptied or cleared: {outcomes:?}");
    assert!(db.poison("qa-unreadable").unwrap().is_some(), "the poison stays");
}

// ── Desktop #225: the independent review's P2s, natively ────────────────────────────────────────

/// Review P2 (poison.rs:429): a pin the empty step can put back neither through its handle nor by path
/// is recorded apart from the poison, so the successful empty that clears the poison does not drop it,
/// and the next sweep puts it back. Red on the mutant "the lost pin is not recorded" (the pin is gone).
#[test]
fn ld0105_p2_native_a_pin_lost_both_ways_outlives_the_empty_and_the_next_sweep_restores_it() {
    use crate::windows_cf::poison::{Emptied, FAIL_PIN_RESTORE_BY_HANDLE, FAIL_PIN_RESTORE_BY_PATH, SweepKind, sweep_poisoned};
    let mut qa = QaRoot::new(crate::windows_cf::HYDRATION_POLICY);
    qa.connect(refuse_every_fetch);
    let key = poisoned_whole_pinned(&qa, "lost-pin.bin");
    let state = tempfile::tempdir().unwrap();
    let db = qa_db(state.path(), &[("qa-lost-pin", "lost-pin.bin")]);
    db.record_poison("qa-lost-pin", "corrupt", key, 1).unwrap();
    FAIL_PIN_RESTORE_BY_HANDLE.set(true);
    FAIL_PIN_RESTORE_BY_PATH.set(true);
    let first = sweep_poisoned(&db, &qa.root, SweepKind::Tick);
    FAIL_PIN_RESTORE_BY_HANDLE.set(false);
    FAIL_PIN_RESTORE_BY_PATH.set(false);
    let lost = (census_of(&qa.path("lost-pin.bin")), db.poison("qa-lost-pin").unwrap(), db.pin_to_restore("qa-lost-pin").unwrap());
    let second = sweep_poisoned(&db, &qa.root, SweepKind::Tick);
    let restored = census_of(&qa.path("lost-pin.bin"));
    println!("ld0105-p2-native: lost pin: first {first:?} then {lost:?}; second {second:?}, census {restored:?}");
    assert_eq!(first, vec![("qa-lost-pin".to_string(), Emptied::Emptied)]);
    assert_eq!(lost.0, (OnDisk::Nothing, true, false), "emptied, the pin not back yet");
    assert_eq!(lost.1, None, "the poison is cleared");
    assert_eq!(lost.2, Some(CF_PIN_STATE_PINNED.0), "but the pin is kept to restore");
    assert_eq!(restored.2, true, "the next sweep pinned it again");
    assert_eq!(db.pin_to_restore("qa-lost-pin").unwrap(), None, "and the record ends");
}

/// Review P2 (cf_range.rs:125): the promotion's ONDISK check is the on-disk range query through an
/// attribute-only handle (nothing fetched). Whole on disk: yes; partly or cloud-only: no. Red on the
/// mutant "trust sent" (`on_disk_whole` returning `true`).
#[test]
fn ld0105_p2_native_the_promotion_reads_ondisk_through_an_attribute_handle() {
    let mut qa = QaRoot::new(crate::windows_cf::HYDRATION_POLICY);
    qa.connect(serve_first_64k);
    let names = mint_five(&qa);
    let size = 1u64 << 20;
    let seen: Vec<(String, bool)> = names.iter().map(|n| (n.to_string(), crate::windows_cf::streaming::on_disk_whole(&qa.path(n), size))).collect();
    println!("ld0105-p2-native: on_disk_whole {seen:?}, census {:?}", census(&qa, &names));
    let whole = |n: &str| seen.iter().find(|(name, _)| name == n).unwrap().1;
    assert!(whole("local.bin") && whole("local-pinned.bin"), "wholly on disk: {seen:?}");
    assert!(!whole("partial.bin"), "64 KiB of 1 MiB: {seen:?}");
    assert!(!whole("cloud.bin"), "nothing on disk: {seen:?}");
    assert_eq!(census_of(&qa.path("cloud.bin")).0, OnDisk::Nothing, "and the query fetched nothing");
}

/// Review P2 (poison.rs:538): a write made during the session (after the session's key, before the
/// failure) is "needs attention": the poison records no key, and the sweep keeps the file as it is.
/// Red on the mutant "the key read at the failure" (the sweep empties the person's write).
#[test]
fn ld0105_p2_native_a_write_during_the_session_is_kept_as_needs_attention() {
    use crate::windows_cf::poison::{Emptied, SweepKind, content_key, poison_now, sweep_poisoned};
    let mut qa = QaRoot::new(crate::windows_cf::HYDRATION_POLICY);
    qa.connect(refuse_every_fetch);
    let _ = poisoned_whole_pinned(&qa, "written.bin");
    let path = qa.path("written.bin");
    let key_before = content_key(&path);
    std::thread::sleep(Duration::from_millis(50));
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, b"mine"))
        .unwrap();
    let state = tempfile::tempdir().unwrap();
    let db = qa_db(state.path(), &[("qa-written", "written.bin")]);
    poison_now(&db, "qa-written", &path, "corrupt", key_before);
    let recorded = db.poison("qa-written").unwrap().unwrap().key;
    let outcome = sweep_poisoned(&db, &qa.root, SweepKind::Close);
    println!("ld0105-p2-native: write during the session: key {key_before:?} -> recorded {recorded:?}, sweep {outcome:?}");
    assert_eq!(recorded, None, "the key moved during the session: none recorded");
    assert_eq!(outcome, vec![("qa-written".to_string(), Emptied::NeedsAttention)]);
    assert_eq!(census_of(&path).0, OnDisk::Wholly, "the person's write is kept");
}

/// The data-loss repro's provider: until it is told to serve everything, it leaves a partly hydrated
/// placeholder (64 KiB of each request); then it serves every request whole, as a session serving the
/// writer's fetch of the missing range does.
static T10_SERVE_ALL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

unsafe extern "system" fn t10_fetch(info: *const CF_CALLBACK_INFO, params: *const CF_CALLBACK_PARAMETERS) {
    if !T10_SERVE_ALL.load(std::sync::atomic::Ordering::SeqCst) {
        // SAFETY: forwarded as received.
        return unsafe { serve_first_64k(info, params) };
    }
    if info.is_null() || params.is_null() {
        return;
    }
    // SAFETY: valid for the duration of the callback.
    let r = unsafe { request_of(info, params) };
    let data = content(r.file_size as usize);
    // SAFETY: the keys of this live request.
    unsafe {
        let _ = transfer_range(r.connection, r.transfer, r.request, &data, || false, r.offset, r.length);
    }
}

/// P1 #1 from the native QA rung (desktop #225, lead ruling 2026-11-06), the data-loss repro: a
/// partly downloaded file; another process writes 21 B at offset 0 (a FileStream, share ReadWrite);
/// Windows fetches the missing range first, the provider serves it, the write lands and moves
/// LastWriteTime; then that session fails after a write. Its poison keeps the key the session read
/// before ITS first transfer, so the sweep sees the write: needs attention, the person's bytes kept.
/// Red on the mutant "the key read at the failure" (7188578's `poison_now`: the sweep empties the write).
#[test]
fn ld0105_p1_native_a_write_into_a_partly_downloaded_file_is_kept() {
    use crate::windows_cf::poison::{Emptied, SweepKind, content_key, poison_now, sweep_poisoned};
    let mut qa = QaRoot::new(crate::windows_cf::HYDRATION_POLICY);
    qa.connect(t10_fetch);
    assert!(placeholders::create_placeholder(&qa.root, "qa-t10", "t10.bin", 1 << 20, 1_700_000_000, false).unwrap());
    let path = qa.path("t10.bin");
    let mut one = [0u8; 1];
    let _ = std::fs::File::open(&path).and_then(|mut f| std::io::Read::read(&mut f, &mut one));
    let partial = census_of(&path);
    assert_eq!(partial.0, OnDisk::Partly, "a partly downloaded file: {partial:?}");
    // The session that later poisons: its key before its first transfer.
    let key_before = content_key(&path);
    T10_SERVE_ALL.store(true, std::sync::atomic::Ordering::SeqCst);
    let script = format!(
        "$f=[IO.File]::Open('{}','Open','ReadWrite','ReadWrite'); $f.Write([byte[]](1..21),0,21); $f.Close()",
        path.display()
    );
    let wrote = std::process::Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", &script]).status();
    let written = census_of(&path);
    let key_after = content_key(&path);
    let state = tempfile::tempdir().unwrap();
    let db = qa_db(state.path(), &[("qa-t10", "t10.bin")]);
    poison_now(&db, "qa-t10", &path, "transfer", key_before);
    let recorded = db.poison("qa-t10").unwrap().unwrap().key;
    let outcome = sweep_poisoned(&db, &qa.root, SweepKind::Tick);
    let mut head = [0u8; 21];
    let read = std::fs::File::open(&path).and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head));
    println!(
        "ld0105-p1-native: write into a partial file: {wrote:?}, census {partial:?} -> {written:?}, key {key_before:?} -> {key_after:?}, recorded {recorded:?}, sweep {outcome:?}, census {:?}, head {read:?} {:?}",
        census_of(&path),
        &head[..4]
    );
    assert!(wrote.is_ok_and(|s| s.success()), "the other process wrote");
    assert_ne!(key_after, key_before, "the write moved the content key");
    assert_eq!(recorded, None, "the poison keeps no key: the bytes are not only the attempt's");
    assert_eq!(outcome, vec![("qa-t10".to_string(), Emptied::NeedsAttention)]);
    assert_ne!(census_of(&path).0, OnDisk::Nothing, "never emptied");
    assert_eq!(head.to_vec(), (1u8..=21).collect::<Vec<_>>(), "the person's write is still there");
}

/// Desktop #225 into #226's UI, end to end (lead 2026-11-06): a poisoned file the person changed is a
/// needs-attention row of `get_active_transfers` (the board plus state.db); "Discard and download again"
/// (the command's `discard_now`) empties and refetches it, and the row is gone. Red on the mutant
/// "the board alone" (`snapshot_with_attention` returning `snapshot_dtos`): no row to act on.
#[test]
fn ld0105_u1_native_a_changed_file_is_a_board_row_until_discard_and_download_again() {
    use crate::transfer_progress::{ACTION_DISCARD, ATTENTION_CHANGED, TransferBoard};
    use crate::windows_cf::poison::{Emptied, SweepKind, discard_now, sweep_poisoned};
    let mut qa = QaRoot::new(crate::windows_cf::HYDRATION_POLICY);
    qa.connect(refuse_every_fetch);
    let key = poisoned_whole_pinned(&qa, "board.bin");
    let state = tempfile::tempdir().unwrap();
    let db = qa_db(state.path(), &[("qa-board", "board.bin")]);
    db.record_poison("qa-board", "corrupt", key, 1).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    std::fs::OpenOptions::new()
        .write(true)
        .open(qa.path("board.bin"))
        .and_then(|mut f| std::io::Write::write_all(&mut f, b"mine"))
        .unwrap();
    assert_eq!(sweep_poisoned(&db, &qa.root, SweepKind::Close), vec![("qa-board".to_string(), Emptied::NeedsAttention)]);
    let board = TransferBoard::new();
    let rows = |db: &crate::state_db::StateDb| board.snapshot_with_attention(Some(db));
    let before = rows(&db);
    let result = discard_now(&db, &qa.root, "qa-board", |_| {});
    let after = rows(&db);
    println!("ld0105-u1-native: rows before {before:?}, discard {result:?}, rows after {after:?}");
    assert_eq!(before.len(), 1, "the changed file is a row");
    assert_eq!((before[0].file_id.as_str(), before[0].name.as_str()), ("qa-board", "board.bin"));
    assert_eq!((before[0].attention, before[0].action), (Some(ATTENTION_CHANGED), Some(ACTION_DISCARD)));
    assert_eq!(result, Ok(()));
    assert!(after.is_empty(), "the discard ends the row: {after:?}");
}

/// F2b (desktop #225 QA rung, robocopy ERROR 379): the diagnosis matrix and the fix's rung.
#[path = "ld0105_f2b_native_tests.rs"]
mod f2b;
