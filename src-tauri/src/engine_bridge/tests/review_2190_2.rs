//! `review_2190_2` engine tests, split out of `engine_bridge.rs`.

use super::*;
    /// Spec D11, D11b, D12 and I6 (task 1888): off macOS the engine resumes the operations paused for `auth` itself,
    /// after its reconcile pass and the key check. Fixtures are made up: no real name, path or person.
    mod reconcile_then_resume_tests {
        use super::*;
        use crate::runner::ResumeTicket;
        use std::sync::atomic::Ordering::SeqCst;

        const KEY: [u8; 32] = [7u8; 32];
        const OTHER_KEY: [u8; 32] = [9u8; 32];

        struct Pc {
            dir: tempfile::TempDir,
            root: PathBuf,
            bridge: EngineBridge,
            /// The ticket the bridge's queue reads, as the runner shares it.
            #[cfg_attr(target_os = "macos", allow(dead_code))]
            resume: ResumeTicket,
        }

        impl Pc {
            fn new() -> Self {
                let dir = tempfile::tempdir().unwrap();
                let root = dir.path().join("Beebeeb");
                std::fs::create_dir_all(&root).unwrap();
                let resume = ResumeTicket::default();
                let bridge = test_bridge_with_api(&dir.path().join("state.db"), "https://api.example".into(), KEY)
                    .with_resume(resume.clone());
                Self { dir, root, bridge, resume }
            }

            /// A due change that needs no network (`PinTree`): it shows whether the queue sends.
            #[cfg(not(target_os = "macos"))]
            fn due_pin(&self, op_id: &str) {
                self.sql(&format!(
                    "INSERT INTO operation_queue (op_id, kind, file_id, metadata_json, attempts, max_attempts, next_retry_at,
                                                  created_at, updated_at)
                     VALUES ('{op_id}', 'pin_tree', 'pin-{op_id}', '{{\"operation\":\"pin_tree\",\"pinned\":true}}', 0, 5, 0,
                             100, 100)"
                ));
            }

            fn sql(&self, sql: &str) {
                self.bridge.db().test_execute(sql).unwrap();
            }

            /// A row: `version` > 0 is server-known (the snapshot's shape).
            fn row(&self, id: &str, parent: Option<&str>, path: &str, folder: bool, version: i64) {
                let parent = parent.map_or("NULL".to_string(), |p| format!("'{p}'"));
                let (status, kind) = (if version > 0 { "synced" } else { "local" }, if folder { "folder" } else { "file" });
                self.sql(&format!(
                    "INSERT INTO files (file_id, path, status, item_kind, current_version, parent_id, size_bytes, remote_updated_at)
                     VALUES ('{id}', '{path}', '{status}', '{kind}', {version}, {parent}, 0, 0)"
                ));
            }

            /// An op paused for `auth` as a refused session pauses it (`record_pause_claimed`).
            fn op(&self, op_id: &str, kind: &str, file_id: &str, parent: Option<&str>, metadata: serde_json::Value) {
                let parent = parent.map_or("NULL".to_string(), |p| format!("'{p}'"));
                let payload = self.dir.path().join(format!("staged-{op_id}"));
                std::fs::write(&payload, op_id.as_bytes()).unwrap();
                let metadata = metadata.to_string().replace('\'', "''");
                self.sql(&format!(
                    "INSERT INTO operation_queue (op_id, kind, file_id, parent_id, metadata_json, payload_path, attempts,
                                                  max_attempts, next_retry_at, paused_reason, last_error_class, created_at, updated_at)
                     VALUES ('{op_id}', '{kind}', '{file_id}', {parent}, '{metadata}', '{}', 0, 25, 0, 'auth', 'auth', 1, 1)",
                    payload.to_string_lossy()
                ));
            }

            /// A new file's create, its name sealed under `key` for `sealed_for`.
            fn new_file(&self, op_id: &str, file_id: &str, parent: Option<&str>, path: &str, key: [u8; 32], sealed_for: &str) {
                self.row(file_id, parent, path, false, 0);
                let name = path.rsplit('/').next().unwrap();
                self.op(
                    op_id,
                    "upload_version",
                    file_id,
                    parent,
                    json!({
                        "operation": "create_file",
                        "name_encrypted": encrypted_metadata_for_name(&key, sealed_for, name, None).unwrap(),
                        "content_type": null,
                        "uploaded_by": "authenticated_desktop_user",
                    }),
                );
            }

            /// The server folder `t`, the ghost `g` made here at the same path (its create paused for `auth`), and a
            /// new file `f` under the ghost whose create is paused for `auth` too.
            fn ghost_with_twin(&self) {
                self.row("t", None, "Photos", true, 3);
                self.row("g", None, "Photos", true, 0);
                self.op(
                    "op-g",
                    "create_folder",
                    "g",
                    None,
                    json!({
                        "operation": "create_folder",
                        "name_encrypted": encrypted_metadata_for_name(&KEY, "g", "Photos", None).unwrap(),
                    }),
                );
                self.new_file("op-f", "f", Some("g"), "Photos/a.jpg", KEY, "f");
            }

            fn pause(&self, op_id: &str, reason: &str) {
                self.sql(&format!("UPDATE operation_queue SET paused_reason = '{reason}' WHERE op_id = '{op_id}'"));
            }

            fn query<T: rusqlite::types::FromSql>(&self, sql: &str) -> Option<T> {
                use rusqlite::OptionalExtension as _;
                self.bridge.db().hold_lock_for_test().query_row(sql, [], |row| row.get(0)).optional().unwrap()
            }

            /// The op's pause reason; `None` when it is due (or gone: see [`Self::has_op`]).
            fn paused(&self, op_id: &str) -> Option<String> {
                self.query::<Option<String>>(&format!("SELECT paused_reason FROM operation_queue WHERE op_id = '{op_id}'"))
                    .flatten()
            }

            fn has_op(&self, op_id: &str) -> bool {
                self.query::<String>(&format!("SELECT op_id FROM operation_queue WHERE op_id = '{op_id}'")).is_some()
            }

            fn has_row(&self, file_id: &str) -> bool {
                self.query::<String>(&format!("SELECT file_id FROM files WHERE file_id = '{file_id}'")).is_some()
            }

            fn op_parent(&self, op_id: &str) -> Option<String> {
                self.query::<Option<String>>(&format!("SELECT parent_id FROM operation_queue WHERE op_id = '{op_id}'"))
                    .flatten()
            }

            async fn tick(&self, resume: &ResumeTicket, reconciled: &mut bool) {
                self.bridge.reconcile_then_resume(&self.root, resume, reconciled).await.unwrap();
            }
        }

        /// D11, I6: a later tick resumes only when a sign-in asked, and then after the pass: the ghost is folded into
        /// its twin first, so its child resumes under the server folder. An ask is served once.
        #[tokio::test]
        async fn a_resume_runs_after_the_pass_and_only_when_asked() {
            let pc = Pc::new();
            pc.ghost_with_twin();
            let resume = ResumeTicket::default();
            // This run's first pass already ran: a later tick.
            let mut reconciled = true;
            pc.tick(&resume, &mut reconciled).await;
            assert_eq!(pc.paused("op-f").as_deref(), Some("auth"), "asked == done: the auth op stays paused");
            assert!(pc.has_op("op-g") && pc.has_row("g"), "and no pass ran");

            resume.asked.store(1, SeqCst);
            pc.tick(&resume, &mut reconciled).await;
            assert!(!pc.has_op("op-g") && !pc.has_row("g"), "the pass ran first: the ghost was remapped");
            assert_eq!(pc.op_parent("op-f").as_deref(), Some("t"), "its child now waits under the server folder");
            assert_eq!(pc.paused("op-f"), None, "then the remaining auth op was resumed");
            assert_eq!(resume.done.load(SeqCst), 1, "done == asked");

            // The server refuses the session again; with no new ask, nothing is resumed.
            pc.pause("op-f", "auth");
            pc.tick(&resume, &mut reconciled).await;
            assert_eq!(pc.paused("op-f").as_deref(), Some("auth"), "a second call with no new ask resumes nothing");
            assert_eq!(resume.done.load(SeqCst), 1);
        }

        /// Review P2 (task 1888): "Upload to my Beebeeb" releases a held ghost folder's create and asks for a pass. Until
        /// a committed pass has served that ask, the queue sends nothing (at a tick or a send-phase wake: both go
        /// through `process_due_operations`). The pass then sees the released create and folds the ghost into its twin,
        /// so it is never sent; and a release resumes no `auth` op, because it is not a sign-in.
        /// Not on macOS: the queue's pass-owed gate is off there (engine_bridge.rs `process_due_operations`), and macOS
        /// has no "Upload to my Beebeeb" release.
        #[cfg(not(target_os = "macos"))]
        #[tokio::test]
        async fn a_released_change_is_not_sent_before_a_pass_has_seen_it() {
            let pc = Pc::new();
            pc.ghost_with_twin();
            pc.pause("op-g", "found_while_paused");
            pc.due_pin("op-pin");
            // This run's first pass already ran; then the person pressed Upload on the held create.
            let mut reconciled = true;
            pc.resume.pass_wanted.fetch_add(1, SeqCst);
            assert_eq!(pc.bridge.db().release_found_while_paused(&["op-g".to_string()], 200).unwrap(), 1);

            let outcome = pc.bridge.process_due_operations(&pc.root, 200).await.unwrap();
            assert!(outcome.completed_op_ids.is_empty(), "nothing is sent while a release waits for a pass");
            assert!(pc.has_op("op-pin") && pc.has_op("op-g"));

            pc.tick(&pc.resume, &mut reconciled).await;
            assert!(!pc.has_op("op-g") && !pc.has_row("g"), "the pass saw the released create: the ghost was remapped");
            assert_eq!(pc.op_parent("op-f").as_deref(), Some("t"));
            assert_eq!(pc.paused("op-f").as_deref(), Some("auth"), "a release resumes no auth op");
            assert_eq!(pc.resume.pass_served.load(SeqCst), 1, "the release ask is served");
            assert_eq!(pc.resume.done.load(SeqCst), 0, "and no sign-in ask was");

            let outcome = pc.bridge.process_due_operations(&pc.root, 200).await.unwrap();
            assert_eq!(outcome.completed_op_ids, vec!["op-pin".to_string()], "after the pass the queue sends again");
        }

        /// I6: a pass that fails resumes nothing, serves no ask and records no pass for the run, so the next tick tries
        /// again; once it completes, the resume follows it.
        #[tokio::test]
        async fn nothing_is_resumed_when_the_pass_fails() {
            let pc = Pc::new();
            pc.ghost_with_twin();
            // A test-only trigger: deleting the ghost's row fails, so the pass's transaction rolls back.
            pc.sql(
                "CREATE TEMP TRIGGER stop_here BEFORE DELETE ON files WHEN OLD.file_id = 'g'
                 BEGIN SELECT RAISE(ABORT, 'test'); END",
            );
            let resume = ResumeTicket::default();
            resume.asked.store(1, SeqCst);
            let mut reconciled = false;
            assert!(pc.bridge.reconcile_then_resume(&pc.root, &resume, &mut reconciled).await.is_err());
            assert_eq!(pc.paused("op-f").as_deref(), Some("auth"), "no resume without a committed pass");
            assert!(pc.has_op("op-g"), "the failed transaction left the ghost as it was");
            assert!(!reconciled, "no pass recorded for this run");
            assert_eq!(resume.done.load(SeqCst), 0, "the ask stays open");

            pc.sql("DROP TRIGGER stop_here");
            pc.tick(&resume, &mut reconciled).await;
            assert!(!pc.has_op("op-g"));
            assert_eq!(pc.paused("op-f"), None, "the next tick passes, then resumes");
            assert!(reconciled);
            assert_eq!(resume.done.load(SeqCst), 1);
        }

        /// D11's first-tick rule: the first successful tick of an engine run proves the server accepts this session's
        /// token, so it resumes without a sign-in (a Windows PC that launched offline asks nowhere else), after the
        /// pass. Only once per run.
        #[tokio::test]
        async fn the_first_successful_tick_resumes_without_a_sign_in() {
            let pc = Pc::new();
            pc.ghost_with_twin();
            let resume = ResumeTicket::default();
            let mut reconciled = false;
            pc.tick(&resume, &mut reconciled).await;
            assert!(!pc.has_op("op-g") && !pc.has_row("g"), "the pass ran: the ghost was remapped");
            assert_eq!(pc.op_parent("op-f").as_deref(), Some("t"));
            assert_eq!(pc.paused("op-f"), None, "asked == done == 0, and the run's first pass resumed the auth op");
            assert!(reconciled, "this run's first pass is recorded");
            assert_eq!(resume.done.load(SeqCst), 0, "no ask was open");

            pc.pause("op-f", "auth");
            pc.tick(&resume, &mut reconciled).await;
            assert_eq!(pc.paused("op-f").as_deref(), Some("auth"), "the second call does not resume again");
        }

        /// D12: every `auth` op whose name is sealed by its own id and carries no shared context is test-decrypted
        /// under this session's key. One that does not open holds every `auth` op as `key_replaced`; ops paused for
        /// another reason keep it. All opening: resumed. A shared-context op is not tested; it follows the verdict.
        #[tokio::test]
        async fn a_name_that_does_not_open_holds_every_auth_op_as_key_replaced() {
            // One create sealed under another key.
            let pc = Pc::new();
            pc.new_file("op-mine", "f1", None, "a.txt", KEY, "f1");
            pc.new_file("op-theirs", "f2", None, "b.txt", OTHER_KEY, "f2");
            pc.op("op-trash", "trash_file", "f3", None, json!({}));
            pc.new_file("op-quota", "f4", None, "c.txt", KEY, "f4");
            pc.pause("op-quota", "quota");
            let resume = ResumeTicket::default();
            resume.asked.store(1, SeqCst);
            let mut reconciled = true;
            pc.tick(&resume, &mut reconciled).await;
            for op in ["op-mine", "op-theirs", "op-trash"] {
                assert_eq!(pc.paused(op).as_deref(), Some("key_replaced"), "{op}: every auth op is held");
            }
            assert_eq!(pc.paused("op-quota").as_deref(), Some("quota"), "another pause keeps its reason");
            assert_eq!(resume.done.load(SeqCst), 1, "the ask is served by the hold");

            // A name sealed under this key, but for another file: it does not open by its own id.
            let pc = Pc::new();
            pc.new_file("op-mine", "f1", None, "a.txt", KEY, "f1");
            pc.new_file("op-moved", "f2", None, "b.txt", KEY, "f9");
            let mut reconciled = false;
            pc.tick(&ResumeTicket::default(), &mut reconciled).await;
            assert_eq!(pc.paused("op-mine").as_deref(), Some("key_replaced"));
            assert_eq!(pc.paused("op-moved").as_deref(), Some("key_replaced"), "keyed by its own id only");

            // All sealed under this key; a shared-context op sealed elsewhere is not tested: resumed.
            let pc = Pc::new();
            pc.new_file("op-mine", "f1", None, "a.txt", KEY, "f1");
            pc.new_file("op-too", "f2", None, "b.txt", KEY, "f2");
            pc.op(
                "op-shared",
                "upload_version",
                "f3",
                None,
                json!({
                    "operation": "create_file",
                    "name_encrypted": encrypted_metadata_for_name(&OTHER_KEY, "f3", "c.txt", None).unwrap(),
                    "shared_root_id": "shared-root",
                    "share_id": "invite-1",
                }),
            );
            pc.op("op-trash", "trash_file", "f4", None, json!({}));
            let resume = ResumeTicket::default();
            resume.asked.store(1, SeqCst);
            let mut reconciled = true;
            pc.tick(&resume, &mut reconciled).await;
            for op in ["op-mine", "op-too", "op-shared", "op-trash"] {
                assert_eq!(pc.paused(op), None, "{op}: resumed");
            }
            assert_eq!(resume.done.load(SeqCst), 1);
        }

        /// Spec D5, anti-goal 3: the scaffolding directories a pass dropped are removed deepest first with
        /// `remove_dir`, never recursively, and never one that holds something or lies outside the sync root.
        #[test]
        fn scaffold_directories_are_removed_bottom_up_and_never_when_they_hold_something() {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join("Beebeeb");
            std::fs::create_dir_all(root.join("Backups/unknown/A")).unwrap();
            std::fs::create_dir_all(root.join("Backups/unknown/B")).unwrap();
            std::fs::write(root.join("Backups/unknown/B/keep.txt"), b"x").unwrap();
            std::fs::create_dir_all(dir.path().join("outside")).unwrap();
            remove_scaffold_dirs(
                &root,
                &[
                    "Backups/unknown".to_string(),
                    "Backups/unknown/A".to_string(),
                    "Backups/unknown/B".to_string(),
                    "../outside".to_string(),
                    "Missing/X".to_string(),
                    String::new(),
                ],
            );
            assert!(!root.join("Backups/unknown/A").exists(), "an empty one is removed");
            assert!(root.join("Backups/unknown/B/keep.txt").exists(), "one that holds a file stays, file and all");
            assert!(root.join("Backups/unknown").exists(), "its parent is not empty: it stays");
            assert!(dir.path().join("outside").exists(), "never outside the sync root");
            assert!(root.exists(), "never the sync root itself");

            // Listed top-down, removed bottom-up: both go.
            std::fs::create_dir_all(root.join("Backups/other/C")).unwrap();
            remove_scaffold_dirs(&root, &["Backups/other".to_string(), "Backups/other/C".to_string()]);
            assert!(!root.join("Backups/other").exists(), "the parent goes once its child is gone");
            assert!(root.join("Backups").exists(), "what was not listed stays");
        }
    }
