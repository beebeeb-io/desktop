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
use crate::windows_cf::fetch_progress::{fail_transfer_for_tests, keep_fetch_alive, transfer_range_for_tests};
use crate::windows_cf::placeholders;

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

    fn connect(&mut self, fetch: unsafe extern "system" fn(*const CF_CALLBACK_INFO, *const CF_CALLBACK_PARAMETERS)) {
        let table = [
            CF_CALLBACK_REGISTRATION {
                Type: CF_CALLBACK_TYPE_FETCH_DATA,
                Callback: Some(fetch),
            },
            CF_CALLBACK_REGISTRATION {
                Type: CF_CALLBACK_TYPE_NONE,
                Callback: None,
            },
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
        let _ = transfer_range_for_tests(r.connection, r.transfer, r.request, &data, r.offset, served);
        if served < r.length {
            fail_transfer_for_tests(
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
    if unsafe { transfer_range_for_tests(r.connection, r.transfer, r.request, &data, r.offset, r.length) }.is_err() {
        unsafe { fail_transfer_for_tests(r.connection, r.transfer, r.request, r.offset, r.length) };
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
    let kinds: Vec<(OnDisk, bool)> = before.iter().map(|(_, d, _, p)| (*d, *p)).collect();
    assert_eq!(
        kinds,
        vec![
            (OnDisk::Nothing, false),
            (OnDisk::Nothing, true),
            (OnDisk::Partly, false),
            (OnDisk::Wholly, false),
            (OnDisk::Wholly, true)
        ],
        "the five kinds are there before the upgrade"
    );
    // The 0.8.22 engine start: the same registration call the app makes, with UPDATE, no unregister.
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
    // Not asserted: this is the measurement that says why nothing in the app may do it.
    let unregistered = StorageProviderSyncRootManager::Unregister(&HSTRING::from(id.as_str())).is_ok();
    let left: Vec<(String, bool)> = names.iter().map(|n| (n.to_string(), qa.path(n).exists())).collect();
    let views: Vec<String> = names
        .iter()
        .map(|n| format!("{n}={:?}", placeholders::platform_view(&qa.path(n))))
        .collect();
    println!("ld0105-w1-native: D-6 record, WinRT Unregister ok={unregistered}: present {left:?}");
    println!("ld0105-w1-native: D-6 record, views {views:?}");
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

/// Poison `name` by hand, as a failed fetch would: not in sync, then the stamp, then the record.
fn poison(db: &crate::state_db::StateDb, qa: &QaRoot, id: &str, name: &str) {
    placeholders::mark_not_in_sync_path(&qa.path(name)).unwrap();
    let stamp = crate::windows_cf::poison::poison_stamp(&qa.path(name));
    assert!(stamp.is_some(), "the stamp of {name} is readable");
    db.record_poison(id, "corrupt", stamp, 1).unwrap();
}

#[test]
fn ld0105_s6_native_the_empty_step_dehydrates_a_poisoned_placeholder_in_the_measured_order() {
    use crate::windows_cf::poison::{Emptied, empty_poisoned, sweep_poisoned};
    let mut qa = QaRoot::new(crate::windows_cf::HYDRATION_POLICY);
    qa.connect(serve_first_64k); // this process is the connected provider: only it may set IN_SYNC
    let names = mint_five(&qa);
    assert_eq!(names.len(), 5);
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
            empty_poisoned(&qa.path("local-pinned.bin"), stamp.change_ticks.zip(stamp.file_index)),
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
    let emptied = sweep_poisoned(&db, &qa.root);
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
    assert!(
        db.poison("qa-local").unwrap().is_some(),
        "the changed file keeps its poison"
    );
    // A poisoned file refuses every fetch until it is emptied (I5) is the fetch callback's first check;
    // the emptied ones fetch again: a read of the partial file asks for bytes once more.
    let mut one = [0u8; 1];
    let reread = std::fs::File::open(qa.path("partial.bin")).and_then(|mut f| std::io::Read::read(&mut f, &mut one));
    assert!(reread.is_ok(), "an emptied placeholder fetches again: {reread:?}");
}
