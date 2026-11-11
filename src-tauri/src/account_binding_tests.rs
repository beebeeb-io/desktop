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
    let recorded = at(".inspect_err(|error| record_engine_refusal(acct, turn, error))?").expect("it records a refusal");
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
            !line.starts_with("//") && line.contains("spawn_bound_engine(") && !line.contains("fn spawn_bound_engine(")
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
    let runner = [
        include_str!("runner.rs"),
        include_str!("runner/lifecycle.rs"),
        include_str!("runner/telemetry.rs"),
        include_str!("runner/tick_loop.rs"),
        include_str!("runner/working_set_signal.rs"),
    ]
    .join("\n")
    .replace("\r\n", "\n");
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
    let pure = include_str!("../../crates/beebeeb-engine/src/account_binding.rs");
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
    let surfaced = gate
        .find(&squeeze("surface_reset_folder(&surface_app, &folder, files);"))
        .expect("surfaced");
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
    let locked = background
        .find(&squeeze("let transition = SESSION_TRANSITION.lock().await;"))
        .expect("the lock");
    let current = background
        .find(&squeeze("if !session_still_active(&app, &acct, turn)"))
        .expect("the check");
    let restart = background.find("start_engine_if_possible(").expect("a restart");
    let other = background.find("Ok(outcome)=>").expect("every other outcome");
    assert!(
        adopted < locked && locked < current && current < restart && restart < other,
        "{background}"
    );
    assert_eq!(
        background.matches("start_engine_if_possible(").count(),
        1,
        "{background}"
    );

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
    assert_eq!(
        proof.matches("api.get_file(").count(),
        0,
        "no question per file: {proof}"
    );
    let still = squeeze(&code_of(&production, "fn session_still_active("));
    assert!(
        still.contains(&squeeze("acct.session_unchanged_since(turn)")),
        "{still}"
    );
    assert!(still.contains(&squeeze("Arc::ptr_eq(&active, acct)")), "{still}");

    // Review P2-2: a Lock and a sign-out stop every running proof before they wait for the credentials, so a proof
    // never holds the key and the token past either.
    assert!(
        background.contains(&squeeze("OWN_DATA_PROOFS.lock()")) && background.contains("proofs.push(proof)"),
        "the proof is kept to be stopped: {background}"
    );
    for (name, body) in [
        ("lock_vault", code_of(&production, "async fn lock_vault(")),
        (
            "clear_session_impl",
            code_of(&production, "async fn clear_session_impl("),
        ),
    ] {
        let body = squeeze(&body);
        let stop = body
            .find("stop_own_data_proofs();")
            .unwrap_or_else(|| panic!("{name} stops the proofs: {body}"));
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
    assert!(
        PROOF_REQUEST_TIMEOUT >= std::time::Duration::from_secs(300),
        "{PROOF_REQUEST_TIMEOUT:?}"
    );
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
    assert!(
        settle.contains(&squeeze("found_while_paused::recover(&db, &paths.sync_root)")),
        "{settle}"
    );

    let handler = body_between(&production, "tauri::generate_handler![", "])");
    for command in [
        "found_while_paused_review,",
        "found_while_paused_upload,",
        "found_while_paused_keep,",
        "show_kept_folder,",
    ] {
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
            &[
                "kept_folder_to_show(&sync_root, db.last_kept_dir()",
                "std::process::Command::new(\"explorer\").arg(&folder)",
            ][..],
        ),
    ] {
        let body = squeeze(&code_of(&production, &format!("{windows}{signature}")));
        for step in steps {
            assert!(body.contains(&squeeze(step)), "{step}: {body}");
        }
    }
    let own = squeeze(&code_of(&production, "fn own_state_db("));
    assert!(
        own.contains(&squeeze(
            "account_binding::same_account(&owner, identity) == Some(true)"
        )),
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
    assert_eq!(
        own_state_db(&bob(), mine.dir.path()).map(|_| ()),
        refused,
        "another account does not"
    );
    let unowned = local_data_of(None);
    assert_eq!(
        own_state_db(&alice(), unowned.dir.path()).map(|_| ()),
        refused,
        "data nobody owns yet"
    );
    let empty = tempfile::tempdir().unwrap();
    assert_eq!(own_state_db(&alice(), empty.path()).map(|_| ()), refused, "no database");
    let by_email = Identity::new(None, Some("A@Beebeeb.io"));
    assert!(
        own_state_db(&by_email, mine.dir.path()).is_ok(),
        "the same account by its email"
    );

    assert_eq!(
        decision_identity(&AuthorizeFixture::with_session(&alice()).state),
        Ok(alice())
    );
    assert_eq!(
        decision_identity(&AuthorizeFixture::without_session().state).map(|_| ()),
        refused,
        "no session"
    );
}

/// Task 2267, lead ruling on self-review F9 (2026-10-29): Keep's stop message names the kept folder as it is made.
#[cfg(target_os = "windows")]
#[test]
fn keep_stopped_names_the_kept_folder_as_it_is_made() {
    assert!(
        FOUND_WHILE_PAUSED_KEEP_STOPPED.contains("“Beebeeb - not uploaded”"),
        "{FOUND_WHILE_PAUSED_KEEP_STOPPED}"
    );
    let named = format!("“{}”", crate::found_while_paused::KEPT_FOLDER_NAME);
    assert!(
        FOUND_WHILE_PAUSED_KEEP_STOPPED.contains(&named),
        "{FOUND_WHILE_PAUSED_KEEP_STOPPED}"
    );
}

/// Task 2267 (fix round 2): "Show in Explorer" opens only the dated folder the last keep recorded, inside
/// `Beebeeb - not uploaded` next to the Beebeeb folder, and only while it is there.
#[test]
fn show_in_explorer_opens_only_a_dated_folder_in_the_kept_folder_next_to_the_beebeeb_folder() {
    let top = tempfile::tempdir().unwrap();
    let root = top.path().join("Beebeeb");
    let kept = top.path().join("Beebeeb - not uploaded");
    let dated = kept.join("2026-10-28");
    for dir in [
        &root,
        &dated,
        &kept.join("notes"),
        &top.path().join("elsewhere").join("2026-10-28"),
    ] {
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
            let held: Vec<String> = local
                .data
                .db
                .found_while_paused_ops()
                .unwrap()
                .into_iter()
                .map(|op| op.op_id)
                .collect();
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

// The file-size ratchet: the rest of this module, unchanged, in two files.
mod part_2;
mod part_3;
