    use super::*;

    #[tokio::test]
    async fn the_unsent_gate_is_macos_only() {
        // A state dir and a config dir of its own (as `sign_out_keep_tests` runs its sign-outs): the rename queued here
        // is never seen by another test's sign-out, and if a sign-out refuses, the row stays only here.
        let dir = tempfile::tempdir().unwrap();
        let (state_dir, config_dir) = (dir.path().join("state"), dir.path().join("config"));
        std::fs::create_dir_all(&state_dir).unwrap();
        std::fs::create_dir_all(&config_dir).unwrap();
        let body = async {
            let dir = state_paths::beebeeb_state_dir().unwrap();
            let db = state_db::StateDb::open(state_paths::state_db_path_from_state_dir(&dir)).unwrap();
            db.hold_lock_for_test()
                .execute(
                    "INSERT INTO operation_queue (op_id, kind, file_id) VALUES ('r1', 'rename_file', 'f')",
                    [],
                )
                .unwrap();
            let state = AppState::default();
            synthesize_single_account(&state, AccountId("signout-1887-elsewhere".to_string()));
            set_auth_present(&state, false);
            let result = clear_session_impl(&state, false, None, crate::UnknownFilesArg::Keep, None).await;
            #[cfg(target_os = "linux")]
            {
                assert!(result.is_ok(), "Linux purges as before: {result:?}");
                assert!(db.waiting_rows().unwrap().is_empty());
            }
            #[cfg(target_os = "windows")]
            {
                assert!(
                    !format!("{result:?}").contains("unsent_changes"),
                    "Windows keeps its own preflight"
                );
            }
        };
        state_paths::STATE_DIR_OVERRIDE
            .scope(state_dir, crate::config::CONFIG_DIR_OVERRIDE.scope(config_dir, body))
            .await;
    }
