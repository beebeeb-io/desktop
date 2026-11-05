    #[test]
    fn the_popover_command_state_and_handler_are_registered() {
        // Structural, like the other wiring tests here: `run()` needs a real window system, so
        // the registration is asserted on the source. An unmanaged `PopoverRuntime` would make
        // the command (and the runner's progress board) fail at run time, not at compile time.
        let full = crate::root_source::lib_rs().replace("\r\n", "\n");
        // Everything before this test module: the mock-app helpers below also call `manage`.
        let source = &full[..full.find("mod popover_snapshot_command_tests;").unwrap()];
        assert_eq!(
            source
                .matches(".manage(popover_data::PopoverRuntime::default())")
                .count(),
            1
        );
        assert_eq!(
            source.matches("            popover_snapshot,\n").count(),
            1,
            "in generate_handler!"
        );
        // `sync_status` reads the mirrored state through the shared mapping.
        let start = source.find("async fn sync_status(").expect("sync_status exists");
        let body = &source[start..start + source[start..].find("\n}\n").unwrap()];
        assert!(body.contains("engine_status::sync_status_engine(raw.as_str())"));
        // The storage cache is keyed by the vault epoch, and the epoch moves at every place the
        // session is installed (2: sign-in, recovery-phrase unlock) or cleared. Lock and Sign-out clear it in
        // ONE function (`clear_session_holding_slot`, fix round 1 of Task 10), which bumps in its normal and
        // its poisoned-mutex branch; the sign-out's last turn (Task 12 fix round 1) bumps when it clears a session
        // that was put in memory while the sign-out ran; and a re-sign-in whose kept key the server no longer accepts
        // bumps when it ended a session in memory (spec §5.6).
        assert_eq!(source.matches("bump_vault_epoch();\n").count(), 6);
        // The fetch is bound to the session (#139 review P1; tests/windows_session_wiring.rs pins the binding).
        assert!(source.contains(
            "VAULT_EPOCH.load(std::sync::atomic::Ordering::SeqCst),\n                        || session_bound(async move { fetch_storage_usage"
        ));
        // Spec 2026-10-06: the snapshot reads the reconciler's view and never probes the OS.
        let snapshot = {
            let at = source.find("async fn popover_snapshot(").unwrap();
            &source[at..at + source[at..].find("\n}\n").unwrap()]
        };
        // #139 review: the summary comes from the account's cache on `AppState`, which sign-out clears (P2-3), and its
        // wait runs beside the state DB read, not before it (P3-10).
        assert!(snapshot.contains("cached_storage(\n                        &state.popover_storage,"));
        assert!(snapshot.contains("let (storage, db_view) = tokio::join!(storage, db_view);"));
        assert!(snapshot.contains("state.finder_setup.get()"));
        assert!(
            !snapshot.contains("finder_location_state")
                && !snapshot.contains("spawn_blocking(popover_finder_probe_state)")
        );
    }
