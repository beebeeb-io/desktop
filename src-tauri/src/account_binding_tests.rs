    use super::finder_setup_command_tests::{body_between, production_source};
    use super::*;
    use crate::account_binding::{Bound, Identity, bind_before_engine_start};

    const RESET_ALLOWED: bool = !cfg!(target_os = "windows");

    fn alice() -> Identity {
        Identity::new(Some("u-a"), Some("a@beebeeb.io"))
    }

    fn bob() -> Identity {
        Identity::new(Some("u-b"), Some("b@beebeeb.io"))
    }

    /// The code of a function without its comment lines.
    fn code_of(production: &str, signature: &str) -> String {
        body_between(production, signature, "\n}\n")
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    // ---- the census: one start, bound first ----

    /// Ruling P: `sync_status` exposes the refusal on every platform (no cfg around it), and the one bound start
    /// records it from its authorization's error and clears it when it starts an engine.
    #[test]
    fn sync_status_exposes_the_engine_refusal_and_the_bound_start_keeps_it_current() {
        let production = production_source();
        let status = body_between(&production, "async fn sync_status(", "\n}\n");
        let field = status
            .find("\"engine_refusal\": engine_refusal_view(&acct),")
            .expect("sync_status exposes it");
        // Inside the one `json!` literal it returns (whose entries cannot carry a cfg), so on every platform.
        let literal = status.rfind("Ok(serde_json::json!({").expect("its one answer");
        assert!(
            literal < field && status[literal..].matches("Ok(serde_json::json!({").count() == 1,
            "on every platform:\n{status}"
        );
        let bound_text = body_between(&production, "fn start_engine_bound(", "\n}\n");
        // Positions are read in the squeezed text, so the order does not depend on how rustfmt wraps a line.
        let bound = crate::source_pin::squeeze(bound_text);
        let at = |snippet: &str| bound.find(&crate::source_pin::squeeze(snippet));
        let turn = at("let turn = acct.session_generation();").expect("the start reads its turn first");
        let recorded =
            at(".inspect_err(|error| record_engine_refusal(acct, turn, error))?").expect("it records a refusal");
        let spawned = at("spawn(paths.sync_root.clone()").unwrap();
        let cleared = at("*acct.engine_refusal.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;")
            .expect("a start clears it");
        assert!(
            turn < recorded && recorded < spawned && spawned < cleared,
            "{bound_text}"
        );
    }

    /// Every engine start binds the local data to the session's account first, and a failed
    /// binding returns before any engine exists. Every start site goes through the one function.
    #[test]
    fn every_engine_start_is_bound_first() {
        let production = production_source();
        let spawn = concat!("EngineRunner::", "spawn(");
        assert_eq!(
            production.matches(spawn).count(),
            1,
            "every engine start goes through spawn_bound_engine"
        );
        let gate = body_between(&production, "fn spawn_bound_engine(", "\n}\n");
        let signature = &gate[..gate.find(") -> ").expect("its return type")];
        for key_material in ["token", "master_key", "[u8; 32]", "String,"] {
            assert!(
                !signature.contains(key_material),
                "the gate takes no key or token ({key_material}):\n{signature}"
            );
        }
        assert!(
            signature.contains("MutexGuard<'_, Option<EngineRunner>>"),
            "it needs the engine slot held:\n{signature}"
        );
        assert!(gate.contains(spawn), "the one spawn is the gate's own constructor");
        assert!(
            gate.contains("start_engine_bound("),
            "the gate starts through the bound start"
        );

        let bound = body_between(&production, "fn start_engine_bound(", "\n}\n");
        assert!(
            !bound.contains(spawn),
            "the bound start only calls the constructor it is given"
        );
        let authorize = bound.find("authorize_engine_start(").expect("it authorizes");
        let spawned = bound
            .find("spawn(paths.sync_root.clone()")
            .expect("then calls the constructor");
        assert!(authorize < spawned, "the authorization runs before the engine exists");
        assert!(
            bound[authorize..spawned].contains(")?"),
            "a failed authorization returns before the spawn"
        );

        let authorize = body_between(&production, "fn authorize_engine_start(", "\n}\n");
        let occupied = authorize
            .find("engine_slot.is_some()")
            .expect("it refuses an occupied slot");
        let keys = authorize
            .find("keys_for_engine_start(acct)")
            .expect("it reads the keys under the slot");
        let bind = authorize.find("bind_local_data_to_session(").expect("it binds");
        assert!(
            occupied < keys && keys < bind,
            "slot, then keys, then the binding:\n{authorize}"
        );
        assert!(
            authorize[bind..].contains(")?;"),
            "a failed binding returns before the keys are handed out"
        );
        assert!(
            authorize[bind..].contains("Ok(StartPermit::Go(keys))"),
            "the keys leave only after the binding"
        );
        let keys_code = body_between(&production, "fn keys_for_engine_start(", "\n}\n");
        assert!(
            keys_code.contains("identity_of_session(email.as_deref(), profile.as_ref())")
                && keys_code.contains("session.email.clone()"),
            "the identity is built from the session the keys come from:\n{keys_code}"
        );
    }

    /// The six starts of the app, and the two paths the reconciler and the startup restore reach them
    /// through, all end in the one gate; none of them reads the session or holds a key itself. The one read of the
    /// session a start site delegates to (`named_session_identity`, for the start after a naming) takes the session's
    /// email and the cached profile and no key material.
    #[test]
    fn every_start_site_takes_the_slot_first_and_reads_no_keys_itself() {
        let production = production_source();
        let sites = [
            "start_engine_if_possible",
            "persist_sync_root_and_start_engine",
            "start_engine_for_pending_finder_install",
            "pick_sync_root",
            "ensure_sync_root_and_engine",
            "start_check_engine",
            // Task 12 fix round 2, item 2: the start after a naming in the background, which keeps a running engine.
            "start_engine_unless_running",
        ];
        let calls: Vec<&str> = production
            .lines()
            .map(str::trim)
            .filter(|line| {
                !line.starts_with("//")
                    && line.contains("spawn_bound_engine(")
                    && !line.contains("fn spawn_bound_engine(")
            })
            .collect();
        assert_eq!(
            calls.len(),
            sites.len(),
            "one call per start site, and nothing else calls the gate: {calls:?}"
        );
        for name in sites {
            let code = code_of(&production, &format!("fn {name}("));
            let gate = code
                .find("spawn_bound_engine(")
                .unwrap_or_else(|| panic!("{name} does not start through the gate"));
            let slot = ["engine.lock().await", "lock_engine_slot("]
                .iter()
                .filter_map(|take| code.find(take))
                .min()
                .unwrap_or_else(|| panic!("{name} takes the engine slot"));
            assert!(slot < gate, "{name}: the slot, then the start:\n{code}");
            for forbidden in [".session", "master_key", "token"] {
                assert!(
                    !code.contains(forbidden),
                    "{name} handles key material itself ({forbidden}):\n{code}"
                );
            }
        }
        // The start after a naming reads who the session is only through `named_session_identity` (the loop above
        // forbids `.session` in the start itself), and that helper takes out the email and the profile, nothing else.
        assert!(
            code_of(&production, "fn start_engine_unless_running(").contains("named_session_identity(&acct)"),
            "the start after a naming reads the session's identity through the helper"
        );
        let identity = code_of(&production, "fn named_session_identity(");
        for forbidden in ["master_key", "token"] {
            assert!(
                !identity.contains(forbidden),
                "named_session_identity handles key material ({forbidden}):\n{identity}"
            );
        }
        assert!(
            crate::source_pin::squeeze(&identity).contains(&crate::source_pin::squeeze(
                "guard.as_ref().and_then(|session| session.email.clone())"
            )),
            "what it takes out of the session is its email:\n{identity}"
        );
        assert_eq!(
            identity.matches(".clone()").count(),
            2,
            "and the only things cloned are that email and the cached profile:\n{identity}"
        );
        // The reconciler's StartEngine and FinishReady, and the startup restore, reach it through these.
        let ports = include_str!("finder_setup/macos_ports.rs");
        assert!(ports.contains("crate::start_check_engine(") && ports.contains("crate::ensure_sync_root_and_engine("));
        let restore = code_of(&production, "async fn restore_session_inner(");
        assert!(restore.contains("start_engine_if_possible("), "{restore}");
    }

    /// Lead ruling 2: the token and the key reach the engine as `Zeroizing`, and no start receives a
    /// copy from its caller. The runner's task entry takes them the same way.
    #[test]
    fn the_keys_travel_as_zeroizing_values_and_no_start_takes_them_as_parameters() {
        let production = production_source();
        let source = |name: &str| {
            let function = body_between(&production, name, "\n}\n");
            function[..function.find(") -> ").expect("a return type")].to_string()
        };
        for name in ["async fn start_engine_if_possible(", "fn spawn_bound_engine("] {
            let signature = source(name);
            for key_material in ["token", "master_key", "[u8; 32]"] {
                assert!(
                    !signature.contains(key_material),
                    "{name} takes {key_material}:\n{signature}"
                );
            }
        }
        assert!(
            !production.contains("token_clone"),
            "apply_session no longer keeps a plain copy of the token"
        );
        let keys = body_between(&production, "fn keys_for_engine_start(", "\n}\n");
        assert!(
            keys.contains("EngineKeys")
                && keys.contains("zeroize::Zeroizing::new(session.token.clone())")
                && keys.contains("zeroize::Zeroizing::new(session.master_key)"),
            "the keys come out wrapped:\n{keys}"
        );
        assert!(
            production.contains("type EngineKeys = (zeroize::Zeroizing<String>, zeroize::Zeroizing<[u8; 32]>);"),
            "the alias is the wrapped pair"
        );
        let runner = [include_str!("runner.rs"), include_str!("runner/lifecycle.rs"), include_str!("runner/telemetry.rs"), include_str!("runner/tick_loop.rs"), include_str!("runner/working_set_signal.rs")].join("\n").replace("\r\n", "\n");
        for (entry, end) in [("pub fn spawn(", ") -> Self {"), ("async fn run(", ") {\n")] {
            let at = runner.find(entry).unwrap_or_else(|| panic!("{entry}"));
            let signature = &runner[at..at + runner[at..].find(end).unwrap_or_else(|| panic!("the end of {entry}"))];
            assert!(
                signature.contains("zeroize::Zeroizing<String>") && signature.contains("zeroize::Zeroizing<[u8; 32]>"),
                "{entry} takes plain keys:\n{signature}"
            );
        }
    }

    /// The pieces of the binding that take a state dir never read the process-wide one, so a test
    /// points them at a throwaway directory and nothing else can be reached (lead ruling 7).
    #[test]
    fn the_binding_never_reads_the_process_wide_state_dir_except_at_the_edges() {
        let production = production_source();
        for signature in [
            "fn authorize_engine_start(",
            "fn bind_local_data_to_session(",
            "fn bind_data_to_session(",
            "fn adopt_unbound_local_data_in(",
        ] {
            let code = code_of(&production, signature);
            assert!(
                !code.contains("beebeeb_state_dir"),
                "{signature} reads the global state dir:\n{code}"
            );
        }
        let impl_block = body_between(&production, "impl StateDbLocalData {", "\n}\n");
        assert!(!impl_block.contains("beebeeb_state_dir()"), "{impl_block}");
        let bound = code_of(&production, "fn start_engine_bound(");
        assert!(
            !bound.contains("beebeeb_state_dir"),
            "fn start_engine_bound( reads the global state dir:\n{bound}"
        );
        assert!(
            code_of(&production, "fn spawn_bound_engine(").contains("LocalDataPaths::production("),
            "the gate resolves it"
        );
        for edge in ["fn production(", "fn adopt_unbound_local_data_at_startup("] {
            assert!(
                code_of(&production, edge).contains("state_paths::beebeeb_state_dir()"),
                "{edge} resolves it"
            );
        }
    }

    /// Lead ruling 3: `is_disposable_cache_path` stays the ONLY deletion gate. The reset's files go only through
    /// `purge_local_state_files`, and its removal loop checks the gate before every `remove_file`; neither the
    /// binding's pure module nor the glue deletes anything on its own.
    #[test]
    fn the_reset_deletes_files_only_through_the_one_disposable_path_gate() {
        let production = production_source();
        let pure = include_str!("account_binding.rs");
        let pure = &pure[..pure.find("#[cfg(test)]").expect("its test module")];
        for needle in ["remove_file", "remove_dir", "std::fs", "fs::"] {
            assert!(!pure.contains(needle), "the pure binding touches files ({needle})");
        }
        let impl_block = body_between(&production, "impl StateDbLocalData {", "\n}\n");
        let reset = body_between(&production, "    fn reset(&self)", "\n    }\n");
        for part in [impl_block, reset] {
            for needle in ["remove_file", "remove_dir", "std::fs", "fs::"] {
                assert!(
                    !part.contains(needle),
                    "the glue deletes a file itself ({needle}):\n{part}"
                );
            }
        }
        assert!(
            reset.contains("purge_local_state_files_in("),
            "the reset is the sign-out purge:\n{reset}"
        );
        // Task 1887: the purge's one removal is the `remove_file` its wrapper hands down, and it runs only in
        // `remove_purge_paths`, right behind the gate.
        let wrapper = code_of(&production, "fn purge_local_state_files_as(");
        assert_eq!(
            wrapper.matches("remove_file(").count(),
            1,
            "one removal in the purge:\n{wrapper}"
        );
        assert!(wrapper.contains("purge_local_state_files_with("), "{wrapper}");
        let purge = code_of(&production, "fn purge_local_state_files_with(");
        for needle in ["remove_file", "remove_dir", "remove("] {
            assert!(
                !purge.contains(needle),
                "the purge removes only through `remove_purge_paths` ({needle}):\n{purge}"
            );
        }
        let removal = code_of(&production, "fn remove_purge_paths(");
        let removals: Vec<usize> = removal.match_indices("remove(").map(|(at, _)| at).collect();
        assert_eq!(removals.len(), 1, "one removal in the purge:\n{removal}");
        assert!(
            removal[..removals[0]]
                .rfind("is_disposable_cache_path(")
                .is_some_and(|gate| removal[gate..removals[0]].lines().count() < 10),
            "the gate runs right before the removal:\n{removal}"
        );
    }

    /// Lead ruling 4 (R11): the Windows arm refuses instead of resetting. The pure arm is tested
    /// everywhere; this pins that the glue passes the right flag, so a Mac cannot hide a flipped one.
    #[test]
    fn the_glue_forbids_a_reset_on_windows_only() {
        let production = production_source();
        let bind = code_of(&production, "fn bind_data_to_session(");
        // Task 1887 (spec 2026-10-10 W12): the binding's answer is read once the reset's kept folder was handed on, so
        // its error is returned by `match bound?`.
        let flag = r#"bind_before_engine_start(data, session, !cfg!(target_os = "windows"));"#;
        assert!(
            bind.contains(&format!("let bound = account_binding::{flag}")) && bind.contains("match bound? {"),
            "{bind}"
        );
        assert!(
            bind.contains("Trigger::Repair"),
            "a reset on a Mac asks the reconciler for a Repair:\n{bind}"
        );
        assert!(
            !bind.contains(".await"),
            "and never waits for it: the reconciler starts engines through here:\n{bind}"
        );
    }

    /// Task 2267: a Windows start refused with `OTHER_ACCOUNT_ON_WINDOWS` hands its data to the ownership proof, after
    /// the start returned and whatever it returned, for the session that start read (the same account runtime, the
    /// turn read under the engine slot). The proof starts sync only once it adopted, and then only under the
    /// session-transition lock for that same session (fix round 1, findings 6 and 8). One proof per session at a time,
    /// its slot freed however it ends, a panic included (finding 7). Each question is bounded and the asking stops
    /// when the session changes (finding 4); a refused token is not reported as "offline" (finding 5).
    #[test]
    fn a_windows_refusal_over_unowned_data_asks_for_the_proof_and_only_an_adoption_starts_sync() {
        let production = production_source();
        let squeeze = crate::source_pin::squeeze;
        let gate = squeeze(&code_of(&production, "fn spawn_bound_engine("));
        let hook = squeeze(
            "#[cfg(target_os = \"windows\")] if matches!(&started, Err(error) if error == account_binding::OTHER_ACCOUNT_ON_WINDOWS) && let Ok(active) = state.active_account() && std::ptr::eq(Arc::as_ptr(&active), acct) { prove_own_data_in_background(surface_app, paths.sync_root.clone(), active, acct.session_generation()); }",
        );
        let surfaced = gate.find(&squeeze("surface_reset_folder(&surface_app, &folder, files);")).expect("surfaced");
        let asked = gate.find(&hook).unwrap_or_else(|| panic!("the hook: {gate}"));
        assert!(surfaced < asked && gate.ends_with("started"), "{gate}");

        let background = squeeze(&code_of(&production, "fn prove_own_data_in_background("));
        for step in [
            "impl Drop for Flight",
            "if *in_flight == Some(key) { return; }",
            "let _flight = flight;",
        ] {
            assert!(background.contains(&squeeze(step)), "{step}: {background}");
        }
        let adopted = background.find("Outcome::Adopted(tally)").expect("the adopted arm");
        let locked = background.find(&squeeze("let transition = SESSION_TRANSITION.lock().await;")).expect("the lock");
        let current = background.find(&squeeze("if !session_still_active(&app, &acct, turn)")).expect("the check");
        let restart = background.find("start_engine_if_possible(").expect("a restart");
        let other = background.find("Ok(outcome)=>").expect("every other outcome");
        assert!(adopted < locked && locked < current && current < restart && restart < other, "{background}");
        assert_eq!(background.matches("start_engine_if_possible(").count(), 1, "{background}");

        let proof = squeeze(&code_of(&production, "async fn prove_own_data_of_session("));
        for step in [
            "keys_for_engine_start(acct)",
            "own_data_proof::prove_and_adopt(",
            // Fix round 2 (review P2-1): this account's tree, bounded; self-review F2: its trash too.
            "tokio::time::timeout(LIST_TIMEOUT,",
            "api.sync_snapshot().await",
            "api.list_trashed_files_all().await",
            "own_data_proof::listing_of(",
            // Self-review F5: the proof's own client waits longer than the app's 30 s per request.
            ".with_request_timeout(PROOF_REQUEST_TIMEOUT)",
            // Review P2-2: the proof's client is one a Lock or a sign-out waits for.
            "windows_cf::track_credentials(&api);",
            "|| session_still_active(app, acct, turn)",
            "Outcome::Unavailable(tally) if tally.signed_out == 0",
            "record_engine_refusal(acct, turn, account_binding::IDENTITY_UNKNOWN);",
        ] {
            assert!(proof.contains(&squeeze(step)), "{step}: {proof}");
        }
        assert_eq!(proof.matches("api.get_file(").count(), 0, "no question per file: {proof}");
        let still = squeeze(&code_of(&production, "fn session_still_active("));
        assert!(still.contains(&squeeze("acct.session_unchanged_since(turn)")), "{still}");
        assert!(still.contains(&squeeze("Arc::ptr_eq(&active, acct)")), "{still}");

        // Review P2-2: a Lock and a sign-out stop every running proof before they wait for the credentials, so a proof
        // never holds the key and the token past either.
        assert!(
            background.contains(&squeeze("OWN_DATA_PROOFS.lock()")) && background.contains("proofs.push(proof)"),
            "the proof is kept to be stopped: {background}"
        );
        for (name, body) in [
            ("lock_vault", code_of(&production, "async fn lock_vault(")),
            ("clear_session_impl", code_of(&production, "async fn clear_session_impl(")),
        ] {
            let body = squeeze(&body);
            let stop = body.find("stop_own_data_proofs();").unwrap_or_else(|| panic!("{name} stops the proofs: {body}"));
            let wait = body
                .find(&squeeze("windows_cf::wait_for_credential_release().await?;"))
                .unwrap_or_else(|| panic!("{name} waits: {body}"));
            assert!(stop < wait, "{name} stops the proofs before it waits: {body}");
        }
        let stops = squeeze(&code_of(&production, "fn stop_own_data_proofs("));
        assert!(stops.contains(".abort()"), "{stops}");
    }

    /// Self-review F5: the proof's listing is not bounded by the app's 30 s per request. A large tree on a slow link has
    /// minutes (a Lock or a sign-out stops the proof at once anyway), and the whole listing has room for more than one.
    #[cfg(target_os = "windows")]
    #[test]
    fn the_proofs_listing_has_minutes_not_the_apps_thirty_seconds() {
        assert!(PROOF_REQUEST_TIMEOUT >= std::time::Duration::from_secs(300), "{PROOF_REQUEST_TIMEOUT:?}");
        assert!(PROOF_REQUEST_TIMEOUT.as_secs() > crate::link_health::API_REQUEST_TIMEOUT_SECS);
        assert!(LIST_TIMEOUT >= PROOF_REQUEST_TIMEOUT * 2, "{LIST_TIMEOUT:?}");
    }

    /// Task 2267 (fix round 2): what Settings › Sync decides about the changes found while sync was paused. A keep a
    /// crash interrupted is settled in the one engine start, after the binding (so it acts on this account's own data
    /// only) and before the engine exists (lead ruling (a) 4). The three commands are registered; Upload and Keep run
    /// under the session-transition lock and only on the signed-in account's own data; no command takes a path.
    #[test]
    fn the_found_while_paused_decision_is_wired_and_an_interrupted_keep_is_settled_before_the_engine() {
        let production = production_source();
        let squeeze = crate::source_pin::squeeze;
        let bound = squeeze(&code_of(&production, "fn start_engine_bound("));
        let at = |snippet: &str| {
            bound
                .find(&squeeze(snippet))
                .unwrap_or_else(|| panic!("{snippet}:\n{bound}"))
        };
        let permitted = at("StartPermit::FinderRemovalOwed => return Ok(EngineStart::FinderRemovalOwed)");
        let settled = at("#[cfg(target_os = \"windows\")] settle_an_interrupted_keep(paths);");
        let spawned = at("spawn(paths.sync_root.clone()");
        assert!(permitted < settled && settled < spawned, "{bound}");
        let settle = squeeze(&code_of(&production, "fn settle_an_interrupted_keep("));
        assert!(settle.contains(&squeeze("found_while_paused::recover(&db, &paths.sync_root)")), "{settle}");

        let handler = body_between(&production, "tauri::generate_handler![", "])");
        for command in ["found_while_paused_review,", "found_while_paused_upload,", "found_while_paused_keep,", "show_kept_folder,"] {
            assert!(handler.contains(command), "{command} is registered");
        }
        let windows = "#[cfg(target_os = \"windows\")]\n#[tauri::command]\n";
        for (signature, steps) in [
            (
                "async fn found_while_paused_review(state: State<'_, AppState>) -> Result<found_while_paused::Review, String> {",
                &[
                    "let _transition = SESSION_TRANSITION.lock().await;",
                    "let identity = decision_identity(&state)?;",
                    "own_local_data_for_decision(&identity)?",
                    "found_while_paused::review(&db.found_while_paused_changes()",
                    "review.synced_by = kept_folder_synced_by(&sync_root);",
                ][..],
            ),
            (
                // Lead review round 3, P3-1: only the changes the card showed.
                "async fn found_while_paused_upload(state: State<'_, AppState>, op_ids: Vec<String>) -> Result<FoundWhilePausedReleased, String> {",
                &[
                    "let _transition = SESSION_TRANSITION.lock().await;",
                    "let identity = decision_identity(&state)?;",
                    "own_local_data_for_decision(&identity)?",
                    "db.release_found_while_paused(&op_ids, now_unix_seconds())",
                ][..],
            ),
            (
                // Self-review of round 3, F7: only the changes the card showed.
                "async fn found_while_paused_keep(\n    state: State<'_, AppState>,\n    op_ids: Vec<String>,\n) -> Result<found_while_paused::KeepResult, String> {",
                &[
                    "let _transition = SESSION_TRANSITION.lock().await;",
                    "let identity = decision_identity(&state)?;",
                    "own_local_data_for_decision(&identity)?",
                    "found_while_paused::keep(&db, &sync_root, &today, &found_while_paused::rename_no_replace, &op_ids)",
                    "kept.synced_by = kept_folder_synced_by(&sync_root);",
                ][..],
            ),
            (
                "async fn show_kept_folder() -> Result<(), String> {",
                &["kept_folder_to_show(&sync_root, db.last_kept_dir()", "std::process::Command::new(\"explorer\").arg(&folder)"][..],
            ),
        ] {
            let body = squeeze(&code_of(&production, &format!("{windows}{signature}")));
            for step in steps {
                assert!(body.contains(&squeeze(step)), "{step}: {body}");
            }
        }
        let own = squeeze(&code_of(&production, "fn own_state_db("));
        assert!(
            own.contains(&squeeze("account_binding::same_account(&owner, identity) == Some(true)")),
            "{own}"
        );
    }

    /// Task 2267 (fix round 2): Upload and Keep act on this PC's local data only when its recorded owner is the
    /// account signed in now. Another account's data, data with no owner, no database and no session change nothing.
    #[test]
    fn a_decision_about_found_changes_acts_only_on_the_signed_in_accounts_own_data() {
        let refused = Err(FOUND_WHILE_PAUSED_NOT_YOURS.to_string());
        let mine = local_data_of(Some(alice()));
        assert!(own_state_db(&alice(), mine.dir.path()).is_ok(), "the owner decides");
        assert_eq!(own_state_db(&bob(), mine.dir.path()).map(|_| ()), refused, "another account does not");
        let unowned = local_data_of(None);
        assert_eq!(own_state_db(&alice(), unowned.dir.path()).map(|_| ()), refused, "data nobody owns yet");
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(own_state_db(&alice(), empty.path()).map(|_| ()), refused, "no database");
        let by_email = Identity::new(None, Some("A@Beebeeb.io"));
        assert!(own_state_db(&by_email, mine.dir.path()).is_ok(), "the same account by its email");

        assert_eq!(decision_identity(&AuthorizeFixture::with_session(&alice()).state), Ok(alice()));
        assert_eq!(decision_identity(&AuthorizeFixture::without_session().state).map(|_| ()), refused, "no session");
    }

    /// Task 2267, lead ruling on self-review F9 (2026-10-29): Keep's stop message names the kept folder as it is made.
    #[cfg(target_os = "windows")]
    #[test]
    fn keep_stopped_names_the_kept_folder_as_it_is_made() {
        assert!(FOUND_WHILE_PAUSED_KEEP_STOPPED.contains("“Beebeeb - not uploaded”"), "{FOUND_WHILE_PAUSED_KEEP_STOPPED}");
        let named = format!("“{}”", crate::found_while_paused::KEPT_FOLDER_NAME);
        assert!(FOUND_WHILE_PAUSED_KEEP_STOPPED.contains(&named), "{FOUND_WHILE_PAUSED_KEEP_STOPPED}");
    }

    /// Task 2267 (fix round 2): "Show in Explorer" opens only the dated folder the last keep recorded, inside
    /// `Beebeeb - not uploaded` next to the Beebeeb folder, and only while it is there.
    #[test]
    fn show_in_explorer_opens_only_a_dated_folder_in_the_kept_folder_next_to_the_beebeeb_folder() {
        let top = tempfile::tempdir().unwrap();
        let root = top.path().join("Beebeeb");
        let kept = top.path().join("Beebeeb - not uploaded");
        let dated = kept.join("2026-10-28");
        for dir in [&root, &dated, &kept.join("notes"), &top.path().join("elsewhere").join("2026-10-28")] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let recorded = |path: &Path| Some(path.to_string_lossy().into_owned());
        assert_eq!(kept_folder_to_show(&root, Ok(recorded(&dated))), Ok(dated.clone()));
        for other in [
            None,
            recorded(&kept),
            recorded(&kept.join("..")),
            recorded(&dated.join("..")),
            recorded(&dated.join("..").join("2026-10-28")),
            recorded(&kept.join("notes")),
            recorded(&kept.join("2026-10-29")),
            recorded(&root),
            recorded(&top.path().join("elsewhere").join("2026-10-28")),
            Some("2026-10-28".to_string()),
        ] {
            assert!(kept_folder_to_show(&root, Ok(other.clone())).is_err(), "{other:?}");
        }
        assert!(kept_folder_to_show(&root, Err(rusqlite::Error::InvalidQuery)).is_err());
    }

    /// Task 2267 (fix round 2, lead ruling (a) 4), behaviourally: a keep that a crash interrupted after its move is
    /// finished by the next engine start before the engine is created. When the engine exists the journal is gone and
    /// the kept item's row and queued change are forgotten; its bytes are in the kept folder.
    #[cfg(target_os = "windows")]
    #[test]
    fn a_keep_a_crash_interrupted_is_finished_before_the_next_engine_is_created() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let local = local_data_of(None);
            let top = tempfile::tempdir().unwrap();
            let root = top.path().join("Beebeeb");
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(root.join("A.txt"), b"local bytes").unwrap();
            assert_eq!(
                local
                    .data
                    .db
                    .adopt_owner_holding(None, &alice(), &std::collections::HashSet::new(), 5)
                    .unwrap(),
                Some(1),
                "precondition: the PC is this account's and its change is held"
            );
            let crash_after_moving = |from: &Path, to: &Path| -> std::io::Result<()> {
                crate::found_while_paused::rename_no_replace(from, to).unwrap();
                panic!("the process ends here");
            };
            let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let held: Vec<String> =
                    local.data.db.found_while_paused_ops().unwrap().into_iter().map(|op| op.op_id).collect();
                crate::found_while_paused::keep(&local.data.db, &root, "2026-10-28", &crash_after_moving, &held)
            }));
            assert!(
                crashed.is_err() && local.data.db.keep_journal().unwrap().is_some(),
                "precondition: a keep was interrupted after its move"
            );

            let fx = AuthorizeFixture::with_session(&alice());
            let mut slot = fx.acct.engine.lock().await;
            let paths = LocalDataPaths::for_test(local.dir.path(), &root);
            let seen = std::cell::RefCell::new(None);
            let result = start_engine_bound(&fx.state, &fx.acct, &mut slot, &paths, |_root, _token, _key| {
                *seen.borrow_mut() = Some((
                    local.data.db.keep_journal().unwrap(),
                    local.data.db.get_file("file-a").unwrap().is_some(),
                    local.data.db.list_due_operations(i64::MAX).unwrap().len()
                        + local.data.db.found_while_paused_ops().unwrap().len(),
                ));
                crate::runner::EngineRunner::for_test_with_task(tokio::spawn(async {}))
            });
            assert_eq!(result, Ok(EngineStart::Started));
            assert_eq!(
                seen.into_inner(),
                Some((None, false, 0)),
                "when the engine is created the keep is finished: no journal, no row, no queued change"
            );
            let kept = top
                .path()
                .join("Beebeeb - not uploaded")
                .join("2026-10-28")
                .join("A.txt");
            assert_eq!(std::fs::read(kept).unwrap(), b"local bytes", "nothing lost");
            assert!(!root.join("A.txt").exists());
        });
    }

    // ---- the local data under test ----

    struct LocalFixture {
        data: StateDbLocalData,
        files: Vec<PathBuf>,
        staging: PathBuf,
        dir: tempfile::TempDir,
    }

    impl Drop for LocalFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.staging);
        }
    }

    /// A computer holding one account's queued change, staged upload and cached file, in real
    /// files under the OS temp dir so `is_disposable_cache_path` allows their removal.
    fn local_data_of(owner: Option<Identity>) -> LocalFixture {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::state_db::StateDb::open(dir.path().join("state.db")).unwrap();
        let staging = std::env::temp_dir().join(format!("bb-test-r10-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&staging).unwrap();
        let payload = staging.join("op-1-payload.bin");
        let cache = staging.join("file-a-cache.bin");
        let staged = staging.join("staged-1.bin");
        for path in [&payload, &cache, &staged] {
            std::fs::write(path, b"local bytes").unwrap();
        }
        db.upsert_file(&crate::state_db::FileEntry {
            file_id: "file-a".into(),
            path: "/A.txt".into(),
            status: crate::state_db::FileStatus::Local,
            size_bytes: 11,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: crate::state_db::ItemKind::File,
        })
        .unwrap();
        db.mark_cached("file-a", cache.to_str().unwrap(), 11, 10).unwrap();
        db.enqueue_operation(&crate::state_db::PendingOperation {
            op_id: "op-1".into(),
            kind: crate::state_db::OperationKind::UploadVersion,
            file_id: Some("file-a".into()),
            parent_id: None,
            target_path: Some("/A.txt".into()),
            // A new file's upload says it creates the file, as the scan queues it: its row counts as made on this PC
            // (task 2267, lead review round 2, P2-1).
            metadata_json: Some(r#"{"operation":"create_file"}"#.into()),
            payload_path: Some(payload.to_str().unwrap().to_string()),
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 5,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: 100,
            updated_at: 100,
        })
        .unwrap();
        db.track_staged_payload(staged.to_str().unwrap(), None, false).unwrap();
        if let Some(owner) = owner {
            db.set_owner(&owner).unwrap();
        }
        LocalFixture {
            data: StateDbLocalData {
                db,
                sync_root: None,
                staging_dirs: Vec::new(),
                kept: Default::default(),
            },
            files: vec![payload, cache, staged],
            staging,
            dir,
        }
    }

    // ---- the decision, run against real state.db rows and real files ----

    /// R10: a different account never reuses this computer's local data, whichever way it is
    /// identified. macOS and Linux reset it before the engine starts; Windows starts nothing and
    /// discards nothing silently.
    #[test]
    fn a_different_account_never_reuses_local_data() {
        let by_id_and_email = bob();
        let by_email_only = Identity::new(None, Some("b@beebeeb.io"));
        for session in [by_id_and_email, by_email_only] {
            let local = local_data_of(Some(alice()));
            let db = &local.data.db;
            let result = bind_data_to_session(&AppState::default(), &local.data, &session);
            if RESET_ALLOWED {
                assert_eq!(result, Ok(()), "{session:?}");
                assert!(
                    db.list_due_operations(i64::MAX).unwrap().is_empty(),
                    "no queued change survives"
                );
                assert!(db.list_files().unwrap().is_empty(), "no file row survives");
                assert!(!db.has_account_data().unwrap());
                for path in &local.files {
                    assert!(!path.exists(), "{} is deleted", path.display());
                }
                assert_eq!(db.owner().unwrap(), Some(session.clone()));
            } else {
                assert_eq!(
                    result,
                    Err(crate::account_binding::OTHER_ACCOUNT_ON_WINDOWS.to_string())
                );
                assert_eq!(db.list_due_operations(i64::MAX).unwrap().len(), 1, "nothing discarded");
                assert_eq!(db.owner().unwrap(), Some(alice()));
            }
        }
    }

    /// R8 + R10: the same account keeps everything, also offline (by its exact email).
    #[test]
    fn the_same_account_never_purges() {
        for session in [alice(), Identity::new(None, Some("a@beebeeb.io"))] {
            let local = local_data_of(Some(alice()));
            assert_eq!(
                bind_before_engine_start(&local.data, &session, RESET_ALLOWED),
                Ok(Bound::Kept),
                "{session:?}"
            );
            assert_eq!(
                bind_data_to_session(&AppState::default(), &local.data, &session),
                Ok(())
            );
            assert_eq!(local.data.db.list_due_operations(i64::MAX).unwrap().len(), 1);
            for path in &local.files {
                assert!(path.exists(), "{} is kept", path.display());
            }
            assert_eq!(
                local.data.db.owner().unwrap().and_then(|owner| owner.user_id),
                Some("u-a".to_string())
            );
        }
    }

    /// Ruling A″ (fix round 3; this was F5): a session whose email is a letter-case variant of the owner's (no user id to
    /// settle it) is the SAME account, because the server keeps one account per canonical address. It starts on every
    /// platform and keeps everything; the record follows the session's spelling. History: Task 10 read it as another
    /// account (a reset on macOS and Linux); rulings A and A′ refused it.
    #[test]
    fn a_case_variant_of_the_owners_email_is_the_same_account_and_keeps_its_data() {
        let local = local_data_of(Some(Identity::new(None, Some("sam@beebeeb.io"))));
        let result = bind_data_to_session(
            &AppState::default(),
            &local.data,
            &Identity::new(None, Some("Sam@beebeeb.io")),
        );
        assert_eq!(result, Ok(()), "it starts, on every platform");
        assert_eq!(
            local.data.db.list_due_operations(i64::MAX).unwrap().len(),
            1,
            "nothing discarded"
        );
        for path in &local.files {
            assert!(path.exists(), "{} is kept", path.display());
        }
        assert_eq!(
            local.data.db.owner().unwrap(),
            Some(Identity::new(None, Some("Sam@beebeeb.io"))),
            "one owner, in the session's spelling"
        );
    }

    /// Fail closed: local data without a recorded owner is never handed to a session.
    #[test]
    fn local_data_without_an_owner_is_reset_before_any_account_uses_it() {
        let local = local_data_of(None);
        let result = bind_data_to_session(&AppState::default(), &local.data, &bob());
        if RESET_ALLOWED {
            assert_eq!(result, Ok(()));
            assert!(local.data.db.list_due_operations(i64::MAX).unwrap().is_empty());
            for path in &local.files {
                assert!(!path.exists(), "{} is deleted", path.display());
            }
            assert_eq!(local.data.db.owner().unwrap(), Some(bob()));
        } else {
            assert_eq!(
                result,
                Err(crate::account_binding::OTHER_ACCOUNT_ON_WINDOWS.to_string())
            );
            assert_eq!(local.data.db.list_due_operations(i64::MAX).unwrap().len(), 1);
        }
    }

    /// R10, behaviourally: another account's engine starts only AFTER the previous account's local
    /// state was purged. The whole gate runs (`start_engine_bound`, the function the one real spawn sits
    /// in), with a constructor that records what is on disk at the exact moment an engine would be created.
    /// Windows refuses instead, and the constructor is never called.
    #[test]
    fn another_accounts_engine_starts_only_after_the_local_state_was_purged() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let local = local_data_of(Some(alice()));
            let fx = AuthorizeFixture::with_session(&bob());
            let mut slot = fx.acct.engine.lock().await;
            let paths = LocalDataPaths::for_test(local.dir.path(), &local.staging);
            let seen = std::cell::RefCell::new(None);
            let spawner = || {
                |_root: PathBuf, token: zeroize::Zeroizing<String>, key: zeroize::Zeroizing<[u8; 32]>| {
                    assert_eq!(
                        (token.as_str(), *key),
                        ("tok-r10", [7u8; 32]),
                        "b's own keys start b's engine"
                    );
                    *seen.borrow_mut() = Some((
                        local.data.db.list_due_operations(i64::MAX).unwrap().len(),
                        local.files.iter().filter(|path| path.exists()).count(),
                        local.data.db.owner().unwrap(),
                    ));
                    crate::runner::EngineRunner::for_test_with_task(tokio::spawn(async {}))
                }
            };
            let mut result = start_engine_bound(&fx.state, &fx.acct, &mut slot, &paths, spawner());
            if cfg!(target_os = "macos") && RESET_ALLOWED {
                // The reset left the previous account's Finder domain registered, so nothing starts yet ...
                assert_eq!(result, Ok(EngineStart::FinderRemovalOwed));
                assert!(
                    seen.borrow().is_none() && slot.is_none(),
                    "no engine was created while the removal is owed"
                );
                // ... until a removal is confirmed.
                #[cfg(target_os = "macos")]
                clear_finder_removal_owed_in(local.dir.path());
                result = start_engine_bound(&fx.state, &fx.acct, &mut slot, &paths, spawner());
            }
            if RESET_ALLOWED {
                assert_eq!(result, Ok(EngineStart::Started));
                assert_eq!(
                    seen.into_inner(),
                    Some((0, 0, Some(bob()))),
                    "when the engine is created: no queued change, no file of the previous account, and b is the owner"
                );
                assert!(slot.is_some(), "and it is in the slot");
            } else {
                assert_eq!(
                    result,
                    Err(crate::account_binding::OTHER_ACCOUNT_ON_WINDOWS.to_string())
                );
                assert_eq!(seen.into_inner(), None, "no engine was created");
                assert!(slot.is_none());
            }
        });
    }

    /// An owner that cannot be compared with the session stops the start and deletes nothing, on every
    /// platform.
    #[test]
    fn an_owner_that_cannot_be_compared_deletes_nothing_and_starts_nothing() {
        let local = local_data_of(Some(Identity::new(Some("u-a"), None)));
        let result = bind_data_to_session(
            &AppState::default(),
            &local.data,
            &Identity::new(None, Some("a@beebeeb.io")),
        );
        assert_eq!(result, Err(crate::account_binding::IDENTITY_UNKNOWN.to_string()));
        assert_eq!(local.data.db.list_due_operations(i64::MAX).unwrap().len(), 1);
        assert_eq!(local.data.db.owner().unwrap(), Some(Identity::new(Some("u-a"), None)));
        for path in &local.files {
            assert!(path.exists(), "{} is kept", path.display());
        }
    }

    #[test]
    fn a_fresh_computer_records_its_first_account() {
        let dir = tempfile::tempdir().unwrap();
        let data = StateDbLocalData {
            db: crate::state_db::StateDb::open(dir.path().join("state.db")).unwrap(),
            sync_root: None,
            staging_dirs: Vec::new(),
            kept: Default::default(),
        };
        assert_eq!(bind_before_engine_start(&data, &bob(), RESET_ALLOWED), Ok(Bound::Kept));
        assert_eq!(data.db.owner().unwrap(), Some(bob()));
    }

    /// Lead ruling 4: a purge that fails blocks the engine. Here the failure is at the row step (a trigger that
    /// refuses the queue's deletion stands in for a disk that fails): nothing starts, and no row and no owner
    /// changes (the rows are cleared in one transaction). The file step failing is the next two tests.
    #[test]
    fn a_purge_whose_row_step_fails_blocks_the_engine_start() {
        let local = local_data_of(Some(alice()));
        {
            let conn = rusqlite::Connection::open(local.dir.path().join("state.db")).unwrap();
            conn.execute_batch("CREATE TRIGGER refuse_the_delete BEFORE DELETE ON operation_queue BEGIN SELECT RAISE(ABORT, 'disk I/O error'); END;")
                .unwrap();
        }
        let fx = AuthorizeFixture::with_session(&bob());
        let slot = fx.acct.engine.try_lock().unwrap();
        let result = authorize_engine_start(
            &fx.state,
            &fx.acct,
            &slot,
            &LocalDataPaths::for_test(local.dir.path(), &local.staging),
        );
        if RESET_ALLOWED {
            let error = result.expect_err("a failed purge starts nothing");
            assert!(error.contains("disk I/O error"), "{error}");
        } else {
            assert!(result.is_err(), "Windows refuses before any purge");
        }
        assert_eq!(
            local.data.db.owner().unwrap(),
            Some(alice()),
            "the new account is not recorded over data still there"
        );
        assert_eq!(
            local.data.db.list_due_operations(i64::MAX).unwrap().len(),
            1,
            "the queue rows are untouched"
        );
        if RESET_ALLOWED {
            // The purge removes files FIRST and clears rows only when every file is gone (fix round 1, F4), so
            // a failure at the row step comes after the files. The reset of another account's data is going
            // ahead by design; what matters is that nothing started and no row or owner changed.
            for path in &local.files {
                assert!(
                    !path.exists(),
                    "{} was removed before the row step failed",
                    path.display()
                );
            }
        }
    }

    /// A state.db that cannot be opened stops the start too.
    #[test]
    fn a_state_db_that_cannot_be_opened_blocks_the_engine_start() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("state.db"),
            b"this is not a database, and it is long enough to be read as one: 0123456789",
        )
        .unwrap();
        let fx = AuthorizeFixture::with_session(&bob());
        let slot = fx.acct.engine.try_lock().unwrap();
        let error = authorize_engine_start(
            &fx.state,
            &fx.acct,
            &slot,
            &LocalDataPaths::for_test(dir.path(), dir.path()),
        )
        .expect_err("an unreadable database starts nothing");
        assert!(error.contains("state.db") || error.contains("database"), "{error}");
    }

    // ---- the gate's own contract ----

    struct AuthorizeFixture {
        state: AppState,
        acct: Arc<crate::account::AccountRuntime>,
    }

    impl AuthorizeFixture {
        fn without_session() -> Self {
            let state = AppState::default();
            let id = crate::account::AccountId::new_v4();
            crate::account::synthesize_single_account(&state, id);
            let acct = state.active_account().unwrap();
            Self { state, acct }
        }

        /// A signed-in account whose session is `identity`'s (its email, and its profile when it has an id).
        fn with_session(identity: &Identity) -> Self {
            let fx = Self::without_session();
            *fx.acct.session.lock().unwrap() = Some(Session {
                token: "tok-r10".into(),
                master_key: [7u8; 32],
                email: identity.email.clone(),
            });
            *fx.acct.auth_email.lock().unwrap() = identity.email.clone();
            if let (Some(user_id), Some(email)) = (&identity.user_id, &identity.email) {
                *fx.acct.cached_profile.lock().unwrap() = Some(
                    serde_json::from_str::<account_dto::AccountProfile>(&format!(
                        r#"{{"user_id":"{user_id}","email":"{email}","email_verified":true,"created_at":"2026-01-01T00:00:00Z"}}"#
                    ))
                    .unwrap(),
                );
            }
            fx
        }
    }

    /// A start onto a database that has never had an owner first owes the Finder removal on a Mac (R3); this confirms
    /// that removal, as the reconciler does, and starts again. Off a Mac it is one start.
    fn authorize_after_removal(
        fx: &AuthorizeFixture,
        slot: &tokio::sync::MutexGuard<'_, Option<crate::runner::EngineRunner>>,
        paths: &LocalDataPaths,
    ) -> Result<StartPermit, String> {
        let first = authorize_engine_start(&fx.state, &fx.acct, slot, paths);
        if matches!(first, Ok(StartPermit::FinderRemovalOwed)) {
            #[cfg(target_os = "macos")]
            clear_finder_removal_owed_in(&paths.state_dir);
            return authorize_engine_start(&fx.state, &fx.acct, slot, paths);
        }
        first
    }

    #[test]
    fn a_start_with_no_session_starts_nothing_and_binds_nothing() {
        let fx = AuthorizeFixture::without_session();
        let dir = tempfile::tempdir().unwrap();
        let slot = fx.acct.engine.try_lock().unwrap();
        assert!(matches!(
            authorize_engine_start(
                &fx.state,
                &fx.acct,
                &slot,
                &LocalDataPaths::for_test(dir.path(), dir.path())
            )
            .unwrap(),
            StartPermit::NoSession
        ));
        assert!(
            !dir.path().join("state.db").exists(),
            "no database was even created for a vault that is locked"
        );
        assert_eq!(
            keys_for_engine_start(&fx.acct).err(),
            Some(EngineStartBlocked::NoSession)
        );
    }

    #[test]
    fn a_start_with_a_session_binds_and_then_hands_out_the_keys() {
        let fx = AuthorizeFixture::with_session(&alice());
        let dir = tempfile::tempdir().unwrap();
        let slot = fx.acct.engine.try_lock().unwrap();
        let StartPermit::Go((token, key)) =
            authorize_after_removal(&fx, &slot, &LocalDataPaths::for_test(dir.path(), dir.path())).unwrap()
        else {
            panic!("keys for a signed-in account");
        };
        assert_eq!((token.as_str(), *key), ("tok-r10", [7u8; 32]));
        let db = crate::state_db::StateDb::open(dir.path().join("state.db")).unwrap();
        assert_eq!(
            db.owner().unwrap(),
            Some(alice()),
            "the first account to start here owns the data"
        );
    }

    #[test]
    fn a_start_never_overwrites_an_engine_in_the_slot() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let fx = AuthorizeFixture::with_session(&alice());
            let task = tokio::spawn(async {});
            *fx.acct.engine.lock().await = Some(crate::runner::EngineRunner::for_test_with_task(task));
            let dir = tempfile::tempdir().unwrap();
            let slot = fx.acct.engine.lock().await;
            let error = authorize_engine_start(
                &fx.state,
                &fx.acct,
                &slot,
                &LocalDataPaths::for_test(dir.path(), dir.path()),
            )
            .expect_err("an occupied slot is never started over");
            assert_eq!(error, ENGINE_SLOT_OCCUPIED_ERROR);
            assert!(!dir.path().join("state.db").exists(), "and nothing was bound");
        });
    }

    #[test]
    fn a_start_refuses_while_an_earlier_stop_is_unconfirmed_before_it_binds() {
        let fx = AuthorizeFixture::with_session(&alice());
        fx.acct.engine_stop_unconfirmed.store(true, Ordering::SeqCst);
        let dir = tempfile::tempdir().unwrap();
        let slot = fx.acct.engine.try_lock().unwrap();
        let error = authorize_engine_start(
            &fx.state,
            &fx.acct,
            &slot,
            &LocalDataPaths::for_test(dir.path(), dir.path()),
        )
        .expect_err("refused");
        assert_eq!(error, ENGINE_START_REFUSED_ERROR);
        assert!(!dir.path().join("state.db").exists(), "a refused start binds nothing");
    }

    fn profile_of(user_id: &str, email: &str) -> account_dto::AccountProfile {
        serde_json::from_str::<account_dto::AccountProfile>(&format!(
            r#"{{"user_id":"{user_id}","email":"{email}","email_verified":true,"created_at":"2026-01-01T00:00:00Z"}}"#
        ))
        .unwrap()
    }

    #[test]
    fn the_session_identity_takes_the_profile_only_when_it_is_this_sessions() {
        let profile = |email: &str| profile_of("u-a", email);
        assert_eq!(
            identity_of_session(Some("a@beebeeb.io"), Some(&profile("a@beebeeb.io"))),
            Identity::new(Some("u-a"), Some("a@beebeeb.io"))
        );
        assert_eq!(
            identity_of_session(Some("a@beebeeb.io"), Some(&profile("A@beebeeb.io"))),
            Identity::new(None, Some("a@beebeeb.io")),
            "a profile whose email differs only in case is another account's: never folded"
        );
        assert_eq!(
            identity_of_session(Some("a@beebeeb.io"), Some(&profile("someone-else@beebeeb.io"))),
            Identity::new(None, Some("a@beebeeb.io")),
            "a profile of another email is ignored"
        );
        assert_eq!(
            identity_of_session(Some("a@beebeeb.io"), None),
            Identity::new(None, Some("a@beebeeb.io")),
            "offline: the email alone"
        );
        assert_eq!(
            identity_of_session(None, Some(&profile("a@beebeeb.io"))),
            Identity::default(),
            "no session email: a cached profile is nobody's to borrow"
        );
        assert_eq!(
            identity_of_session(Some("  "), Some(&profile("a@beebeeb.io"))),
            Identity::default(),
            "an empty email is no email"
        );
    }

    /// F1: the identity of a session is its own. A session with no email is unidentified even when a cached profile
    /// of some other account exists, and an unidentified session against a recorded owner is refused with a reason
    /// and deletes nothing.
    #[test]
    fn an_unidentified_session_is_refused_and_takes_no_cached_profile_for_its_own() {
        let local = local_data_of(Some(alice()));
        let fx = AuthorizeFixture::without_session();
        *fx.acct.session.lock().unwrap() = Some(Session {
            token: "tok-b".into(),
            master_key: [5u8; 32],
            email: None,
        });
        *fx.acct.auth_email.lock().unwrap() = None;
        *fx.acct.cached_profile.lock().unwrap() = Some(profile_of("u-a", "a@beebeeb.io"));
        let (_, identity) = keys_for_engine_start(&fx.acct).expect("keys");
        assert_eq!(identity, Identity::default(), "the profile is not borrowed");
        let slot = fx.acct.engine.try_lock().unwrap();
        let result = authorize_engine_start(
            &fx.state,
            &fx.acct,
            &slot,
            &LocalDataPaths::for_test(local.dir.path(), &local.staging),
        );
        assert_eq!(
            result.err(),
            Some(crate::account_binding::IDENTITY_UNKNOWN.to_string()),
            "an unidentified session against a recorded owner"
        );
        assert_eq!(local.data.db.owner().unwrap(), Some(alice()));
        assert_eq!(
            local.data.db.list_due_operations(i64::MAX).unwrap().len(),
            1,
            "nothing is deleted either"
        );
    }

    /// F1: the keys and the identity a start checks come from the same session, read together. The mirrored
    /// account fields (`auth_email`, `cached_profile`) are never where the identity comes from, so they cannot
    /// disagree with the keys.
    #[test]
    fn a_start_pairs_the_keys_with_the_identity_of_the_same_session() {
        let local = local_data_of(Some(alice()));
        let fx = AuthorizeFixture::with_session(&bob());
        // The mirrored fields describe another account; they play no part.
        *fx.acct.auth_email.lock().unwrap() = Some("a@beebeeb.io".into());
        *fx.acct.cached_profile.lock().unwrap() = Some(profile_of("u-a", "a@beebeeb.io"));
        let (_, identity) = keys_for_engine_start(&fx.acct).expect("keys");
        assert_eq!(
            identity,
            Identity::new(None, Some("b@beebeeb.io")),
            "the identity is the session's own"
        );
        let slot = fx.acct.engine.try_lock().unwrap();
        let result = authorize_engine_start(
            &fx.state,
            &fx.acct,
            &slot,
            &LocalDataPaths::for_test(local.dir.path(), &local.staging),
        );
        if RESET_ALLOWED {
            // On a Mac b's engine then waits for the previous account's Finder domain to be removed (F3).
            if cfg!(target_os = "macos") {
                assert!(
                    matches!(result, Ok(StartPermit::FinderRemovalOwed)),
                    "b waits for the removal: {result:?}"
                );
            } else {
                assert!(
                    matches!(result, Ok(StartPermit::Go(_))),
                    "b starts, on a clean slate: {result:?}"
                );
            }
            assert!(
                local.data.db.list_due_operations(i64::MAX).unwrap().is_empty(),
                "a's queue is not b's"
            );
            assert_eq!(
                local.data.db.owner().unwrap(),
                Some(Identity::new(None, Some("b@beebeeb.io")))
            );
        } else {
            assert_eq!(
                result.err(),
                Some(crate::account_binding::OTHER_ACCOUNT_ON_WINDOWS.to_string())
            );
            assert_eq!(
                local.data.db.list_due_operations(i64::MAX).unwrap().len(),
                1,
                "nothing discarded"
            );
        }
    }

    // ---- the log directory survives the reset (lead ruling 5) ----

    /// The R10 reset removes the previous account's files through the sign-out purge's one deletion gate, and
    /// that gate never reaches the lifecycle log directory, in the home or in the app's container.
    #[test]
    fn an_account_reset_never_deletes_the_lifecycle_log() {
        if !RESET_ALLOWED {
            return; // Windows refuses instead of resetting; nothing to delete.
        }
        let root = std::env::temp_dir().join(format!("bb-test-r10-logs-{}", uuid::Uuid::new_v4()));
        let logs = root.join("Library").join("Logs").join("Beebeeb");
        let container_logs = root
            .join("Library")
            .join("Containers")
            .join("io.beebeeb.app")
            .join("Data")
            .join("Library")
            .join("Logs")
            .join("Beebeeb");
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::create_dir_all(&container_logs).unwrap();
        let ordinary = root.join("ordinary-cache.bin");
        let (log, rotated) = (logs.join("lifecycle.log"), container_logs.join("lifecycle.log.1"));
        for path in [&ordinary, &log, &rotated] {
            std::fs::write(path, b"x").unwrap();
        }
        let dir = tempfile::tempdir().unwrap();
        let db = crate::state_db::StateDb::open(dir.path().join("state.db")).unwrap();
        for (n, path) in [&ordinary, &log, &rotated].into_iter().enumerate() {
            db.track_staged_payload(path.to_str().unwrap(), None, false).unwrap();
            db.enqueue_operation(&crate::state_db::PendingOperation {
                op_id: format!("op-{n}"),
                kind: crate::state_db::OperationKind::UploadVersion,
                file_id: None,
                parent_id: None,
                target_path: Some(format!("/{n}.txt")),
                metadata_json: None,
                payload_path: Some(path.to_str().unwrap().to_string()),
                base_version: None,
                base_object_version_id: None,
                attempts: 0,
                max_attempts: 5,
                next_retry_at: 0,
                last_error: None,
                backup_source_key: None,
                created_at: 1,
                updated_at: 1,
            })
            .unwrap();
        }
        db.set_owner(&alice()).unwrap();
        let data = StateDbLocalData {
            db,
            sync_root: None,
            staging_dirs: Vec::new(),
            kept: Default::default(),
        };
        assert_eq!(bind_data_to_session(&AppState::default(), &data, &bob()), Ok(()));
        let survived = (log.exists(), rotated.exists(), ordinary.exists());
        let cleared = !data.db.has_account_data().unwrap();
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(
            survived,
            (true, true, false),
            "the log directory survives; the ordinary file does not"
        );
        assert!(cleared, "and the account's rows are all gone");
    }

    /// On a Mac a reset asks the reconciler for a Repair, so Finder drops the old listing and gets Beebeeb back
    /// for the new account; the same account asks for nothing. The reconciler's end is a stand-in that only
    /// receives, and the call returns without waiting for an answer.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_reset_asks_the_reconciler_for_a_repair_and_a_kept_account_asks_for_nothing() {
        use finder_setup::driver::{Event, FinderSetupHandle, FinderSetupView};
        use finder_setup::launch_location::LaunchLocation;
        let (handle, mut rx) = FinderSetupHandle::for_test(FinderSetupView::initial(LaunchLocation::Applications));
        let state = AppState::default();
        let _ = state.finder_setup.set(handle);

        let kept = local_data_of(Some(alice()));
        assert_eq!(bind_data_to_session(&state, &kept.data, &alice()), Ok(()));
        assert!(rx.try_recv().is_err(), "the same account asks for no repair");

        let reset = local_data_of(Some(alice()));
        assert_eq!(bind_data_to_session(&state, &reset.data, &bob()), Ok(()));
        let event = rx.try_recv().expect("a reset asks the reconciler for something");
        assert!(
            matches!(event, Event::Trigger(finder_setup::core::Trigger::Repair)),
            "a Repair, and nothing else"
        );
        assert!(rx.try_recv().is_err(), "once");
    }

    /// F4: the account reset sweeps orphan staged files too (plaintext copies no row points at), through the
    /// staging directories its `StateDbLocalData` was given.
    #[test]
    fn an_account_reset_also_sweeps_orphan_staged_files() {
        if !RESET_ALLOWED {
            return;
        }
        let mut local = local_data_of(Some(alice()));
        let staging = std::env::temp_dir()
            .join(format!("bb-test-1834-f4-reset-{}", uuid::Uuid::new_v4()))
            .join("finder-writes");
        std::fs::create_dir_all(&staging).unwrap();
        let orphan = staging.join("orphan.bin");
        std::fs::write(&orphan, b"plaintext with no row").unwrap();
        local.data.staging_dirs = vec![staging.clone()];
        assert_eq!(bind_data_to_session(&AppState::default(), &local.data, &bob()), Ok(()));
        let swept = !orphan.exists();
        let _ = std::fs::remove_dir_all(staging.parent().unwrap());
        assert!(swept, "the orphan is gone");
    }

    /// F4 at the gate: an account reset that cannot remove a file starts nothing, leaves the owner and every row
    /// where they were, and the retry (once the file can go) starts the engine on a clean slate.
    #[test]
    fn an_account_reset_that_cannot_remove_a_file_starts_nothing_until_it_can() {
        if !RESET_ALLOWED {
            return; // Windows refuses before any purge
        }
        let local = local_data_of(Some(alice()));
        // The journal's staged copy, not the queued upload's: on a Mac a waiting write's copy that is not a regular
        // file fails the reset's keep step, before the purge (task 1887, `the_r10_reset_keeps_before_it_purges`).
        let journal_copy = local.files[2].clone();
        std::fs::remove_file(&journal_copy).unwrap();
        std::fs::create_dir(&journal_copy).unwrap(); // a removal that fails
        let fx = AuthorizeFixture::with_session(&bob());
        let slot = fx.acct.engine.try_lock().unwrap();
        let paths = LocalDataPaths::for_test(local.dir.path(), &local.staging);
        let error = authorize_engine_start(&fx.state, &fx.acct, &slot, &paths).expect_err("nothing starts");
        assert!(error.contains("could not be removed"), "{error}");
        assert_eq!(
            local.data.db.owner().unwrap(),
            Some(alice()),
            "the new account is not recorded over data still there"
        );
        assert_eq!(
            local.data.db.list_due_operations(i64::MAX).unwrap().len(),
            1,
            "the queue is still there"
        );
        std::fs::remove_dir(&journal_copy).unwrap();
        let retry = authorize_engine_start(&fx.state, &fx.acct, &slot, &paths);
        // On a Mac the engine then waits for the previous account's Finder domain to be removed (F3).
        if cfg!(target_os = "macos") {
            assert!(
                matches!(retry, Ok(StartPermit::FinderRemovalOwed)),
                "the retry resets, then waits for the removal: {retry:?}"
            );
        } else {
            assert!(matches!(retry, Ok(StartPermit::Go(_))), "the retry starts: {retry:?}");
        }
        assert!(local.data.db.list_due_operations(i64::MAX).unwrap().is_empty());
        assert_eq!(local.data.db.owner().unwrap(), Some(bob()));
    }

    /// F2: a database the engine start itself creates is not the upgrade's, so the adoption window is shut from
    /// the beginning: unowned data that shows up in it later is never adopted.
    #[test]
    fn a_database_the_engine_start_creates_never_opens_the_adoption_window() {
        let fx = AuthorizeFixture::with_session(&alice());
        let dir = tempfile::tempdir().unwrap();
        let slot = fx.acct.engine.try_lock().unwrap();
        assert!(matches!(
            authorize_after_removal(&fx, &slot, &LocalDataPaths::for_test(dir.path(), dir.path())),
            Ok(StartPermit::Go(_))
        ));
        let db = crate::state_db::StateDb::open(dir.path().join("state.db")).unwrap();
        db.clear_account_data(false).unwrap(); // no owner left ...
        rusqlite::Connection::open(dir.path().join("state.db"))
            .unwrap()
            .execute("INSERT INTO sync_state(key, value) VALUES ('cursor', '1')", []) // ... and unowned data
            .unwrap();
        assert!(
            !db.adopt_owner_once(Some(&bob())).unwrap(),
            "the window was shut when the database was created"
        );
        assert_eq!(db.owner().unwrap(), None);
    }

    /// F3: a reset on a Mac does not start the new account's engine in that call. It records a
    /// Finder-removal debt in the reset's own transaction, asks the reconciler for a Repair (never waiting for it),
    /// and every start refuses while the debt stands. A removal that FAILS releases nothing, so nothing starts and
    /// nothing is reset a second time; a confirmed removal releases it, and then the engine starts.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_reset_whose_removal_fails_keeps_the_new_accounts_engine_from_starting_until_a_removal_is_confirmed() {
        use finder_setup::driver::{Event, FinderSetupHandle, FinderSetupView};
        use finder_setup::launch_location::LaunchLocation;
        fn start(
            fx: &AuthorizeFixture,
            slot: &mut tokio::sync::MutexGuard<'_, Option<crate::runner::EngineRunner>>,
            paths: &LocalDataPaths,
            started: &std::cell::Cell<u32>,
        ) -> Result<EngineStart, String> {
            start_engine_bound(&fx.state, &fx.acct, slot, paths, |_root, _token, _key| {
                started.set(started.get() + 1);
                crate::runner::EngineRunner::for_test_with_task(tokio::spawn(async {}))
            })
        }
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let (handle, mut rx) = FinderSetupHandle::for_test(FinderSetupView::initial(LaunchLocation::Applications));
            let local = local_data_of(Some(alice()));
            let fx = AuthorizeFixture::with_session(&bob());
            let _ = fx.state.finder_setup.set(handle);
            let mut slot = fx.acct.engine.lock().await;
            let paths = LocalDataPaths::for_test(local.dir.path(), &local.staging);
            let started = std::cell::Cell::new(0);

            // 1. b's start: the reset runs, the debt is recorded, Repair is asked for, and no engine starts.
            assert_eq!(
                start(&fx, &mut slot, &paths, &started),
                Ok(EngineStart::FinderRemovalOwed)
            );
            assert_eq!(started.get(), 0);
            assert!(slot.is_none());
            assert!(
                local.data.db.finder_removal_owed().unwrap(),
                "the debt is recorded with the reset"
            );
            assert!(
                local.data.db.list_due_operations(i64::MAX).unwrap().is_empty(),
                "the reset itself went through"
            );
            assert_eq!(local.data.db.owner().unwrap(), Some(bob()));
            assert!(
                matches!(rx.try_recv(), Ok(Event::Trigger(finder_setup::core::Trigger::Repair))),
                "a Repair was asked for"
            );

            // 2. The removal failed: nothing released the debt. Still no engine, and no second reset or Repair.
            assert_eq!(
                start(&fx, &mut slot, &paths, &started),
                Ok(EngineStart::FinderRemovalOwed)
            );
            assert_eq!(started.get(), 0);
            assert!(rx.try_recv().is_err(), "the refusal asks for nothing more");
            assert_eq!(local.data.db.owner().unwrap(), Some(bob()));
            // 2b. While the debt stands, even a DIFFERENT account that signs in is refused before anything is bound:
            //     no second reset, and the owner is not touched.
            let bobs_session = fx.acct.session.lock().unwrap().take();
            *fx.acct.session.lock().unwrap() = Some(Session {
                token: "tok-a".into(),
                master_key: [4u8; 32],
                email: alice().email,
            });
            assert_eq!(
                start(&fx, &mut slot, &paths, &started),
                Ok(EngineStart::FinderRemovalOwed)
            );
            assert_eq!(
                local.data.db.owner().unwrap(),
                Some(bob()),
                "nothing was bound while the removal is owed"
            );
            *fx.acct.session.lock().unwrap() = bobs_session;

            // 3. A removal is confirmed (the reconciler's port does exactly this): the engine may start.
            clear_finder_removal_owed_in(local.dir.path());
            assert!(!local.data.db.finder_removal_owed().unwrap());
            assert_eq!(start(&fx, &mut slot, &paths, &started), Ok(EngineStart::Started));
            assert_eq!(started.get(), 1);
            assert!(slot.is_some());
        });
    }

    /// F3: the debt is read by the reconciler's facts (`finder_removal_owed_in`) and cleared by its port
    /// (`clear_finder_removal_owed_in`), through the same database the gate reads.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_reconcilers_view_of_the_removal_debt_is_the_gates() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!finder_removal_owed_in(dir.path()), "no database, no debt");
        let db = crate::state_db::StateDb::open(dir.path().join("state.db")).unwrap();
        assert!(!finder_removal_owed_in(dir.path()));
        db.set_finder_removal_owed(true).unwrap();
        assert!(finder_removal_owed_in(dir.path()));
        assert!(!db.has_account_data().unwrap(), "the debt is not account data");
        clear_finder_removal_owed_in(dir.path());
        assert!(!finder_removal_owed_in(dir.path()));
    }

    /// `purge` with the app's `desktop.toml` in a directory of its own. On a Mac a sign-out purge that completes clears
    /// the unfinished sign-out marker in the file `DesktopConfig::path()` names (spec 2026-10-10 W13): a test that calls
    /// the purge itself, outside the sign-out tests' purge gate, must never clear it in the sandbox those tests share.
    #[cfg(not(target_os = "windows"))]
    fn with_own_config<R>(purge: impl FnOnce() -> R) -> R {
        let config = tempfile::tempdir().unwrap();
        crate::config::CONFIG_DIR_OVERRIDE.sync_scope(config.path().to_path_buf(), purge)
    }

    /// The sign-out purge, behaviourally (macOS and Linux), through the real function `clear_session_impl` calls and
    /// a state dir of its own: it clears every account row and the owner, records a removal debt only when the
    /// sign-out's Finder removal was not confirmed, and a failure of any step stops it with the sign-out sentence.
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn the_sign_out_purge_clears_everything_and_records_the_debt_only_when_the_removal_was_not_confirmed() {
        for (owe, expected) in [(false, false), (true, true)] {
            let local = local_data_of(Some(alice()));
            rusqlite::Connection::open(local.dir.path().join("state.db"))
                .unwrap()
                .execute("INSERT INTO sync_state(key, value) VALUES ('cursor', '9')", [])
                .unwrap();
            assert_eq!(
                with_own_config(|| purge_local_data_for_sign_out(Ok(local.dir.path().to_path_buf()), None, owe, None)),
                Ok(())
            );
            assert!(!local.data.db.has_account_data().unwrap(), "no account row survives");
            assert_eq!(local.data.db.owner().unwrap(), None, "and nobody owns what is left");
            assert_eq!(
                local.data.db.finder_removal_owed().unwrap(),
                expected,
                "debt, owe = {owe}"
            );
            for path in &local.files {
                assert!(!path.exists(), "{} is gone", path.display());
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn the_sign_out_purge_stops_with_the_sign_out_sentence_when_any_step_fails() {
        // A file that cannot be removed.
        let local = local_data_of(Some(alice()));
        std::fs::remove_file(&local.files[0]).unwrap();
        std::fs::create_dir(&local.files[0]).unwrap();
        let error = purge_local_data_for_sign_out(Ok(local.dir.path().to_path_buf()), None, false, None).unwrap_err();
        assert!(
            error.starts_with(SIGN_OUT_PURGE_FAILED) && error.contains("could not be removed"),
            "{error}"
        );
        assert_eq!(local.data.db.owner().unwrap(), Some(alice()), "nothing was cleared");
        assert_eq!(local.data.db.list_due_operations(i64::MAX).unwrap().len(), 1);
        // A database that cannot be opened, and a state dir that cannot be resolved.
        let broken = tempfile::tempdir().unwrap();
        std::fs::write(
            broken.path().join("state.db"),
            b"this is not a database, and it is long enough to be read as one: 0123456789",
        )
        .unwrap();
        assert!(
            purge_local_data_for_sign_out(Ok(broken.path().to_path_buf()), None, false, None)
                .unwrap_err()
                .starts_with(SIGN_OUT_PURGE_FAILED)
        );
        assert!(
            purge_local_data_for_sign_out(Err("not initialised".into()), None, false, None)
                .unwrap_err()
                .starts_with(SIGN_OUT_PURGE_FAILED)
        );
        // No database yet is a fresh install: nothing to purge.
        let fresh = tempfile::tempdir().unwrap();
        assert_eq!(
            purge_local_data_for_sign_out(Ok(fresh.path().to_path_buf()), None, true, None),
            Ok(())
        );
    }

    /// F3, shape: the gate refuses while a removal is owed, before it binds anything and again after a reset; and the
    /// reset records the debt in its own transaction (a Mac only).
    #[test]
    fn the_gate_refuses_while_a_finder_removal_is_owed_and_a_reset_records_the_debt() {
        let production = production_source();
        let bind = code_of(&production, "fn bind_local_data_to_session(");
        let first = bind.find("finder_removal_owed()").expect("it reads the debt");
        let bound = bind.find("bind_data_to_session(").expect("it binds");
        let second = bind.rfind("finder_removal_owed()").unwrap();
        assert!(
            first < bound && bound < second,
            "the debt is read before the binding and again after it:\n{bind}"
        );
        assert_eq!(
            bind.matches("return Ok(BoundData::FinderRemovalOwed)").count(),
            2,
            "{bind}"
        );
        let reset = body_between(&production, "    fn reset(&self)", "\n    }\n");
        assert!(
            reset.contains("clear_account_data(cfg!(target_os = \"macos\"))"),
            "the reset owes the removal on a Mac:\n{reset}"
        );
        let bound = body_between(&production, "fn start_engine_bound(", "\n}\n");
        assert!(
            bound.contains("StartPermit::FinderRemovalOwed => return Ok(EngineStart::FinderRemovalOwed)"),
            "{bound}"
        );
    }

    /// This is a public repository: the comments in the account-binding code, the reconciler's core and the state
    /// database say what the code guarantees, and never announce that something is unresolved. (The needles
    /// are generic and split so this test does not match itself.)
    #[test]
    fn no_public_comment_announces_something_still_open() {
        let needles = [
            concat!("is still", " open"),
            concat!("not", " closed"),
            concat!("un", "mitigated"),
            concat!("known", " weakness"),
            concat!("un", "fixed"),
        ];
        let strip_tests = |source: &str| {
            source[..source.find("#[cfg(test)]\nmod tests {").unwrap_or(source.len())].to_ascii_lowercase()
        };
        let sources = [
            ("lib.rs", production_source().to_ascii_lowercase()),
            (
                "finder_setup/core.rs",
                strip_tests(&include_str!("finder_setup/core.rs").replace("\r\n", "\n")),
            ),
            (
                "account_binding.rs",
                strip_tests(&include_str!("account_binding.rs").replace("\r\n", "\n")),
            ),
            (
                "state_db.rs",
                strip_tests(&include_str!("state_db.rs").replace("\r\n", "\n")),
            ),
        ];
        for (name, source) in &sources {
            for needle in needles {
                assert!(!source.contains(needle), "{name} says {needle:?}");
            }
        }
    }

    /// R1 (fix round 2): the adoption window shuts on EVERY way out of the startup restore, not only on the paths
    /// that reach the adoption call: a Windows restore that is refused, a restore with no account, a session
    /// already installed, a Keychain with nothing in it. The restore is a thin wrapper that shuts the window after
    /// the real body has returned (by whatever return), and the real body adopts on what it loaded. The restore
    /// needs a real window system, so this is a source pin; the window's behaviour is the state_db and gate tests.
    #[test]
    fn every_return_of_the_startup_restore_shuts_the_adoption_window() {
        let production = production_source();
        let wrapper = code_of(&production, "async fn restore_session_on_startup(");
        let statements: Vec<&str> = wrapper
            .lines()
            .skip(1)
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect();
        assert_eq!(
            statements,
            [
                "restore_session_inner(app).await;",
                "adopt_unbound_local_data_at_startup(None);"
            ],
            "the wrapper is exactly: run the restore, then shut the window, with nothing that can skip it:\n{wrapper}"
        );
        let body = code_of(&production, "async fn restore_session_inner(");
        assert!(
            body.contains("return;"),
            "the real restore has early returns (that is the point of the wrapper)"
        );
    }

    /// R1: an engine start shuts a window that is still open, on a database that already existed (the startup pass
    /// could not run, say because the state dir was not resolved yet). Unowned data that appears later is then
    /// never adopted.
    #[test]
    fn an_engine_start_shuts_an_open_adoption_window_on_an_existing_database() {
        let local = local_data_of(None); // a database from before the upgrade: unowned rows, window open
        let fx = AuthorizeFixture::with_session(&alice());
        let slot = fx.acct.engine.try_lock().unwrap();
        let _ = authorize_engine_start(
            &fx.state,
            &fx.acct,
            &slot,
            &LocalDataPaths::for_test(local.dir.path(), &local.staging),
        );
        local.data.db.clear_account_data(false).unwrap();
        rusqlite::Connection::open(local.dir.path().join("state.db"))
            .unwrap()
            .execute("INSERT INTO sync_state(key, value) VALUES ('cursor', '1')", [])
            .unwrap();
        assert!(
            !local.data.db.adopt_owner_once(Some(&bob())).unwrap(),
            "the engine start shut the window"
        );
        assert_eq!(local.data.db.owner().unwrap(), None);
    }

    /// A signed-in account the way a sign-in builds it: the session email is `session_email(profile)` (the server's
    /// own record and nothing typed) and the profile is cached, exactly what `desktop_login`, `desktop_login_2fa`
    /// and `apply_session` do. `None`: the sign-in's account-record fetch failed, so the session has no identity.
    fn signed_in_via(profile: Option<&account_dto::AccountProfile>) -> AuthorizeFixture {
        let fx = AuthorizeFixture::without_session();
        let email = session_email(profile);
        *fx.acct.session.lock().unwrap() = Some(Session {
            token: "tok-r10".into(),
            master_key: [7u8; 32],
            email: email.clone(),
        });
        *fx.acct.auth_email.lock().unwrap() = email;
        *fx.acct.cached_profile.lock().unwrap() = profile.cloned();
        fx
    }

    /// An HTTP server on localhost that answers every request with `status_line` and `body`; the base URL.
    fn answering(status_line: &'static str, body: String) -> String {
        answering_after(status_line, body, || {})
    }

    /// [`answering`], running `before_reply` after the request arrived and before the answer is sent: a change to
    /// the world that happens WHILE a fetch is in flight.
    fn answering_after(status_line: &'static str, body: String, before_reply: impl Fn() + Send + 'static) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                before_reply();
                let response = format!(
                    "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        base_url
    }

    /// R2 (fix round 2; round 3): the email a session is known by is the server's own spelling of it, and nothing
    /// else. Typed text is not a parameter any more, so a sign-in that has no account record has no email.
    #[test]
    fn the_session_email_is_the_servers_spelling_and_nothing_else() {
        let profile = profile_of("u-1", "sam@beebeeb.io");
        assert_eq!(session_email(Some(&profile)).as_deref(), Some("sam@beebeeb.io"));
        assert_eq!(
            session_email(Some(&profile_of("u-1", "  sam@beebeeb.io "))).as_deref(),
            Some("sam@beebeeb.io"),
            "trimmed"
        );
        assert_eq!(
            session_email(Some(&profile_of("u-1", "  "))),
            None,
            "an empty email names nobody"
        );
        assert_eq!(session_email(None), None, "no account record: no email");
    }

    /// R2: one account is ONE owner whichever way it signs in, because the identity is the server's record and
    /// never the text a sign-in was handed. The account `sam@x` (user id u-1) owns the data; its sign-in starts
    /// without purging its own unsynced queue. The server's other account `Sam@x` (a different user id) is another
    /// owner (the ids decide, whatever the emails say).
    #[test]
    fn one_account_is_one_owner_whichever_way_it_signs_in_and_case_twins_stay_apart() {
        let owner = Identity::new(Some("u-1"), Some("sam@beebeeb.io"));
        let server = profile_of("u-1", "sam@beebeeb.io");
        let fx = signed_in_via(Some(&server));
        let local = local_data_of(Some(owner.clone()));
        let slot = fx.acct.engine.try_lock().unwrap();
        let result = authorize_engine_start(
            &fx.state,
            &fx.acct,
            &slot,
            &LocalDataPaths::for_test(local.dir.path(), &local.staging),
        );
        assert!(
            matches!(result, Ok(StartPermit::Go(_))),
            "the same account starts: {result:?}"
        );
        assert_eq!(
            local.data.db.list_due_operations(i64::MAX).unwrap().len(),
            1,
            "its own queue is never purged"
        );
        assert_eq!(local.data.db.owner().unwrap(), Some(owner.clone()), "still one owner");
        for path in &local.files {
            assert!(path.exists(), "{} is kept", path.display());
        }
        // The server's other account `Sam@x` (a different user id): another owner.
        let twin = signed_in_via(Some(&profile_of("u-2", "Sam@beebeeb.io")));
        let local = local_data_of(Some(owner));
        let slot = twin.acct.engine.try_lock().unwrap();
        let result = authorize_engine_start(
            &twin.state,
            &twin.acct,
            &slot,
            &LocalDataPaths::for_test(local.dir.path(), &local.staging),
        );
        if RESET_ALLOWED {
            assert!(
                !matches!(result, Ok(StartPermit::Go(_)) if !local.data.db.list_due_operations(i64::MAX).unwrap().is_empty()),
                "{result:?}"
            );
            assert!(
                local.data.db.list_due_operations(i64::MAX).unwrap().is_empty(),
                "the twin does not inherit the queue"
            );
        } else {
            assert_eq!(
                result.err(),
                Some(crate::account_binding::OTHER_ACCOUNT_ON_WINDOWS.to_string())
            );
        }
    }

    /// R2, wiring: every sign-in names the session by the server's profile and by nothing it was handed, and the
    /// browser handoff fetches the profile and passes NO fallback email (round 3: a failed fetch leaves the session
    /// unidentified instead of falling back to the web client's text).
    #[test]
    fn every_sign_in_names_the_session_by_the_servers_email() {
        let production = production_source();
        for signature in ["async fn desktop_login(", "async fn desktop_login_2fa("] {
            let code = code_of(&production, signature);
            let canonical = code
                .find("session_email(Some(&profile))")
                .unwrap_or_else(|| panic!("{signature}: the profile names the email"));
            // The token, the profile and the flags are written in the sign-in's turn, by `store_first_sign_in` (Task 12).
            let stored = code
                .find("store_first_sign_in(&state, &acct, &mut turn, &session_token, &email, profile)")
                .expect("the profile is cached by the checked store");
            assert!(
                canonical < stored,
                "{signature}: the email is read from the profile BEFORE it is moved into the cache, and the Keychain gets it"
            );
        }
        let store = code_of(&production, "fn store_first_sign_in(");
        assert!(
            store
                .find("persist_session_token_to_keychain(&write, acct.id.as_str(), token, Some(email))?;")
                .unwrap()
                < store.find("*guard = Some(profile);").unwrap(),
            "{store}"
        );
        let apply = code_of(&production, "pub(crate) async fn apply_session(");
        assert!(
            apply.contains("let email = session_email(profile.as_ref());"),
            "{apply}"
        );
        assert!(
            apply.find("session_email(").unwrap() < apply.find("install_new_session_or_revoke(").unwrap(),
            "the server's email reaches the checked install"
        );
        assert!(
            code_of(&production, "async fn install_new_session_or_revoke(")
                .contains("install_new_session(state, acct, turn, token, master_key, email, profile)?")
        );
        assert!(
            code_of(&production, "fn install_new_session(").contains(
                "persist_session_to_keychain(&write, acct.id.as_str(), token, master_key, email.as_deref())?;"
            )
        );
        let signature = apply[..apply.find(") -> Result<(), String> {").unwrap()].to_string();
        assert!(
            !signature.contains("email"),
            "apply_session takes no email, only the server's profile:\n{signature}"
        );
        let browser = include_str!("browser_login.rs");
        assert!(
            browser.contains("crate::fetch_canonical_profile(&creds.session_token).await"),
            "the browser handoff fetches the profile"
        );
        assert!(
            browser.contains("let email = crate::session_email(profile.as_ref());"),
            "and names the email from it alone"
        );
        assert!(!browser.contains("creds.email"), "the browser's own email is not read");
    }

    /// Fix round 3 (R2 residual): a sign-in whose account-record fetch FAILED has no identity, and an unidentified
    /// session starts nothing and changes nothing: no engine, no purge, no adoption, no owner recorded, no Finder
    /// debt, no database opened. Whatever the local data is: another account's, unowned, or none.
    #[tokio::test]
    async fn an_unidentified_session_starts_nothing_and_changes_nothing() {
        let unidentified = signed_in_via(None); // what `fetch_canonical_profile` -> None leaves behind
        assert!(
            unidentified
                .acct
                .session
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|session| session.email.is_none())
        );
        for (name, owner) in [
            ("another account's data", Some(alice())),
            ("data with no recorded owner", None),
        ] {
            let local = local_data_of(owner.clone());
            let mut slot = unidentified.acct.engine.try_lock().unwrap();
            let paths = LocalDataPaths::for_test(local.dir.path(), &local.staging);
            let created = std::cell::Cell::new(false);
            let result = start_engine_bound(
                &unidentified.state,
                &unidentified.acct,
                &mut slot,
                &paths,
                |_root, _token, _key| {
                    created.set(true);
                    crate::runner::EngineRunner::for_test_with_task(tokio::spawn(async {}))
                },
            );
            assert_eq!(
                result,
                Err(crate::account_binding::IDENTITY_UNKNOWN.to_string()),
                "{name}: the existing identity message"
            );
            assert!(!created.get() && slot.is_none(), "{name}: no engine");
            assert_eq!(
                local.data.db.list_due_operations(i64::MAX).unwrap().len(),
                1,
                "{name}: nothing purged"
            );
            for path in &local.files {
                assert!(path.exists(), "{name}: {} is kept", path.display());
            }
            assert_eq!(
                local.data.db.owner().unwrap(),
                owner,
                "{name}: no owner recorded, none adopted, none replaced"
            );
            assert!(!local.data.db.finder_removal_owed().unwrap(), "{name}: no Finder debt");
        }
        // No data at all: the database is not even created.
        let dir = tempfile::tempdir().unwrap();
        let mut slot = unidentified.acct.engine.try_lock().unwrap();
        let result = start_engine_bound(
            &unidentified.state,
            &unidentified.acct,
            &mut slot,
            &LocalDataPaths::for_test(dir.path(), dir.path()),
            |_root, _token, _key| panic!("no engine for an unidentified session"),
        );
        assert_eq!(result, Err(crate::account_binding::IDENTITY_UNKNOWN.to_string()));
        assert!(
            !dir.path().join("state.db").exists(),
            "an unidentified start opens nothing"
        );
    }

    /// Fix round 3: the fetch fails, nothing happens; a later fetch that succeeds names the session, and the start
    /// then binds exactly as any other (the same account keeps its queue, another account's is reset).
    #[tokio::test]
    async fn a_later_fetch_that_succeeds_names_the_session_and_the_start_binds_normally() {
        for (name, profile, is_the_owner) in [
            ("the owner", profile_of("u-a", "a@beebeeb.io"), true),
            ("another account", profile_of("u-b", "b@beebeeb.io"), false),
        ] {
            let local = local_data_of(Some(alice()));
            let fx = signed_in_via(None);
            let paths = LocalDataPaths::for_test(local.dir.path(), &local.staging);
            // The account record cannot be fetched (the server errors, or answers with something that is no
            // profile): still unidentified, still refused, nothing deleted.
            for (status, body) in [("500 Internal Server Error", "{}"), ("200 OK", "{}")] {
                let base_url = answering(status, body.to_string());
                assert!(
                    !establish_session_identity(&fx.state, &fx.acct, &base_url, None).await,
                    "{name}: {status}"
                );
                assert_eq!(
                    fx.acct.session.lock().unwrap().as_ref().unwrap().email,
                    None,
                    "{name}: still unidentified"
                );
                let slot = fx.acct.engine.try_lock().unwrap();
                let refused = authorize_engine_start(&fx.state, &fx.acct, &slot, &paths);
                assert_eq!(
                    refused.err(),
                    Some(crate::account_binding::IDENTITY_UNKNOWN.to_string()),
                    "{name}: {status}"
                );
                assert_eq!(
                    local.data.db.list_due_operations(i64::MAX).unwrap().len(),
                    1,
                    "{name}: nothing purged"
                );
                assert_eq!(
                    local.data.db.owner().unwrap(),
                    Some(alice()),
                    "{name}: the owner is untouched"
                );
            }
            // A later fetch succeeds.
            let base_url = answering("200 OK", serde_json::to_string(&profile).unwrap());
            assert!(
                establish_session_identity(&fx.state, &fx.acct, &base_url, None).await,
                "{name}"
            );
            assert_eq!(
                fx.acct.session.lock().unwrap().as_ref().unwrap().email.as_deref(),
                Some(profile.email.as_str()),
                "{name}: named by the server"
            );
            assert_eq!(
                fx.acct
                    .cached_profile
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map(|cached| cached.user_id.clone()),
                Some(profile.user_id.clone())
            );
            assert_eq!(
                fx.acct.auth_email.lock().unwrap().as_deref(),
                Some(profile.email.as_str()),
                "{name}: the Account page shows it"
            );
            let slot = fx.acct.engine.try_lock().unwrap();
            let result = authorize_after_removal(&fx, &slot, &paths);
            if is_the_owner {
                assert!(matches!(result, Ok(StartPermit::Go(_))), "{name}: {result:?}");
                assert_eq!(
                    local.data.db.list_due_operations(i64::MAX).unwrap().len(),
                    1,
                    "{name}: its own queue is kept"
                );
                assert_eq!(local.data.db.owner().unwrap(), Some(alice()));
            } else if RESET_ALLOWED {
                assert!(matches!(result, Ok(StartPermit::Go(_))), "{name}: {result:?}");
                assert!(
                    local.data.db.list_due_operations(i64::MAX).unwrap().is_empty(),
                    "{name}: another account's queue is reset"
                );
                assert_eq!(local.data.db.owner().unwrap(), Some(bob()));
            } else {
                assert_eq!(
                    result.err(),
                    Some(crate::account_binding::OTHER_ACCOUNT_ON_WINDOWS.to_string())
                );
            }
            // A session that is named already, or was replaced meanwhile, is left alone.
            assert!(
                !establish_session_identity(&fx.state, &fx.acct, &base_url, None).await,
                "{name}: already named"
            );
        }
    }

    /// A signed-in session whose stored email is a legacy spelling (`Sam@Beebeeb.io`, as an older build saved it) and the
    /// local data it owns under that same spelling, with no id recorded; the server spells the address `sam@beebeeb.io`.
    fn a_legacy_spelling() -> (AuthorizeFixture, LocalFixture, account_dto::AccountProfile) {
        let legacy = Identity::new(None, Some("Sam@Beebeeb.io"));
        let fx = AuthorizeFixture::with_session(&legacy);
        AuthVault::new(platform_keychain_store_for(fx.acct.id.as_str()))
            .store_account_email("Sam@Beebeeb.io")
            .ok();
        (fx, local_data_of(Some(legacy)), profile_of("u-1", "sam@beebeeb.io"))
    }

    /// Ruling A″ (fix round 3): the owner backfill and the naming compare in the engine start's own canonical form, so a
    /// legacy spelling that differs from the server's only in a NON-ASCII letter case (`Émile@…`, `émile@…`) is
    /// completed and named like an ASCII variant. Both folded ASCII letters only before.
    #[test]
    fn a_non_ascii_legacy_spelling_is_completed_and_named() {
        let legacy = Identity::new(None, Some("Émile@beebeeb.io"));
        let fx = AuthorizeFixture::with_session(&legacy);
        let local = local_data_of(Some(legacy));
        let server = profile_of("u-1", "émile@beebeeb.io");
        assert_eq!(
            backfill_local_data_owner(&SessionWrite::for_test(), local.dir.path(), &server),
            Ok(true),
            "the owner is completed"
        );
        assert_eq!(
            local.data.db.owner().unwrap(),
            Some(Identity::new(Some("u-1"), Some("émile@beebeeb.io")))
        );
        let named = name_session_after_profile(
            &SessionWrite::for_test(),
            &fx.state,
            &fx.acct,
            "tok-r10",
            &server,
            Some(local.dir.path()),
        );
        assert!(named, "the session is named after the server's spelling");
        assert_eq!(session_spelling(&fx).as_deref(), Some("émile@beebeeb.io"));
        assert_eq!(
            local.data.db.list_due_operations(i64::MAX).unwrap().len(),
            1,
            "its queued change is kept"
        );
    }

    fn session_spelling(fx: &AuthorizeFixture) -> Option<String> {
        fx.acct
            .session
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|session| session.email.clone())
    }

    /// Fix round 1, item 1 (review I1, P3-A): the session is renamed to the server's spelling only after the
    /// owner record was completed. When that fails (here the state dir holds a `state.db` that is not a database, so it
    /// cannot be read, on every platform), nothing is renamed: the session, the Keychain email and the cache stay as
    /// they were, and the next engine start still knows the account's own data (its queued change is kept, nothing is
    /// reset).
    #[test]
    fn a_failed_owner_backfill_renames_nothing_and_the_next_start_keeps_the_data() {
        let (fx, local, server) = a_legacy_spelling();
        let keychain_before = keychain_account_email(fx.acct.id.as_str());
        let unreadable = tempfile::tempdir().unwrap();
        std::fs::write(
            unreadable.path().join(crate::state_paths::STATE_DB_FILENAME),
            b"this is not a database, and it is long enough to be read as one: 0123456789",
        )
        .unwrap();
        assert!(
            state_db_from_state_dir(unreadable.path()).is_err(),
            "the premise: state.db cannot be read"
        );
        let named = name_session_after_profile(
            &SessionWrite::for_test(),
            &fx.state,
            &fx.acct,
            "tok-r10",
            &server,
            Some(unreadable.path()),
        );
        assert!(!named, "a failed backfill names nothing");
        assert_eq!(
            session_spelling(&fx).as_deref(),
            Some("Sam@Beebeeb.io"),
            "the stored spelling stays"
        );
        assert_eq!(
            keychain_account_email(fx.acct.id.as_str()),
            keychain_before,
            "and so does the Keychain's"
        );
        assert!(fx.acct.cached_profile.lock().unwrap().is_none(), "nothing is cached");
        let session = identity_of_session(
            session_spelling(&fx).as_deref(),
            fx.acct.cached_profile.lock().unwrap().as_ref(),
        );
        assert_eq!(
            bind_before_engine_start(&local.data, &session, RESET_ALLOWED),
            Ok(Bound::Kept),
            "the next start keeps the data"
        );
        assert_eq!(
            local.data.db.list_due_operations(i64::MAX).unwrap().len(),
            1,
            "its queued change is still there"
        );
    }

    /// Fix round 2, item 3 (re-check N2): the naming completed the owner record (its id and the server's
    /// spelling) but could not write the Keychain email, and the next launch is OFFLINE: the session comes back from the
    /// Keychain with the legacy spelling and no id. The engine-start binding must not take that for another account:
    /// nothing of the account's own data is reset. Since ruling A″ the spelling IS the same account (equal in canonical
    /// form), so the offline start proceeds and keeps everything; under rulings A and A′ it was refused until a fetch.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_stale_keychain_spelling_after_a_completed_owner_never_resets_the_data_on_an_offline_launch() {
        let (fx, local, server) = a_legacy_spelling();
        let id = fx.acct.id.as_str().to_string();
        // The naming in the run before: the backfill succeeds, the Keychain email write fails.
        crate::keychain::test_fail_account_email_writes(Some(&id), true);
        let named = name_session_after_profile(
            &SessionWrite::for_test(),
            &fx.state,
            &fx.acct,
            "tok-r10",
            &server,
            Some(local.dir.path()),
        );
        crate::keychain::test_fail_account_email_writes(Some(&id), false);
        assert!(named, "the premise: the backfill succeeded, so the session was named");
        assert_eq!(
            local.data.db.owner().unwrap(),
            Some(Identity::new(Some("u-1"), Some("sam@beebeeb.io"))),
            "the premise: the owner record was completed"
        );
        assert_eq!(
            keychain_account_email(&id).as_deref(),
            Some("Sam@Beebeeb.io"),
            "the premise: the Keychain write failed"
        );
        // The offline launch: the session comes back from the Keychain, no profile can be fetched.
        let restored = identity_of_session(keychain_account_email(&id).as_deref(), None);
        assert_eq!(
            restored,
            Identity::new(None, Some("Sam@Beebeeb.io")),
            "the premise: the stale spelling, no id"
        );
        assert_eq!(
            bind_before_engine_start(&local.data, &restored, RESET_ALLOWED),
            Ok(Bound::Kept),
            "the same account starts offline; its own data is never reset for a spelling"
        );
        assert_eq!(
            local.data.db.list_due_operations(i64::MAX).unwrap().len(),
            1,
            "its queued change is still there"
        );
        assert_eq!(
            local.data.db.owner().unwrap().and_then(|owner| owner.user_id),
            Some("u-1".to_string()),
            "and its owner record keeps the account's id"
        );
        // Once a fetch names the session (the cached profile carries the id), the ids decide and the data is kept.
        let named_again = identity_of_session(Some("sam@beebeeb.io"), Some(&server));
        assert_eq!(
            bind_before_engine_start(&local.data, &named_again, RESET_ALLOWED),
            Ok(Bound::Kept)
        );
        assert_eq!(
            local.data.db.list_due_operations(i64::MAX).unwrap().len(),
            1,
            "still there"
        );
    }

    /// Ruling P (Task 12 fix round 2): a session that does not know who it is (its account record could not be fetched)
    /// is refused, and `sync_status` carries the closed code and its sentence and nothing else. The later naming start
    /// (the session named after the server's record and its profile cached, as `name_session_after_profile` leaves it)
    /// runs and clears it. Fix round 3: this used a case-only spelling, which ruling A″ makes the same account.
    #[test]
    fn an_unidentified_sessions_refusal_shows_in_sync_status_until_the_naming_start_runs() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let local = local_data_of(Some(Identity::new(Some("u-1"), Some("sam@beebeeb.io"))));
            let fx = AuthorizeFixture::with_session(&Identity::default());
            let paths = LocalDataPaths::for_test(local.dir.path(), &local.staging);
            let mut slot = fx.acct.engine.lock().await;
            let refused = start_engine_bound(&fx.state, &fx.acct, &mut slot, &paths, |_root, _token, _key| {
                unreachable!("no engine")
            });
            assert_eq!(
                refused,
                Err(crate::account_binding::IDENTITY_UNKNOWN.to_string()),
                "the premise: the start is refused"
            );
            let shown = engine_refusal_view(&fx.acct);
            assert_eq!(
                shown,
                serde_json::json!({ "code": "identity_unknown", "sentence": crate::account_binding::IDENTITY_UNKNOWN })
            );
            assert_eq!(
                shown.as_object().unwrap().len(),
                2,
                "the code and the sentence, nothing about the account"
            );
            fx.acct.session.lock().unwrap().as_mut().unwrap().email = Some("sam@beebeeb.io".into());
            *fx.acct.cached_profile.lock().unwrap() = Some(profile_of("u-1", "sam@beebeeb.io"));
            let started = start_engine_bound(&fx.state, &fx.acct, &mut slot, &paths, |_root, _token, _key| {
                crate::runner::EngineRunner::for_test_with_task(tokio::spawn(async {}))
            });
            assert_eq!(started, Ok(EngineStart::Started), "the premise: the naming start runs");
            assert_eq!(
                engine_refusal_view(&fx.acct),
                serde_json::Value::Null,
                "and clears the refusal"
            );
            assert_eq!(
                local.data.db.list_due_operations(i64::MAX).unwrap().len(),
                1,
                "the account's own change is kept"
            );
        });
    }

    /// Fix round 3, M5: a Lock and a Sign-out drop a Keychain email write still owed (it belonged to the session they
    /// end); a Write keeps it, because the naming that marks a failed write moves the generation with a Write.
    #[test]
    fn a_lock_and_a_sign_out_drop_a_keychain_email_write_still_owed_and_a_write_keeps_it() {
        use crate::account::SessionTransition;
        let fx = AuthorizeFixture::with_session(&alice());
        for (by, kept) in [
            (SessionTransition::Lock, false),
            (SessionTransition::SignOut, false),
            (SessionTransition::Write, true),
        ] {
            fx.acct.keychain_email_pending.store(true, Ordering::SeqCst);
            let _turn = SessionWrite::for_test();
            fx.acct.advance_session_generation_holding_session_write_lock(by);
            assert_eq!(fx.acct.keychain_email_pending.load(Ordering::SeqCst), kept, "{by:?}");
        }
    }

    /// Ruling P: an engine-start error that is not one of the two binding refusals records nothing (here the slot is
    /// taken), and only the exact two sentences count as refusals.
    #[test]
    fn an_engine_start_error_that_is_not_a_binding_refusal_records_nothing() {
        use crate::account_binding::{IDENTITY_UNKNOWN, OTHER_ACCOUNT_ON_WINDOWS, Refusal};
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let local = local_data_of(Some(alice()));
            let fx = AuthorizeFixture::with_session(&alice());
            let paths = LocalDataPaths::for_test(local.dir.path(), &local.staging);
            let mut slot = fx.acct.engine.lock().await;
            *slot = Some(crate::runner::EngineRunner::for_test_with_task(tokio::spawn(async {})));
            let refused = start_engine_bound(&fx.state, &fx.acct, &mut slot, &paths, |_root, _token, _key| {
                unreachable!("no engine")
            });
            assert_eq!(
                refused,
                Err(ENGINE_SLOT_OCCUPIED_ERROR.to_string()),
                "the premise: another error"
            );
            assert_eq!(
                engine_refusal_view(&fx.acct),
                serde_json::Value::Null,
                "recorded nothing"
            );
        });
        assert_eq!(Refusal::of(IDENTITY_UNKNOWN), Some(Refusal::IdentityUnknown));
        assert_eq!(
            Refusal::of(OTHER_ACCOUNT_ON_WINDOWS),
            Some(Refusal::OtherAccountOnWindows)
        );
        assert_eq!(Refusal::of("read the local data owner: disk I/O error"), None);
        assert_eq!(
            Refusal::of(&format!("{IDENTITY_UNKNOWN} (detail)")),
            None,
            "an exact sentence only"
        );
        assert_eq!(Refusal::of(""), None);
    }

    /// Ruling P: every session transition clears the refusal, and a refusal found before a transition is not recorded
    /// after it (the start checks the generation it read, under the same mutex the transition clears).
    #[test]
    fn every_session_transition_clears_the_engine_refusal_and_a_stale_one_is_not_recorded() {
        use crate::account::SessionTransition;
        use crate::account_binding::{IDENTITY_UNKNOWN, Refusal};
        let fx = AuthorizeFixture::with_session(&alice());
        for by in [
            SessionTransition::Write,
            SessionTransition::Lock,
            SessionTransition::SignOut,
        ] {
            *fx.acct.engine_refusal.lock().unwrap() = Some(Refusal::IdentityUnknown);
            let _turn = SessionWrite::for_test();
            fx.acct.advance_session_generation_holding_session_write_lock(by);
            assert_eq!(
                engine_refusal_view(&fx.acct),
                serde_json::Value::Null,
                "{by:?} cleared it"
            );
        }
        let before = fx.acct.session_generation();
        {
            let _turn = SessionWrite::for_test();
            fx.acct
                .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
        }
        record_engine_refusal(&fx.acct, before, IDENTITY_UNKNOWN);
        assert_eq!(
            engine_refusal_view(&fx.acct),
            serde_json::Value::Null,
            "found before the transition: not recorded"
        );
        record_engine_refusal(&fx.acct, fx.acct.session_generation(), IDENTITY_UNKNOWN);
        assert_eq!(
            engine_refusal_view(&fx.acct)["code"],
            "identity_unknown",
            "found for the current session: recorded"
        );
    }

    /// Local data with an owner and a file row (which writes the Finder change log too), nothing waiting to upload:
    /// the state the Windows sign-out sees for an account that synced and finished.
    fn a_synced_account(owner: &Identity) -> (tempfile::TempDir, StateDbLocalData) {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::state_db::StateDb::open(dir.path().join("state.db")).unwrap();
        db.upsert_file(&crate::state_db::FileEntry {
            file_id: "file-a".into(),
            path: "/A.txt".into(),
            status: crate::state_db::FileStatus::Local,
            size_bytes: 11,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: crate::state_db::ItemKind::File,
        })
        .unwrap();
        db.set_owner(owner).unwrap();
        (
            dir,
            StateDbLocalData {
                db,
                sync_root: None,
                staging_dirs: Vec::new(),
                kept: Default::default(),
            },
        )
    }

    /// The database steps of the Windows sign-out (`windows_cf::signout::purge`; its file handling is Windows-only).
    fn windows_sign_out(data: &StateDbLocalData) {
        data.db
            .windows_signout_preflight()
            .expect("nothing waits to upload, so the sign-out goes ahead");
        data.db.finish_windows_signout().expect("and finishes");
    }

    /// Fix to the R10 binding (ruling W1): after a Windows sign-out of an account that had a file row, nothing of that
    /// account counts as local data any more, so another account's sign-in starts instead of being refused as if the
    /// previous account's data were still here.
    #[test]
    fn after_a_windows_sign_out_another_account_starts() {
        let (_dir, data) = a_synced_account(&alice());
        windows_sign_out(&data);
        assert!(!data.db.has_account_data().unwrap(), "no account data is left");
        assert_eq!(data.db.owner().unwrap(), None, "and no owner");
        assert_eq!(
            bind_before_engine_start(&data, &bob(), false),
            Ok(Bound::Kept),
            "another account's sign-in starts"
        );
        assert_eq!(
            data.db.owner().unwrap(),
            Some(bob()),
            "and owns what it syncs from now on"
        );
    }

    /// Ruling A″ (fix round 3): on Windows a letter-case variant of the owner's email starts without a sign-out, as it
    /// did before the binding existed (ruling A′ refused it with "Sign out, then sign in again"). Its synced data is
    /// kept, and the record follows the session's spelling.
    #[test]
    fn on_windows_a_case_variant_starts_without_a_sign_out() {
        let (_dir, data) = a_synced_account(&Identity::new(None, Some("sam@beebeeb.io")));
        let variant = Identity::new(None, Some("Sam@Beebeeb.io"));
        assert_eq!(
            bind_before_engine_start(&data, &variant, false),
            Ok(Bound::Kept),
            "it starts"
        );
        assert!(data.db.has_account_data().unwrap(), "and its data is kept");
        assert_eq!(
            data.db.owner().unwrap(),
            Some(variant),
            "one owner, in the session's spelling"
        );
    }

    /// Ruling W2 (kept as it is): with a change waiting to upload, the Windows sign-out refuses ("Pending changes
    /// remain…") and discards nothing, and the binding keeps refusing a start for an email that is not the owner's
    /// (here the same person's changed address, which no id settles). Fix round 3: this used a case-only variant, which
    /// ruling A″ makes the same account.
    #[test]
    fn on_windows_a_change_waiting_to_upload_blocks_the_sign_out_and_the_start_stays_refused() {
        let local = local_data_of(Some(Identity::new(None, Some("sam@beebeeb.io"))));
        let changed_address = Identity::new(None, Some("samuel@beebeeb.io"));
        let refused = Err(crate::account_binding::OTHER_ACCOUNT_ON_WINDOWS.to_string());
        assert_eq!(
            bind_before_engine_start(&local.data, &changed_address, false),
            refused,
            "the premise"
        );
        assert!(
            local.data.db.windows_signout_preflight().is_err(),
            "the sign-out refuses while a change waits"
        );
        assert!(
            local.data.db.finish_windows_signout().is_err(),
            "and could not finish either"
        );
        assert_eq!(
            bind_before_engine_start(&local.data, &changed_address, false),
            refused,
            "so the next start is refused again"
        );
        assert_eq!(
            local.data.db.list_due_operations(i64::MAX).unwrap().len(),
            1,
            "nothing was discarded"
        );
    }

    /// Fix round 1, item 1 (review I1, P2-B): with no owner record to complete (Windows), a session that has an
    /// email is never renamed to another spelling, so an upgraded Windows user whose saved email differs from the
    /// server's only in letter case starts normally (the profile the restore caches anyway does not change who the
    /// session is: it names a different spelling). Since ruling A″ the renamed session would start too (the same
    /// canonical address); under rulings A and A′ it was refused.
    #[test]
    fn without_an_owner_record_a_case_variant_email_is_never_renamed_and_starts_normally() {
        let (fx, local, server) = a_legacy_spelling();
        let named =
            name_session_after_profile(&SessionWrite::for_test(), &fx.state, &fx.acct, "tok-r10", &server, None);
        assert!(!named);
        assert_eq!(
            session_spelling(&fx).as_deref(),
            Some("Sam@Beebeeb.io"),
            "never renamed"
        );
        *fx.acct.cached_profile.lock().unwrap() = Some(server.clone()); // the restore caches the probed profile either way
        let session = identity_of_session(
            session_spelling(&fx).as_deref(),
            fx.acct.cached_profile.lock().unwrap().as_ref(),
        );
        assert_eq!(
            bind_before_engine_start(&local.data, &session, false),
            Ok(Bound::Kept),
            "Windows starts normally"
        );
        let renamed = identity_of_session(Some("sam@beebeeb.io"), Some(&server));
        assert_eq!(
            bind_before_engine_start(&local.data, &renamed, false),
            Ok(Bound::Kept),
            "a renamed session starts too"
        );
        // An email-less session is still named, as before (that is not a rename).
        let unnamed = signed_in_via(None);
        assert!(name_session_after_profile(
            &SessionWrite::for_test(),
            &unnamed.state,
            &unnamed.acct,
            "tok-r10",
            &server,
            None
        ));
        assert_eq!(session_spelling(&unnamed).as_deref(), Some("sam@beebeeb.io"));
    }

    /// Fix round 3: an answer is never applied to a session it was not fetched for. If the session is replaced (a
    /// different token) or named by someone else while the fetch is in flight, the old answer is dropped, so one
    /// account's record can never become the identity of another account's keys.
    #[tokio::test]
    async fn an_answer_is_dropped_when_the_session_changed_while_the_fetch_ran() {
        let owners_answer = serde_json::to_string(&profile_of("u-a", "a@beebeeb.io")).unwrap();
        // 1. The session is replaced by another sign-in's (another token) during the fetch.
        let fx = signed_in_via(None);
        let acct = fx.acct.clone();
        let base_url = answering_after("200 OK", owners_answer.clone(), move || {
            *acct.session.lock().unwrap() = Some(Session {
                token: "tok-another".into(),
                master_key: [9u8; 32],
                email: None,
            });
        });
        assert!(
            !establish_session_identity(&fx.state, &fx.acct, &base_url, None).await,
            "a replaced session is not named by the old answer"
        );
        {
            let session = fx.acct.session.lock().unwrap();
            assert_eq!(
                session.as_ref().map(|s| (s.token.as_str(), s.email.clone())),
                Some(("tok-another", None)),
                "the new session stays unidentified"
            );
        }
        assert!(
            fx.acct.cached_profile.lock().unwrap().is_none(),
            "and no profile is cached for it"
        );
        // 2. The same session is named by someone else during the fetch: the first name stands.
        let fx = signed_in_via(None);
        let acct = fx.acct.clone();
        let base_url = answering_after("200 OK", owners_answer, move || {
            acct.session.lock().unwrap().as_mut().unwrap().email = Some("someone-else@beebeeb.io".into());
        });
        assert!(!establish_session_identity(&fx.state, &fx.acct, &base_url, None).await);
        assert_eq!(
            fx.acct.session.lock().unwrap().as_ref().unwrap().email.as_deref(),
            Some("someone-else@beebeeb.io")
        );
        assert!(fx.acct.cached_profile.lock().unwrap().is_none());
    }

    /// Fix round 3: the restore at launch names an unidentified session BEFORE it asks an engine to start, so the
    /// "open Beebeeb again" the refusal asks for is enough.
    #[test]
    fn the_startup_restore_names_an_unidentified_session_before_it_starts_an_engine() {
        let production = production_source();
        // Task 12 fix round 1, item 7: the restore itself asks the server once (the probe, whose answer names the
        // session); a second request runs in the background after the engine start, which never waits for it.
        let restore = code_of(&production, "async fn restore_stored_session(");
        assert!(
            !restore.contains("establish_session_identity("),
            "the restore does not wait for a second request:\n{restore}"
        );
        let inner = code_of(&production, "async fn restore_session_inner(");
        let restored = inner
            .find("if !restore_stored_session(&state, &acct, &runner::api_base_url()).await {")
            .expect("the restore runs first");
        let started = inner.find("start_engine_if_possible(").expect("and then starts");
        let spawned = inner
            .find("tauri::async_runtime::spawn(identify_then_start(app.clone(), IdentifyPace::Now));")
            .expect("the second request is spawned");
        assert!(restored < started && started < spawned, "{inner}");
        let windows = inner
            .find("#[cfg(target_os = \"windows\")]\n    if acct\n        .session\n        .lock()")
            .expect("Windows keeps today's rule");
        assert!(
            inner[windows..started]
                .contains("establish_session_identity(&state, &acct, &runner::api_base_url(), None).await;"),
            "{inner}"
        );
        assert!(
            inner[spawned - 60..spawned].contains("#[cfg(not(target_os = \"windows\"))]"),
            "{inner}"
        );
    }

    /// R3 (fix round 2): a start onto an UNOWNED, EMPTY database on a Mac owes the Finder removal as a reset does: the
    /// database may have been deleted or never existed while the previous account's domain stayed registered. The
    /// debt is recorded with the first owner, nothing starts in that call, and a confirmed removal (or an Observe that
    /// finds no domain) lets the next start through. Off a Mac nothing is owed.
    #[test]
    fn a_first_start_on_an_unowned_empty_database_owes_the_finder_removal_on_a_mac() {
        let fx = AuthorizeFixture::with_session(&alice());
        let dir = tempfile::tempdir().unwrap();
        let slot = fx.acct.engine.try_lock().unwrap();
        let paths = LocalDataPaths::for_test(dir.path(), dir.path());
        let first = authorize_engine_start(&fx.state, &fx.acct, &slot, &paths);
        let db = crate::state_db::StateDb::open(dir.path().join("state.db")).unwrap();
        assert_eq!(
            db.owner().unwrap(),
            Some(alice()),
            "the first account to start here is recorded"
        );
        if cfg!(target_os = "macos") {
            assert!(
                matches!(first, Ok(StartPermit::FinderRemovalOwed)),
                "nothing starts while a removal is owed: {first:?}"
            );
            assert!(db.finder_removal_owed().unwrap(), "the debt is recorded");
            #[cfg(target_os = "macos")]
            clear_finder_removal_owed_in(dir.path());
            let second = authorize_engine_start(&fx.state, &fx.acct, &slot, &paths);
            assert!(
                matches!(second, Ok(StartPermit::Go(_))),
                "a confirmed removal lets the start through: {second:?}"
            );
            assert!(
                !db.finder_removal_owed().unwrap(),
                "and nothing is owed again: the owner is recorded now"
            );
        } else {
            assert!(matches!(first, Ok(StartPermit::Go(_))), "no Finder, no debt: {first:?}");
            assert!(!db.finder_removal_owed().unwrap());
        }
    }

    /// R3: the exceptions are exact. A sign-out whose Finder removal was CONFIRMED leaves the domain gone, so the
    /// next account's first start owes nothing (no wait on every sign-in after a sign-out); one whose removal was
    /// not confirmed owes it. Off a Mac there is no Finder domain and no removal to confirm: a start after any
    /// sign-out owes nothing.
    #[test]
    fn a_start_after_a_confirmed_sign_out_owes_nothing_and_after_an_unconfirmed_one_it_does() {
        for (owe_after_sign_out, expect_debt) in [(false, false), (true, true)] {
            let local = local_data_of(Some(alice()));
            // The sign-out purge, as `clear_session_impl` runs it, with its removal confirmed or not. Off a Mac it
            // passes `owe_removal = false` whatever happened (pinned by
            // `a_failed_sign_out_purge_stops_the_sign_out_on_macos_and_linux`).
            #[cfg(not(target_os = "windows"))]
            assert_eq!(
                with_own_config(|| purge_local_data_for_sign_out(
                    Ok(local.dir.path().to_path_buf()),
                    None,
                    cfg!(target_os = "macos") && owe_after_sign_out,
                    None
                )),
                Ok(())
            );
            // Windows signs out through `windows_cf::signout::purge`, which refuses while a change is queued (so the
            // queue is emptied first, as a finished sync leaves it) and ends in `finish_windows_signout`: every row
            // cleared, no Finder debt recorded.
            #[cfg(target_os = "windows")]
            {
                local.data.db.purge_all_local_state().unwrap();
                local.data.db.finish_windows_signout().unwrap();
            }
            let fx = AuthorizeFixture::with_session(&bob());
            let slot = fx.acct.engine.try_lock().unwrap();
            let result = authorize_engine_start(
                &fx.state,
                &fx.acct,
                &slot,
                &LocalDataPaths::for_test(local.dir.path(), &local.staging),
            );
            if cfg!(target_os = "macos") && expect_debt {
                assert!(
                    matches!(result, Ok(StartPermit::FinderRemovalOwed)),
                    "owe = {owe_after_sign_out}: {result:?}"
                );
            } else {
                assert!(
                    matches!(result, Ok(StartPermit::Go(_))),
                    "owe = {owe_after_sign_out}: {result:?}"
                );
                assert!(!local.data.db.finder_removal_owed().unwrap());
            }
        }
    }

    /// Fix round 3: the "domain gone" mark holds only until the domain is registered again. After a confirmed
    /// sign-out the next first start owes nothing; once the reconciler has added the domain the mark is cleared, so
    /// an unowned, empty database found later owes the removal check again.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_registered_domain_is_no_longer_known_gone_so_a_later_first_start_owes_a_check() {
        let local = local_data_of(Some(alice()));
        assert_eq!(
            with_own_config(|| purge_local_data_for_sign_out(Ok(local.dir.path().to_path_buf()), None, false, None)),
            Ok(())
        );
        assert!(
            local.data.db.finder_domain_gone().unwrap(),
            "a confirmed sign-out leaves the domain known gone"
        );
        // The reconciler adds the domain (a confirmed add), as `Ports::domain_added` does on a Mac.
        clear_finder_domain_gone_in(local.dir.path());
        assert!(
            !local.data.db.finder_domain_gone().unwrap(),
            "a confirmed add clears the mark"
        );
        // The owner row is gone with the sign-out and the database is empty: the next first start owes the check.
        let fx = AuthorizeFixture::with_session(&bob());
        let slot = fx.acct.engine.try_lock().unwrap();
        let result = authorize_engine_start(
            &fx.state,
            &fx.acct,
            &slot,
            &LocalDataPaths::for_test(local.dir.path(), &local.staging),
        );
        assert!(matches!(result, Ok(StartPermit::FinderRemovalOwed)), "{result:?}");
    }

    /// R4 (fix round 2): the staging directories the sweep covers come from the PRODUCTION call sites, and they pass
    /// the complete list. A test build's list is empty unless a test sets it (so no test sweeps the shared sandbox the
    /// engine-bridge tests stage into); these tests set it and drive the real call sites, so a call site that passes an
    /// empty or a shortened list leaves an orphan behind and fails.
    struct StagingOverride {
        _serial: std::sync::MutexGuard<'static, ()>,
    }

    impl StagingOverride {
        fn set(dirs: Vec<PathBuf>) -> Self {
            static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
            let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
            *STAGING_DIRS_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) = Some(dirs);
            Self { _serial: serial }
        }
    }

    impl Drop for StagingOverride {
        fn drop(&mut self) {
            *STAGING_DIRS_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
    }

    /// Two staging directories in the shape the engine uses (`<root>/beebeeb/finder-writes`), one orphan in each.
    fn two_staging_dirs_with_orphans() -> (PathBuf, Vec<PathBuf>, Vec<PathBuf>) {
        let root = std::env::temp_dir().join(format!("bb-test-r4-{}", uuid::Uuid::new_v4()));
        let dirs: Vec<PathBuf> = ["cache", "temp"]
            .iter()
            .map(|name| root.join(name).join("beebeeb").join("finder-writes"))
            .collect();
        let orphans: Vec<PathBuf> = dirs.iter().map(|dir| dir.join("orphan.bin")).collect();
        for (dir, orphan) in dirs.iter().zip(&orphans) {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(orphan, b"plaintext with no row").unwrap();
        }
        (root, dirs, orphans)
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn the_sign_out_purge_sweeps_every_production_staging_directory() {
        let (root, dirs, orphans) = two_staging_dirs_with_orphans();
        let _override = StagingOverride::set(dirs);
        let state_dir = tempfile::tempdir().unwrap();
        crate::state_db::StateDb::open(state_dir.path().join("state.db")).unwrap();
        // The function `clear_session_impl` calls, which reaches `purge_local_state_files`.
        assert_eq!(
            with_own_config(|| purge_local_data_for_sign_out(Ok(state_dir.path().to_path_buf()), None, false, None)),
            Ok(())
        );
        let left: Vec<&PathBuf> = orphans.iter().filter(|orphan| orphan.exists()).collect();
        let _ = std::fs::remove_dir_all(&root);
        assert!(
            left.is_empty(),
            "every staging directory is swept, none is skipped: {left:?}"
        );
    }

    #[test]
    fn the_engine_start_reset_sweeps_every_production_staging_directory() {
        if !RESET_ALLOWED {
            return; // Windows refuses instead of resetting
        }
        let (root, dirs, orphans) = two_staging_dirs_with_orphans();
        let _override = StagingOverride::set(dirs.clone());
        crate::state_paths::init_for_test(); // the process-wide state dir `LocalDataPaths::production` resolves
        let mut paths = LocalDataPaths::production(&root).unwrap();
        assert_eq!(paths.staging_dirs, dirs, "the production paths carry the complete list");
        // The real paths, but with a database of this test's own (the shared one belongs to the sign-out tests).
        let local = local_data_of(Some(alice()));
        paths.state_dir = local.dir.path().to_path_buf();
        let fx = AuthorizeFixture::with_session(&bob());
        let slot = fx.acct.engine.try_lock().unwrap();
        let _ = authorize_engine_start(&fx.state, &fx.acct, &slot, &paths);
        let left: Vec<&PathBuf> = orphans.iter().filter(|orphan| orphan.exists()).collect();
        let _ = std::fs::remove_dir_all(&root);
        assert!(left.is_empty(), "the reset swept every staging directory: {left:?}");
    }

    /// R4 (fix round 3): the list a RELEASE build sweeps is complete. A test build never compiles the release arm of
    /// `production_staging_dirs` (it reads the override), so two things hold it: the arm is exactly one call to
    /// `release_staging_dirs` (source pin), and that function returns every directory the engine stages into
    /// (behaviour, and `engine_bridge`'s own test that those are the engine's directories).
    #[test]
    fn a_release_build_sweeps_every_directory_the_engine_stages_into() {
        let candidates = engine_bridge::finder_staging_candidates();
        // macOS: the data root, then the cache root and the temp root that builds before the move used (spec
        // 2026-10-09 §8.4). Elsewhere: the preferred directory and the temp-dir fallback.
        let expected = if cfg!(target_os = "macos") { 3 } else { 2 };
        assert_eq!(
            candidates.len(),
            expected,
            "every directory the engine stages into, now and before"
        );
        assert_eq!(
            release_staging_dirs(),
            candidates,
            "the release list is the engine's complete list, in order"
        );

        let production = production_source();
        let code = code_of(&production, "fn production_staging_dirs(");
        let lines: Vec<&str> = code.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
        assert_eq!(
            lines,
            [
                "fn production_staging_dirs() -> Vec<PathBuf> {",
                "#[cfg(not(test))]",
                "{",
                "release_staging_dirs()",
                "}"
            ],
            "the release arm is one call to release_staging_dirs and nothing else:\n{code}"
        );
        let release = code_of(&production, "fn release_staging_dirs(");
        let release: Vec<&str> = release.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
        assert_eq!(
            release,
            [
                "fn release_staging_dirs() -> Vec<PathBuf> {",
                "engine_bridge::finder_staging_candidates()"
            ],
            "the release list is the engine's own list, not a copy of it"
        );
    }

    /// F7 (fix round 1 of Task 10): the sign-in token, and the recovery-phrase unlock's token and key, are
    /// `Zeroizing`, and the recovery-phrase copies are wiped before the engine start's await instead of living
    /// to the end of the function.
    #[test]
    fn the_sign_in_and_recovery_copies_of_the_token_and_key_are_wiped() {
        let production = production_source();
        for signature in ["async fn desktop_login(", "async fn desktop_login_2fa("] {
            let code = code_of(&production, signature);
            let binding = concat!("let session_", "token");
            assert!(
                code.contains(&format!("{binding} = zeroize::Zeroizing::new(")),
                "{signature}: a plain session token"
            );
            assert_eq!(
                code.matches(&format!("{binding} ")).count(),
                1,
                "{signature} keeps a second plain copy"
            );
        }
        let unlock = code_of(&production, "async fn desktop_unlock_with_recovery_phrase(");
        assert!(
            unlock.contains("let token = zeroize::Zeroizing::new("),
            "the loaded token is wrapped:\n{unlock}"
        );
        assert!(
            !unlock.contains("let master_key = *verified_key;"),
            "no plain copy of the verified key:\n{unlock}"
        );
        let wiped = unlock.find("drop(token);").expect("the token is wiped");
        // The one copy into the session happens in `install_recovered_session` (Task 12), which borrows the key.
        assert!(
            unlock.find("drop(verified_key);").expect("the key is wiped")
                > unlock.find("install_recovered_session(").unwrap()
        );
        let install = code_of(&production, "fn install_recovered_session(");
        assert!(
            install.contains("verified_key: &[u8; 32],") && install.contains("master_key: *verified_key"),
            "{install}"
        );
        let started = unlock.rfind("start_engine_if_possible(").expect("the engine start");
        assert!(
            wiped < started,
            "both copies are gone before the await of the engine start:\n{unlock}"
        );
    }

    // ---- sign-out and startup (source pins) ----

    /// R10: on macOS and Linux a sign-out whose purge fails stops, instead of warning and completing. The shape,
    /// as source pins; the behaviour is `the_sign_out_purge_*` below.
    #[test]
    fn a_failed_sign_out_purge_stops_the_sign_out_on_macos_and_linux() {
        let production = production_source();
        let purge = body_between(&production, "fn purge_local_data_for_sign_out(", "\n}\n");
        assert_eq!(purge.matches("return Err(").count(), 1, "a failed clear stops it");
        assert_eq!(
            purge.matches("SIGN_OUT_PURGE_FAILED").count(),
            3,
            "a failed clear, a failed purge and an unopenable database all stop it"
        );
        assert!(
            purge.contains("db.clear_account_data(owe_finder_removal)"),
            "a sign-out leaves no account row and no owner"
        );
        // Task 1887 (spec 2026-10-10 §7.3): the one warning is the unfinished-marker clear's, a best-effort step after
        // the rows are cleared. A purge or a clear that fails still stops the sign-out.
        assert_eq!(
            purge.matches("tracing::warn!").count(),
            1,
            "no warn-and-continue left but the marker clear's:\n{purge}"
        );
        assert!(
            crate::source_pin::squeeze(purge).contains(&crate::source_pin::squeeze(
                "sign_out::kept_record::clear_unfinished_at(&path)) { tracing::warn!("
            )),
            "the one warning is the marker clear's:\n{purge}"
        );
        let clear = body_between(&production, "async fn clear_session_impl(", "\n}\n");
        assert!(
            crate::source_pin::squeeze(clear).contains(&crate::source_pin::squeeze(
                "purge_local_data_for_sign_out(state_paths::beebeeb_state_dir(), cfg_sync_root.as_deref(), owe_removal, kept.as_ref()).map_err(&kept_with)?;"
            )),
            "sign-out returns the purge's error:\n{clear}"
        );
        assert!(
            clear.contains("let owe_removal = finder_cleanup.is_err();"),
            "a removal the reconciler could not confirm is owed before any engine starts:\n{clear}"
        );
        assert!(
            crate::source_pin::squeeze(clear).contains(&crate::source_pin::squeeze(
                "#[cfg(not(target_os = \"macos\"))] let owe_removal = false;"
            )),
            "off a Mac there is no Finder domain, so a sign-out owes no removal:\n{clear}"
        );
    }

    /// The upgrade path runs on what the Keychain gave back, before the probe can drop a revoked token and before
    /// any engine starts, and the window shuts on the path that returns early too.
    #[test]
    fn the_startup_restore_adopts_unbound_local_data_before_anything_starts() {
        let production = production_source();
        let restore = body_between(&production, "async fn restore_stored_session(", "\n}\n");
        let load = restore.find("load_session_from_keychain(").expect("the keychain read");
        let adopt = restore
            .find("adopt_unbound_local_data_at_startup(startup_adoption_candidate(&loaded).as_ref());")
            .expect("the upgrade path runs at startup, on what was loaded");
        assert!(load < adopt, "adoption looks at what the Keychain returned");
        assert!(adopt < restore.find("probe_startup_session(").expect("the probe"));
        assert!(
            !restore.contains("start_engine_if_possible("),
            "the engine start comes after, in the caller"
        );
        let inner = body_between(&production, "async fn restore_session_inner(", "\n}\n");
        assert!(
            inner.find("restore_stored_session(").expect("the restore")
                < inner.find("start_engine_if_possible(").expect("the engine start")
        );
        assert!(
            restore.contains("adopt_unbound_local_data_at_startup(None);"),
            "the early return shuts the window too:\n{restore}"
        );
        let candidate = body_between(&production, "fn startup_adoption_candidate(", "\n}\n");
        assert!(
            candidate.contains("loaded.as_ref().ok()?.as_ref()?"),
            "only a session the Keychain returned COMPLETE names a candidate:\n{candidate}"
        );
    }

    // ---- the upgrade path, with the Keychain the tests use (an in-memory one on a Mac) ----

    /// A complete session in the (test) Keychain: token, wrapped vault key and email.
    #[cfg(target_os = "macos")]
    fn store_complete_session(acct: &crate::account::AccountRuntime, email: &str) {
        persist_session_to_keychain(
            &SessionWrite::for_test(),
            acct.id.as_str(),
            "tok-keychain",
            &[3u8; 32],
            Some(email),
        )
        .expect("the in-memory test store takes it");
    }

    /// What the startup restore does with the Keychain, for the data in `dir`: read the session, hand the adoption
    /// what came back, report who (if anybody) the data now belongs to.
    #[cfg(target_os = "macos")]
    fn startup_adoption(dir: &Path, acct: &crate::account::AccountRuntime) -> Option<Identity> {
        let loaded = load_session_from_keychain(acct.id.as_str(), None);
        adopt_unbound_local_data_in(dir, startup_adoption_candidate(&loaded).as_ref());
        crate::state_db::StateDb::open(dir.join("state.db"))
            .unwrap()
            .owner()
            .unwrap()
    }

    /// Lead ruling 8: an install from before the binding keeps its queue. The account whose vault key this
    /// computer's Keychain holds is recorded as the owner at startup, and the same account's first engine start
    /// then keeps everything.
    #[cfg(target_os = "macos")]
    #[test]
    fn an_upgrader_keeps_the_queue_when_the_keychain_holds_the_same_accounts_vault_key() {
        let fx = AuthorizeFixture::without_session();
        store_complete_session(&fx.acct, "a@beebeeb.io");
        let local = local_data_of(None);
        assert_eq!(
            local.data.db.owner().unwrap(),
            None,
            "the premise: data from before the binding has no owner"
        );
        assert_eq!(
            startup_adoption(local.dir.path(), &fx.acct),
            Some(Identity::new(None, Some("a@beebeeb.io"))),
            "the Keychain's account owns it"
        );

        // The same person signs back in (offline: by the exact email the Keychain names).
        let session = identity_of_session(Some("a@beebeeb.io"), None);
        assert_eq!(bind_data_to_session(&fx.state, &local.data, &session), Ok(()));
        assert_eq!(
            local.data.db.list_due_operations(i64::MAX).unwrap().len(),
            1,
            "the queue survived the upgrade"
        );
        for path in &local.files {
            assert!(path.exists(), "{} survived the upgrade", path.display());
        }
    }

    /// The same upgrade, but another account starts: it does not inherit the queue.
    #[cfg(target_os = "macos")]
    #[test]
    fn an_upgrade_never_hands_the_queue_to_another_account() {
        let fx = AuthorizeFixture::without_session();
        store_complete_session(&fx.acct, "a@beebeeb.io");
        let local = local_data_of(None);
        startup_adoption(local.dir.path(), &fx.acct);
        let bob_session = Identity::new(None, Some("b@beebeeb.io"));
        assert_eq!(bind_data_to_session(&fx.state, &local.data, &bob_session), Ok(()));
        assert!(
            local.data.db.list_due_operations(i64::MAX).unwrap().is_empty(),
            "another account does not inherit the queue"
        );
    }

    // ---- the Keychain keeps an email only next to the session it names (fix round 4) ----

    /// A sign-in that has no identity (its account-record fetch failed: `apply_session` stores `session_email(None)`)
    /// stores its token and vault key with NO email. The email the Keychain kept for the account signed in before is
    /// removed, so the stored session is unidentified and is never taken for that account.
    #[cfg(target_os = "macos")]
    #[test]
    fn an_unidentified_sign_in_removes_the_previous_accounts_email_from_the_keychain() {
        let fx = AuthorizeFixture::without_session();
        let id = fx.acct.id.as_str().to_string();
        store_complete_session(&fx.acct, "a@beebeeb.io");
        assert_eq!(
            keychain_account_email(&id).as_deref(),
            Some("a@beebeeb.io"),
            "the premise: the Keychain names the account before"
        );
        persist_session_to_keychain(
            &SessionWrite::for_test(),
            &id,
            "tok-b",
            &[5u8; 32],
            session_email(None).as_deref(),
        )
        .expect("the in-memory test store takes it");
        assert_eq!(
            keychain_account_email(&id),
            None,
            "no email is kept next to the new token and key"
        );
        let restored = load_session_from_keychain(&id, None)
            .unwrap()
            .expect("the new session is stored");
        assert_eq!(
            (restored.token.as_str(), restored.master_key, restored.email.clone()),
            ("tok-b", [5u8; 32], None),
            "restored without an identity"
        );
    }

    /// What a relaunch then does, step by step as `restore_session_inner` runs it (the restore itself needs an
    /// `AppHandle`): read the Keychain (no email in memory yet), offer the adoption what it read, install the session,
    /// try to name it (the server cannot be reached), start the engine. The session is unidentified: no engine, nothing
    /// purged, nobody adopted, no owner replaced, no Finder debt. For the previous account's data and for unowned data.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn after_an_unidentified_sign_in_a_relaunch_starts_nothing_purges_nothing_and_adopts_nothing() {
        for (name, owner) in [
            ("the previous account's data", Some(alice())),
            ("data with no recorded owner", None),
        ] {
            let fx = AuthorizeFixture::without_session();
            let id = fx.acct.id.as_str().to_string();
            store_complete_session(&fx.acct, "a@beebeeb.io");
            let local = local_data_of(owner.clone());
            persist_session_to_keychain(
                &SessionWrite::for_test(),
                &id,
                "tok-b",
                &[5u8; 32],
                session_email(None).as_deref(),
            )
            .expect("the in-memory test store takes it");

            let loaded = load_session_from_keychain(&id, None);
            adopt_unbound_local_data_in(local.dir.path(), startup_adoption_candidate(&loaded).as_ref());
            let owner_after_adoption = local.data.db.owner().unwrap();
            let session = loaded.unwrap().expect("the new session is stored");
            let restored_email = session.email.clone();
            *fx.acct.session.lock().unwrap() = Some(session);
            establish_session_identity(
                &fx.state,
                &fx.acct,
                &answering("503 Service Unavailable", "{}".to_string()),
                None,
            )
            .await;
            let mut slot = fx.acct.engine.try_lock().unwrap();
            let paths = LocalDataPaths::for_test(local.dir.path(), &local.staging);
            let seen = std::cell::RefCell::new(None);
            let result = start_engine_bound(&fx.state, &fx.acct, &mut slot, &paths, |_root, _token, _key| {
                *seen.borrow_mut() = Some((
                    local.data.db.list_due_operations(i64::MAX).unwrap().len(),
                    local.files.iter().filter(|path| path.exists()).count(),
                ));
                crate::runner::EngineRunner::for_test_with_task(tokio::spawn(async {}))
            });
            assert_eq!(
                (owner_after_adoption, restored_email, result, seen.into_inner()),
                (
                    owner.clone(),
                    None,
                    Err(crate::account_binding::IDENTITY_UNKNOWN.to_string()),
                    None
                ),
                "{name}: (owner after the adoption, restored email, engine start, what an engine was created with)"
            );
            assert!(slot.is_none(), "{name}: no engine in the slot");
            assert_eq!(
                local.data.db.list_due_operations(i64::MAX).unwrap().len(),
                1,
                "{name}: nothing purged"
            );
            for path in &local.files {
                assert!(path.exists(), "{name}: {} is kept", path.display());
            }
            assert_eq!(local.data.db.owner().unwrap(), owner, "{name}: the owner is unchanged");
            assert!(!local.data.db.finder_removal_owed().unwrap(), "{name}: no Finder debt");
        }
    }

    /// An identified sign-in stores its OWN email in place of the previous account's: the browser sign-in (token and
    /// key) and the password and 2FA sign-ins (token only). A token-only write without an email (no sign-in does that
    /// today; the writer is shared) leaves none either.
    #[cfg(target_os = "macos")]
    #[test]
    fn an_identified_sign_in_stores_its_own_email_in_place_of_the_previous_accounts() {
        let email = session_email(Some(&profile_of("u-b", "b@beebeeb.io")));
        for (name, token_only) in [("browser", false), ("password or 2FA", true)] {
            let fx = AuthorizeFixture::without_session();
            let id = fx.acct.id.as_str().to_string();
            store_complete_session(&fx.acct, "a@beebeeb.io");
            if token_only {
                persist_session_token_to_keychain(&SessionWrite::for_test(), &id, "tok-b", email.as_deref()).unwrap();
            } else {
                persist_session_to_keychain(&SessionWrite::for_test(), &id, "tok-b", &[5u8; 32], email.as_deref())
                    .unwrap();
                let restored = load_session_from_keychain(&id, None).unwrap().expect("stored");
                assert_eq!(
                    restored.email.as_deref(),
                    Some("b@beebeeb.io"),
                    "{name}: the restore knows the session by its own email"
                );
            }
            assert_eq!(keychain_account_email(&id).as_deref(), Some("b@beebeeb.io"), "{name}");
        }
        let fx = AuthorizeFixture::without_session();
        let id = fx.acct.id.as_str().to_string();
        store_complete_session(&fx.acct, "a@beebeeb.io");
        persist_session_token_to_keychain(&SessionWrite::for_test(), &id, "tok-c", None).unwrap();
        assert_eq!(
            keychain_account_email(&id),
            None,
            "a token-only write without an email keeps none"
        );
    }

    /// Every write of a session token removes the email kept for the session before it, BEFORE the token is stored
    /// (so no failure part-way can leave the new token next to the old email), and a removal that fails stops the
    /// write; the session's own email is written after its secrets. The vault key alone is written only by the
    /// recovery-phrase unlock, for the token already stored, whose email was written or removed with that token.
    #[test]
    fn every_write_of_a_session_token_removes_the_previous_email_first() {
        let production = production_source();
        assert_eq!(
            production.matches(".install_session(").count(),
            2,
            "the two writers of a session token"
        );
        for signature in [
            "fn persist_session_to_keychain(",
            "fn persist_session_token_to_keychain(",
        ] {
            let code = code_of(&production, signature);
            let forget = code
                .find("forget_account_email_in_keychain(account_id)?;")
                .unwrap_or_else(|| {
                    panic!("{signature}: removes the previous email, and a failure stops the write:\n{code}")
                });
            let token = code.find(".install_session(").expect("the token");
            assert!(
                forget < token,
                "{signature}: the email goes before the token is stored:\n{code}"
            );
            let own = code
                .find("persist_account_email_to_keychain(&vault, email)")
                .unwrap_or_else(|| panic!("{signature}: its own email"));
            assert!(
                token < own,
                "{signature}: the session's own email is written after its secrets:\n{code}"
            );
        }
        let forget = code_of(&production, "fn forget_account_email_in_keychain(");
        assert!(
            forget.contains(".delete_account_email()") && !forget.contains("let _"),
            "{forget}"
        );
        assert_eq!(
            production.matches(".store_wrapped_master_key(").count(),
            2,
            "with a new token, and for the recovery phrase"
        );
        assert_eq!(
            production.matches("persist_vault_key_to_keychain(").count(),
            2,
            "defined once, called by the recovery-phrase unlock only"
        );
        let recovery = code_of(&production, "async fn desktop_unlock_with_recovery_phrase(");
        assert!(
            recovery.contains("load_session_token_from_keychain(&account_id)?"),
            "the key is for the token already stored"
        );
    }

    /// F2, across restarts: unowned data from before the upgrade, and a Keychain that holds a token but no vault key.
    /// The startup pass adopts nobody and uses up the one window; once the key is added and the session installed,
    /// the engine starts only on a clean slate; at every later startup the Keychain is complete and adopts nothing.
    #[cfg(target_os = "macos")]
    #[test]
    fn adoption_needs_a_vault_key_and_the_window_closes_at_the_first_startup() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let fx = AuthorizeFixture::without_session();
            let id = fx.acct.id.as_str().to_string();
            let local = local_data_of(None); // another account's queue and files, no owner
            // 1. A sign-in stores a token and an email, and no key yet.
            persist_session_token_to_keychain(&SessionWrite::for_test(), &id, "tok-b", Some("b@beebeeb.io")).unwrap();
            // 2. A startup: the Keychain holds no vault key for that token, so nobody is adopted.
            let loaded = load_session_from_keychain(&id, None);
            assert!(loaded.is_err(), "a token without a key does not restore: {loaded:?}");
            assert_eq!(startup_adoption_candidate(&loaded), None);
            assert_eq!(startup_adoption(local.dir.path(), &fx.acct), None, "nobody was adopted");
            // 3. The recovery phrase is entered: the key is stored and the session is installed.
            persist_vault_key_to_keychain(&SessionWrite::for_test(), &id, &[3u8; 32]).unwrap();
            *fx.acct.session.lock().unwrap() = Some(Session {
                token: "tok-b".into(),
                master_key: [3u8; 32],
                email: Some("b@beebeeb.io".into()),
            });
            // 4. B's engine starts. When it is created the previous account's queue and files are gone. (The Mac
            //    first waits for the previous account's Finder domain to be removed: a confirmed removal frees it.)
            let mut slot = fx.acct.engine.lock().await;
            let seen = std::cell::RefCell::new(None);
            let paths = LocalDataPaths::for_test(local.dir.path(), &local.staging);
            let spawner = || {
                |_root: PathBuf, _token: zeroize::Zeroizing<String>, _key: zeroize::Zeroizing<[u8; 32]>| {
                    *seen.borrow_mut() = Some((
                        local.data.db.list_due_operations(i64::MAX).unwrap().len(),
                        local.files.iter().filter(|path| path.exists()).count(),
                    ));
                    crate::runner::EngineRunner::for_test_with_task(tokio::spawn(async {}))
                }
            };
            let first = start_engine_bound(&fx.state, &fx.acct, &mut slot, &paths, spawner());
            assert_eq!(
                first,
                Ok(EngineStart::FinderRemovalOwed),
                "the reset leaves the previous account's domain to remove first"
            );
            assert!(seen.borrow().is_none(), "no engine while the removal is owed");
            clear_finder_removal_owed_in(local.dir.path());
            let started = start_engine_bound(&fx.state, &fx.acct, &mut slot, &paths, spawner());
            assert_eq!(started, Ok(EngineStart::Started));
            assert_eq!(
                seen.into_inner(),
                Some((0, 0)),
                "the engine saw none of the previous account's data"
            );
            drop(slot);
            // 5. A later startup: the Keychain is complete and names the signed-in account. The window is shut, so
            //    nothing is adopted, and nothing of the previous account is back.
            let loaded = load_session_from_keychain(&id, None);
            assert_eq!(
                startup_adoption_candidate(&loaded),
                Some(Identity::new(None, Some("b@beebeeb.io")))
            );
            assert_eq!(
                startup_adoption(local.dir.path(), &fx.acct),
                Some(Identity::new(None, Some("b@beebeeb.io")))
            );
            assert!(local.data.db.list_due_operations(i64::MAX).unwrap().is_empty());
        });
    }

    /// And the window really is shut: with a complete Keychain at a LATER startup, unowned data is not adopted.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_later_startup_never_adopts_even_with_a_complete_keychain() {
        let fx = AuthorizeFixture::without_session();
        let id = fx.acct.id.as_str().to_string();
        let local = local_data_of(None);
        persist_session_token_to_keychain(&SessionWrite::for_test(), &id, "tok-b", Some("b@beebeeb.io")).unwrap();
        assert_eq!(
            startup_adoption(local.dir.path(), &fx.acct),
            None,
            "the first startup: no vault key"
        );
        persist_vault_key_to_keychain(&SessionWrite::for_test(), &id, &[3u8; 32]).unwrap();
        assert_eq!(
            startup_adoption(local.dir.path(), &fx.acct),
            None,
            "a later startup with a complete Keychain: too late"
        );
        assert_eq!(local.data.db.owner().unwrap(), None);
    }
