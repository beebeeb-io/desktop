//! `callbacks.rs`'s unit and probe tests, moved out of it unchanged (LD-0105: the file-size ratchet).
//! `super` is still `crate::windows_cf::callbacks`.

    use super::*;

    /// Sizes agree → transfer the bytes as-is (the steady-state path).
    #[test]
    fn size_match_transfers() {
        assert_eq!(decide_size_action(20, 20), SizeDecision::Transfer);
        assert_eq!(decide_size_action(0, 0), SizeDecision::Transfer);
    }

    /// The exact production bad-row shape: a single-chunk file whose recorded
    /// placeholder size is the ENCRYPTED size (plaintext + 28 = GCM nonce(12) +
    /// tag(16)). The decrypted length (20) is authenticated ground truth, so the
    /// resolution path must be chosen and the corrected size must be the
    /// decrypted plaintext length, NOT the recorded 48.
    #[test]
    fn single_chunk_gcm_overhead_resolves_to_plaintext_len() {
        let plaintext_len = 20;
        let recorded = plaintext_len + 28; // 48 — encrypted size stored by the server bug
        assert_eq!(
            decide_size_action(plaintext_len, recorded),
            SizeDecision::ResolveTo {
                corrected_size: plaintext_len
            }
        );
    }

    /// Multi-chunk overhead (28 * chunk_count) resolves the same way: the
    /// corrected size is always the decrypted length regardless of the delta.
    #[test]
    fn multi_chunk_overhead_resolves_to_plaintext_len() {
        let plaintext_len = 8 * 1024 * 1024; // spans 2 chunks at 4 MiB
        let recorded = plaintext_len + 28 * 2;
        assert_eq!(
            decide_size_action(plaintext_len, recorded),
            SizeDecision::ResolveTo {
                corrected_size: plaintext_len
            }
        );
    }

    /// A recorded size SMALLER than the decrypted length (e.g. truncated/stale
    /// placeholder) still resolves to the authenticated decrypted length.
    #[test]
    fn smaller_recorded_size_resolves_to_plaintext_len() {
        assert_eq!(
            decide_size_action(100, 40),
            SizeDecision::ResolveTo { corrected_size: 100 }
        );
    }

    /// The DB-fallback path builder (task 0783 follow-up): a clean relative
    /// path joins onto the sync root with native separators, tolerating a
    /// leading/trailing slash exactly as populate_placeholders does.
    #[test]
    fn safe_join_builds_path_under_root() {
        let root = std::path::Path::new("C:\\Users\\me\\Beebeeb");
        // Leading slash + nested path → joined under the root.
        assert_eq!(
            safe_join_under_root(root, "/docs/report.txt"),
            Some(root.join("docs").join("report.txt"))
        );
        // Bare leaf, no slash.
        assert_eq!(
            safe_join_under_root(root, "bb-test-upload.txt"),
            Some(root.join("bb-test-upload.txt"))
        );
    }

    /// Path-safety guard: empty, root-only, and traversal rel paths are all
    /// rejected so a server-controlled path can never escape the sync root.
    #[test]
    fn safe_join_rejects_unsafe_paths() {
        let root = std::path::Path::new("C:\\Users\\me\\Beebeeb");
        assert_eq!(safe_join_under_root(root, ""), None);
        assert_eq!(safe_join_under_root(root, "/"), None);
        assert_eq!(safe_join_under_root(root, "///"), None);
        // `..` traversal anywhere in the path is refused.
        assert_eq!(safe_join_under_root(root, "../escape.txt"), None);
        assert_eq!(safe_join_under_root(root, "docs/../../escape.txt"), None);
        assert_eq!(safe_join_under_root(root, "docs/./report.txt"), None);
        // Empty interior segment (double slash) is refused.
        assert_eq!(safe_join_under_root(root, "docs//report.txt"), None);
    }

    /// Desktop 0.8.15 (deliverable 2), the hydration storm. 0.8.14's guard opened every placeholder
    /// for reading (as the reparse point); on Guus's PC 5,294 cloud-only files were then fetched
    /// whole, one after another, in the guard's row order, ~42 GB in 2 h, until C: was full. On a
    /// temporary sync root connected with a counting FETCH_DATA callback (which fails every
    /// fetch, so nothing is served), this measures what asks for a cloud-only file's bytes: the
    /// guard's reads (now attribute-only), the stamp pass's promotion check, the 0.8.14
    /// read-access open (for comparison, not asserted) and the guard's mark (now through a handle
    /// with no data access). Run with --nocapture to see the counts and who asked.
    #[test]
    fn desktop_0815_probe_the_guard_reads_a_cloud_only_file_without_fetching_it() {
        use crate::unknown_check::{OnDisk, PlatformView};
        use std::os::windows::ffi::OsStrExt;
        use std::os::windows::fs::OpenOptionsExt;
        use std::sync::Mutex;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use windows::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
        use windows::core::PCWSTR;
        static FETCHES: AtomicUsize = AtomicUsize::new(0);
        static REQUESTERS: Mutex<Vec<(u32, String, i64, i64)>> = Mutex::new(Vec::new());
        unsafe extern "system" fn counting_fetch(info: *const CF_CALLBACK_INFO, params: *const CF_CALLBACK_PARAMETERS) {
            if info.is_null() || params.is_null() {
                return;
            }
            // SAFETY: both are valid for the duration of the callback (the Cloud Files contract).
            let info = unsafe { &*info };
            let fetch = unsafe { (*params).Anonymous.FetchData };
            let (pid, image) = if info.ProcessInfo.is_null() {
                (0, String::new())
            } else {
                // SAFETY: filled because the root is connected with REQUIRE_PROCESS_INFO.
                let process = unsafe { &*info.ProcessInfo };
                let image = wide_to_vec(process.ImagePath.0)
                    .map(|wide| crate::unknown_check::requester_image_name(&wide))
                    .unwrap_or_default();
                (process.ProcessId, image)
            };
            REQUESTERS.lock().unwrap_or_else(|p| p.into_inner()).push((pid, image, fetch.RequiredFileOffset, fetch.RequiredLength));
            FETCHES.fetch_add(1, Ordering::SeqCst);
            // SAFETY: the keys come from this live callback.
            unsafe {
                fail_transfer(info.ConnectionKey, info.TransferKey, info.RequestKey, fetch.RequiredFileOffset, fetch.RequiredLength)
            };
        }
        struct Registered(std::path::PathBuf);
        impl Drop for Registered {
            fn drop(&mut self) {
                let _ = crate::windows_cf::unregister_sync_root(&self.0);
            }
        }
        struct Connected(CF_CONNECTION_KEY);
        impl Drop for Connected {
            fn drop(&mut self) {
                // SAFETY: the key of the connection made below, disconnected once.
                let _ = unsafe { CfDisconnectSyncRoot(self.0) };
            }
        }
        let settle = || std::thread::sleep(std::time::Duration::from_secs(2));
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        std::fs::create_dir(&root).unwrap();
        crate::windows_cf::register_test_sync_root(&root).unwrap();
        let registered = Registered(root.clone());
        let table = [
            CF_CALLBACK_REGISTRATION { Type: CF_CALLBACK_TYPE_FETCH_DATA, Callback: Some(counting_fetch) },
            CF_CALLBACK_REGISTRATION { Type: CF_CALLBACK_TYPE_NONE, Callback: None },
        ];
        let root_wide: Vec<u16> = root.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        // SAFETY: the path and the table outlive the call; the callback is a static function.
        let key = unsafe {
            CfConnectSyncRoot(PCWSTR(root_wide.as_ptr()), table.as_ptr(), None, CF_CONNECT_FLAG_REQUIRE_PROCESS_INFO)
        }
        .unwrap();
        let connected = Connected(key);
        assert!(
            crate::windows_cf::placeholders::create_placeholder(&root, "probe-cloud-only", "cloud-only.bin", 8 << 20, 1_700_000_000, false)
                .unwrap()
        );
        let cloud_only = root.join("cloud-only.bin");
        let local = root.join("local.bin");
        std::fs::write(&local, b"bytes on this PC").unwrap();
        crate::windows_cf::placeholders::convert_to_in_sync_placeholder(&local, "probe-local").unwrap();
        let db = crate::state_db::StateDb::open(temp.path().join("state.db")).unwrap();
        settle(); // anything that reacts to a new file has done so
        let before = FETCHES.load(Ordering::SeqCst);
        let view = crate::windows_cf::placeholders::platform_view(&cloud_only);
        let in_sync = crate::windows_cf::placeholders::placeholder_in_sync(&cloud_only);
        let entry = crate::windows_cf::placeholders::placeholder_local_entry_trusting(&cloud_only, true);
        let local_view = crate::windows_cf::placeholders::platform_view(&local);
        settle();
        let after_guard_reads = FETCHES.load(Ordering::SeqCst);
        // The stamp pass (which now runs once the guard is done) offers every in-sync file with no
        // agreed content to the range-fetch promotion: a cloud-only one must not be fetched.
        let promoted = crate::windows_cf::placeholders::promote_fetched_content(&db, "probe-cloud-only", &cloud_only).unwrap();
        settle();
        let after_promotion = FETCHES.load(Ordering::SeqCst);
        // The 0.8.14 open, for comparison: read access, as the reparse point.
        drop(
            std::fs::OpenOptions::new()
                .read(true)
                .share_mode(7)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
                .open(&cloud_only)
                .unwrap(),
        );
        settle();
        let after_0814_open = FETCHES.load(Ordering::SeqCst);
        // The guard's mark. It is made only on files that hold bytes, a partly downloaded one
        // included, so it must not ask for the rest: measured here on a cloud-only file.
        let marked = crate::windows_cf::placeholders::mark_not_in_sync_path(&cloud_only).is_ok();
        settle();
        let after_mark = FETCHES.load(Ordering::SeqCst);
        let marked_view = crate::windows_cf::placeholders::platform_view(&cloud_only);
        // And on a file whose bytes are all on this PC, the case the guard meets most.
        let local_marked = crate::windows_cf::placeholders::mark_not_in_sync_path(&local).is_ok();
        let local_marked_view = crate::windows_cf::placeholders::platform_view(&local);
        println!(
            "desktop-0815 probe: fetches before={before} after_guard_reads={after_guard_reads} \
             after_promotion={after_promotion} after_0814_read_open={after_0814_open} after_mark={after_mark} (marked={marked})"
        );
        for (pid, image, offset, length) in REQUESTERS.lock().unwrap_or_else(|p| p.into_inner()).iter() {
            println!(
                "desktop-0815 probe: fetch asked by pid={pid} (this test is {}) image={image} offset={offset} length={length}",
                std::process::id()
            );
        }
        drop(connected);
        drop(registered);
        assert_eq!(
            view,
            PlatformView::Placeholder { on_disk: OnDisk::Nothing, in_sync: true, identity: Some("probe-cloud-only".into()) },
            "read with attribute access only"
        );
        assert_eq!(in_sync, Some(true));
        assert_eq!(entry, crate::engine_bridge::LocalEntry::InSync);
        assert_eq!(
            local_view,
            PlatformView::Placeholder { on_disk: OnDisk::Wholly, in_sync: true, identity: Some("probe-local".into()) }
        );
        assert_eq!(after_guard_reads, before, "the guard's reads asked for no bytes");
        assert!(!promoted);
        assert_eq!(after_promotion, after_guard_reads, "the promotion check asked for no bytes");
        assert!(marked);
        assert_eq!(
            marked_view,
            PlatformView::Placeholder { on_disk: OnDisk::Nothing, in_sync: false, identity: Some("probe-cloud-only".into()) },
            "marked not in sync"
        );
        assert_eq!(after_mark, after_0814_open, "the guard's mark asked for no bytes");
        assert!(local_marked);
        assert_eq!(
            local_marked_view,
            PlatformView::Placeholder { on_disk: OnDisk::Wholly, in_sync: false, identity: Some("probe-local".into()) },
            "a local file marked not in sync"
        );
    }
