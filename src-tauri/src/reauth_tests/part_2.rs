/// Spec D11: a Mac resumes in the engine start, before the engine exists; Windows and Linux do not resume there
/// (the engine does, after its pass). The start reads the ask, and resumes, on macOS only.
#[test]
fn start_engine_bound_resumes_before_the_spawn_on_macos_only() {
    use crate::source_pin::squeeze;
    let production = production_source();
    let macos_only = squeeze("#[cfg(target_os = \"macos\")]");
    let bound = squeeze(&code_of(&production, "fn start_engine_bound("));
    for gated in [
        "let resume_asked = acct.auth_resume_asked.load(Ordering::SeqCst);",
        "resume_operations_paused_for_auth(acct, paths, resume_asked);",
    ] {
        assert!(
            bound.contains(&format!("{macos_only}{}", squeeze(gated))),
            "{gated} on macOS only:\n{bound}"
        );
    }
    let resumed = bound
        .find("resume_operations_paused_for_auth(acct,paths,resume_asked);")
        .unwrap();
    assert!(
        resumed < bound.find("spawn(paths.sync_root.clone()").unwrap(),
        "before the spawn:\n{bound}"
    );
    let other_platforms =
        super::finder_setup_command_tests::without_items(&production, &["#[cfg(target_os = \"macos\")]"]);
    let other_bound = code_of(&other_platforms, "fn start_engine_bound(");
    assert!(
        !other_bound.contains("resume_operations_paused_for_auth") && !other_bound.contains("resume_asked"),
        "Windows and Linux do not resume in the start:\n{other_bound}"
    );
    assert!(
        !other_platforms.contains("fn resume_operations_paused_for_auth("),
        "the start's resume is a Mac's"
    );
    // The engine gets the account's resume requests on every OS.
    let spawn = squeeze(&code_of(&production, "fn spawn_bound_engine("));
    assert!(
        spawn.contains(&squeeze(
            "runner::ResumeTicket { asked: acct.auth_resume_asked.clone(), done: acct.auth_resume_done.clone(),
                 pass_wanted: acct.reconcile_pass_wanted.clone(), pass_served: acct.reconcile_pass_served.clone(), }"
        )),
        "{spawn}"
    );
}

/// Spec D11 and I6: off macOS the engine runs the reconcile pass, then the key check and the resume, inside the
/// successful tick's arm (the pull proved the server accepts this session's token) and before that tick's queue
/// step. D11b: the send phase's wake runs no queue step until this run's first pass has committed.
#[test]
fn the_engine_reconciles_then_resumes_in_the_successful_tick_before_its_queue_step() {
    use crate::source_pin::squeeze;
    let runner = include_str!("../../../crates/beebeeb-engine/src/runner.rs").replace("\r\n", "\n");
    let runner = &runner[..runner.find("#[cfg(test)]\nmod tests").expect("runner's tests")];
    let tick = &runner[runner.find("Ok(tick) => {").expect("the successful tick's arm")..];
    let tick = squeeze(&tick[..tick.find("Err(e) => {").unwrap()]);
    let pass = tick
            .find(&squeeze(
                "#[cfg(not(target_os = \"macos\"))]
                 if tick.reached_server
                     && let Err(error) = bridge.reconcile_then_resume(&sync_root, &resume, &mut reconciled_this_run).await {",
            ))
            .unwrap_or_else(|| panic!("the pass, off macOS, in the arm, only when the pull came back:\n{tick}"));
    assert!(
        tick.find("auth_health.note_result(None);").unwrap() < pass,
        "after the pull succeeded"
    );
    assert!(
        pass < tick.find("queue_step(").unwrap(),
        "before the tick's queue step:\n{tick}"
    );
    assert_eq!(squeeze(runner).matches("reconcile_then_resume(").count(), 1, "one call");
    // Review P2: the engine's queue reads the same ticket, so it sends nothing while a release waits for a pass.
    assert!(
        squeeze(runner).contains(&squeeze(".with_resume(resume.clone())")),
        "the bridge shares the ticket"
    );
    assert!(
        squeeze(runner).contains(&squeeze(
            "#[cfg(not(target_os = \"macos\"))]
                 let mut reconciled_this_run = false;"
        )),
        "per engine run"
    );
    // D11b.
    let phase = &runner[runner.find("Wake::Phase => {").expect("the phase's arm")..];
    let phase = squeeze(&phase[..phase.find("\n            }\n").unwrap()]);
    let waits = phase
        .find(&squeeze(
            "#[cfg(not(target_os = \"macos\"))]
                 if !reconciled_this_run { continue; }",
        ))
        .unwrap_or_else(|| panic!("the phase waits for the first pass:\n{phase}"));
    assert!(waits < phase.find("queue_step(").unwrap(), "{phase}");
}

/// Task 1 ruling (1888 ledger, on 2267's held changes; review P2): "Upload to my Beebeeb" makes a held ghost folder's
/// create due again, so it asks for a pass before the release commits and again after it, on the pass counter (the
/// engine's queue waits for that pass; a release resumes no `auth` op).
#[test]
fn a_released_held_change_asks_for_a_pass() {
    use crate::source_pin::squeeze;
    let production = production_source();
    let upload = squeeze(&code_of(&production, "async fn found_while_paused_upload("));
    let ask = squeeze("pass_wanted.fetch_add(1, Ordering::SeqCst);");
    let released = upload.find("db.release_found_while_paused(").expect("the release");
    let before = upload
        .find(&ask)
        .unwrap_or_else(|| panic!("asks for a pass:\n{upload}"));
    let after = upload.rfind(&ask).unwrap();
    assert!(
        before < released && released < after,
        "before the release commits, and again after it:\n{upload}"
    );
    assert!(
        upload.contains(&squeeze("let pass_wanted = state.active_account()"))
            && upload.contains(&squeeze(".reconcile_pass_wanted.clone();")),
        "the pass counter, not the sign-in counter:\n{upload}"
    );
    assert!(
        !upload.contains("auth_resume_asked"),
        "a release resumes no auth op:\n{upload}"
    );
}

/// Spec §5.6: a re-sign-in whose kept key the server no longer accepts removes only that key. It never purges,
/// never removes the domain, never signs out, never writes a key and never starts an engine; the session in memory
/// ends the Lock way (the turn first, then the reconciler's hold, then the engine stop, all before the writes, which
/// run under the slot); and memory is cleared before the key is removed from both layouts and the token stored.
/// Compared with `squeeze`, so the pins do not depend on rustfmt's layout.
#[test]
fn a_kept_key_the_server_rejects_is_removed_the_lock_way_and_nothing_else_changes() {
    use crate::source_pin::squeeze;
    let production = production_source();
    let settle = squeeze(&code_of(&production, "async fn settle_sign_in("));
    let arm = settle
        .find(&squeeze("reauth::SignInKind::KeyOutdated =>"))
        .expect("the KeyOutdated arm");
    assert!(
            settle[arm..].starts_with(&squeeze(
                "reauth::SignInKind::KeyOutdated => { reauth_without_the_kept_key(state, &acct, &sources, token, profile, turn).await"
            )) || settle[arm..].starts_with(&squeeze(
                "reauth::SignInKind::KeyOutdated => reauth_without_the_kept_key(state, &acct, &sources, token, profile, turn).await"
            )),
            "{settle}"
        );
    let outer = squeeze(&code_of(&production, "async fn reauth_without_the_kept_key("));
    let inner = squeeze(&code_of(&production, "fn reauth_swap_token_without_key("));
    for (name, body) in [
        ("reauth_without_the_kept_key", &outer),
        ("reauth_swap_token_without_key", &inner),
    ] {
        for forbidden in [
            "purge_local_state_files",
            "purge_all_local_state",
            "purge_local_data_for_sign_out",
            "clear_account_data",
            "clear_keychain_session",
            "clear_session_impl",
            "remove_file_provider_domain",
            "finder_remove_for",
            "Trigger::SignOut",
            "Trigger::Repair",
            "persist_vault_key_to_keychain",
            "persist_session_to_keychain",
            "store_wrapped_master_key",
            "install_unlocked_session",
            "EngineRunner::",
            "spawn_bound_engine",
            "start_engine_if_possible",
            "keys_arrived",
        ] {
            assert!(!body.contains(forbidden), "{name} must not use {forbidden}:\n{body}");
        }
    }
    let order = |body: &str, steps: &[&str]| {
        let at: Vec<usize> = steps
            .iter()
            .map(|step| body.find(&squeeze(step)).unwrap_or_else(|| panic!("{step}:\n{body}")))
            .collect();
        assert!(at.windows(2).all(|pair| pair[0] < pair[1]), "{steps:?}:\n{body}");
    };
    order(
        &outer,
        &[
            "drop(claim_session_write(acct, turn)",
            "finder_lock_for(state)",
            "acct.engine.lock().await",
            "stop_engine_in_slot(acct, engine)",
            "reauth_swap_token_without_key(state, acct, sources, token, profile, turn, &engine_slot)",
            "drop(engine_slot);",
        ],
    );
    order(
        &inner,
        &[
            "claim_session_write(acct, turn)",
            "acct.session.lock()",
            "record_local_data_owner(",
            "hold_auth_paused_for_the_replaced_key(sources, _engine_slot)",
            "delete_vault_key_from_keychain(&write, acct.id.as_str())",
            "persist_session_token_to_keychain(&write, acct.id.as_str(), token, email.as_deref())",
            "drop(write);",
        ],
    );
    assert!(
        inner.contains(&squeeze(
            "_engine_slot: &tokio::sync::MutexGuard<'_, Option<EngineRunner>>"
        )),
        "the writes run while the caller holds the engine slot:\n{inner}"
    );
    let delete = squeeze(&code_of(&production, "fn delete_vault_key_from_keychain("));
    for needle in [
        "platform_keychain_store_for(account_id).delete_wrapped_master_key()",
        "keychain::legacy_platform_keychain_store().delete_wrapped_master_key()",
        "if keychain_vault_key_present(account_id) {",
    ] {
        assert!(
            delete.contains(&squeeze(needle)),
            "both layouts, then checked gone ({needle}):\n{delete}"
        );
    }
    for forbidden in ["delete_session_token", "delete_account_email", "clear_session"] {
        assert!(
            !delete.contains(forbidden),
            "only the key is removed ({forbidden}):\n{delete}"
        );
    }
}

/// Spec §5.6: the browser handoff carries the account's current key. `reauth_in_place` returns before it starts an
/// engine when the kept key differs from it (so the handoff's own `apply_session` installs it), and the handoff
/// returns early only when the vault is unlocked with the kept key. The comparison reads every byte.
#[test]
fn a_handed_over_key_that_differs_is_installed_and_the_comparison_reads_every_byte() {
    use crate::source_pin::squeeze;
    let production = production_source();
    let in_place = squeeze(&code_of(&production, "async fn reauth_in_place("));
    let check = in_place
        .find(&squeeze(
            "if vault_unlocked && handoff_replaces_kept_key(acct, handoff_key) {",
        ))
        .expect("the handoff key is compared");
    let start = in_place.find("start_engine_if_possible(").expect("the engine start");
    assert!(check < start, "compared before any engine starts:\n{in_place}");
    assert!(in_place[check..start].contains("returnOk(false);"), "{in_place}");
    let browser = squeeze(&browser_source());
    assert!(
        browser.contains(&squeeze("Some(&*master_key),")),
        "the handoff passes its key to the check"
    );
    assert!(
        browser.contains(&squeeze(
            "Some(crate::SignInSettlement::Reauthenticated { vault_unlocked: true, .. }) => {"
        )),
        "only an unlocked vault returns early"
    );
    let equal = squeeze(&code_of(&production, "fn vault_keys_equal("));
    assert!(
        equal.contains(&squeeze(".fold(0u8, |acc, (x, y)| acc | (x ^ y))"))
            && equal.contains(&squeeze("std::hint::black_box(difference) == 0")),
        "{equal}"
    );
    let compares = equal.replace(&squeeze("std::hint::black_box(difference) == 0"), "");
    for early in ["return", "==", "!=", "any(", "all(", "position("] {
        assert!(!compares.contains(early), "no early exit ({early}):\n{equal}");
    }
    assert!(
        squeeze(&code_of(&production, "fn session_holds_vault_key("))
            .contains(&squeeze("vault_keys_equal(&session.master_key, key)")),
        "the one comparison"
    );
}

/// The server proof is the one place R8 reads a vault key: into a wiped buffer that lives inside one function, from
/// which a `MasterKey` (wiped on drop, not `Clone`) is built at once. Nothing else holds a plain copy: the proof
/// receives the `MasterKey` and sends only its check. (The core takes the 32 bytes by value and has no
/// reference-taking constructor; it wipes the argument it is given, and the buffer here is a `Zeroizing`.)
#[test]
fn the_server_proof_reads_the_key_only_to_send_its_check() {
    let production = production_source();
    assert_eq!(
        production.matches("load_stored_vault_key(").count(),
        2,
        "defined once, called by the proof only"
    );
    assert_eq!(production.matches("fn read_vault_key<").count(), 1);
    assert_eq!(
        production.matches("read_vault_key(").count(),
        2,
        "called for the two Keychain layouts, and only there"
    );
    let proof = code_of(&production, "async fn prove_stored_key(");
    let load = proof.find("load_stored_vault_key(").expect("the proof reads the key");
    let sent = proof.find("recovery_check_answer(").expect("then asks the server");
    assert!(load < sent, "read, then ask:\n{proof}");
    for plain_copy in ["from_bytes", "[u8; 32]", "drop(", "*key"] {
        assert!(
            !proof.contains(plain_copy),
            "the proof makes no plain copy of the key ({plain_copy}):\n{proof}"
        );
    }
    assert!(
        proof.contains("Err(_) => reauth::KeyProof::Unavailable"),
        "an error from the server settles nothing:\n{proof}"
    );
    let read = code_of(&production, "fn read_vault_key<");
    assert!(
        read.contains("-> Result<Option<beebeeb_core::kdf::MasterKey>, String>"),
        "the key leaves as a MasterKey:\n{read}"
    );
    assert!(
        read.contains("let mut raw = zeroize::Zeroizing::new([0u8; 32]);"),
        "the only plain buffer is a wiped one:\n{read}"
    );
    assert!(
        read.contains("beebeeb_core::kdf::MasterKey::from_bytes(*raw)"),
        "{read}"
    );
    assert!(
        code_of(&production, "fn load_stored_vault_key(")
            .contains("-> Result<Option<beebeeb_core::kdf::MasterKey>, String>")
    );
}

/// R10: a same-account re-sign-in records the owner before anything else changes.
#[test]
fn a_same_account_re_sign_in_records_the_owner_first() {
    let production = production_source();
    let body = code_of(&production, "fn reauth_swap_token(");
    let record = body.find("record_local_data_owner(").expect("the owner is recorded");
    assert!(record < body.find("persist_session_token_to_keychain(").expect("then the token"));
    assert!(record < body.find("replace_session_token_in_memory(").expect("then memory"));
}

/// A failure counts as "the token is not stored" up to and including the Keychain write, and as "stored" after it:
/// the owner record and the Keychain write say `before_store`, every later step says `after_store`. (The Keychain
/// write itself cannot be made to fail in a test, so this is pinned as source; the two ends are behaviour in
/// `a_re_sign_in_that_fails_says_whether_its_token_was_stored`.)
#[test]
fn the_swap_calls_its_token_stored_exactly_when_the_keychain_write_has_succeeded() {
    let production = production_source();
    let swap = code_of(&production, "fn reauth_swap_token(");
    let keychain = swap
        .find("persist_session_token_to_keychain(")
        .expect("the Keychain write");
    let (before, after) = swap.split_at(keychain);
    let line_end = after
        .find(".map_err(ReauthError::before_store)?;")
        .expect("the write's failure is before_store");
    assert!(
        !after[..line_end].contains("after_store"),
        "the Keychain write's own failure is not 'stored':\n{after}"
    );
    assert!(
        before.contains("record_local_data_owner(") && before.contains(".map_err(ReauthError::before_store)?;"),
        "{before}"
    );
    assert!(
        !before.contains("after_store"),
        "nothing before the Keychain write is 'stored':\n{before}"
    );
    let later = &after[line_end + ".map_err(ReauthError::before_store)?;".len()..];
    assert!(
        !later.contains("before_store"),
        "nothing after the Keychain write is 'not stored':\n{later}"
    );
    assert!(
        later.matches("ReauthError::after_store").count() >= 3,
        "every later failure is 'stored':\n{later}"
    );
}

/// The account check looks at every retained trace, not only the token.
#[test]
fn the_account_check_counts_every_retained_trace() {
    let production = production_source();
    let facts = code_of(&production, "fn gather_local_facts(");
    for probe in [
        "acct.session.lock()",
        "state.auth_present.lock()",
        "keychain_session_trace(",
        "keychain_email_trace(",
        "keychain_vault_key_present(",
        "local_data_owner(",
        "acct.cached_profile.lock()",
        "queued_or_staged_present(",
    ] {
        assert!(facts.contains(probe), "gather_local_facts must use {probe}");
    }
    assert!(code_of(&production, "fn classify_sign_in_after(").contains("traces: facts.traces.any()"));
    assert!(
        code_of(&production, "fn classify_sign_in(")
            .contains("classify_sign_in_after(state, acct, sources, profile, None)"),
        "one place gathers the facts"
    );
    // An unreadable source is a trace, never an absence: each of the three locks, on its own line, so flipping any
    // ONE of them fails this pin (`a_poisoned_lock_counts_as_a_trace_in_each_of_its_three_places` is the behaviour).
    for lock in [
        "session_in_memory: acct.session.lock().map(|guard| guard.is_some()).unwrap_or(true),",
        "auth_present: state.auth_present.lock().map(|guard| *guard).unwrap_or(true),",
        "cached_profile: acct.cached_profile.lock().map(|guard| guard.is_some()).unwrap_or(true),",
    ] {
        assert!(facts.contains(lock), "a poisoned lock counts as a trace: {lock}");
    }
    assert!(
        !facts.contains("keychain_session_present(") && !facts.contains("keychain_account_email("),
        "the fail-open readers are not the probes"
    );
    let queued = code_of(&production, "fn queued_or_staged_present(");
    assert!(
        queued.contains("Err(_) => true"),
        "an unreadable state.db counts as present:\n{queued}"
    );
}

/// Item 3 is about unowned local DATA only: rows of `state.db`, read through `has_account_data`, in a database whose
/// owner is not recorded. Credentials (the token, the Keychain email, the vault key, the cached profile) are not
/// data: they name an account for the identity order. A source that cannot be read is neither data nor an absence:
/// it is `unreadable`, and settles nothing.
#[test]
fn unowned_data_is_local_data_only_and_an_unreadable_source_settles_nothing() {
    let production = production_source();
    let data = code_of(&production, "fn unowned_local_data(");
    assert!(data.contains("-> Result<bool, String>"), "{data}");
    assert!(
        data.contains("match sources.state_db()? {"),
        "an unreadable database is an error, not an answer:\n{data}"
    );
    // Read through `squeeze`: rustfmt wraps this chain over lines.
    let squeezed_data = crate::source_pin::squeeze(&data);
    assert!(
        squeezed_data.contains(&crate::source_pin::squeeze(
            "Some(db) => db.has_account_data().map_err("
        )),
        "{data}"
    );
    assert!(
        data.contains("None => Ok(false),"),
        "no database yet is no data:\n{data}"
    );
    for credential in ["keychain", "session", "cached_profile", "auth_present", "AuthVault"] {
        assert!(
            !data.contains(credential),
            "unowned data does not look at credentials ({credential}):\n{data}"
        );
    }
    let facts = code_of(&production, "fn gather_local_facts(");
    assert!(
        facts.contains("let owner_unreadable = owner.is_err();"),
        "an owner record that cannot be read:\n{facts}"
    );
    assert!(
        crate::source_pin::squeeze(&facts).contains(&crate::source_pin::squeeze(
            "let data = if owner.is_some() { Ok(false) } else { unowned_local_data(sources) };"
        )),
        "{facts}"
    );
    assert!(facts.contains("unowned_data: matches!(data, Ok(true)),"), "{facts}");
    assert!(
        facts.contains("unreadable: owner_unreadable || data.is_err() || pending_changes.is_err(),"),
        "{facts}"
    );
    // Triage 25: the switch warning's count fails closed, never to 0.
    let count = code_of(&production, "fn pending_changes_count(");
    assert!(count.contains("-> Result<u64, String>"), "{count}");
    assert!(!count.contains("unwrap_or("), "an error is never a count:\n{count}");
    assert!(facts.contains("credentials: credentials_identity(acct)"), "{facts}");
    let classify = code_of(&production, "fn classify_sign_in_after(");
    assert!(
        classify.contains("unowned_data: facts.unowned_data,") && classify.contains("unreadable: facts.unreadable,"),
        "{classify}"
    );
    assert!(
        classify.contains("facts.owner.as_ref().unwrap_or(&facts.credentials)"),
        "an owner record wins; else the credentials name the account:\n{classify}"
    );
    let credentials = code_of(&production, "fn credentials_identity(");
    assert!(
        credentials.contains("acct.cached_profile.lock()") && credentials.contains("keychain_account_email("),
        "{credentials}"
    );
    assert!(
        !credentials.contains("master_key") && !credentials.contains("token"),
        "an identity is not a secret:\n{credentials}"
    );
}

/// The vault-key probe asks both Keychain layouts and never unlocks.
#[test]
fn the_vault_key_probe_asks_both_layouts_and_never_unlocks() {
    let production = production_source();
    let probe = code_of(&production, "fn keychain_vault_key_present(");
    assert!(
        probe.contains("platform_keychain_store_for(account_id)") && probe.contains("legacy_platform_keychain_store()"),
        "{probe}"
    );
    assert!(!probe.contains("unlock") && !probe.contains("AuthVault"), "{probe}");
    // The token and email probes: both layouts, through the fail-closed presence functions, never a read of the item.
    for (name, holds) in [
        ("fn keychain_session_trace(", "holds_session_token("),
        ("fn keychain_email_trace(", "holds_account_email("),
    ] {
        let body = code_of(&production, name);
        assert_eq!(
            body.matches(holds).count(),
            2,
            "{name} asks both layouts with {holds}:\n{body}"
        );
        assert!(
            body.contains("platform_keychain_store_for(account_id)")
                && body.contains("legacy_platform_keychain_store()"),
            "{body}"
        );
        assert!(
            !body.contains("load_")
                && !body.contains("AuthVault")
                && !body.contains(".ok()")
                && !body.contains("unwrap_or(false)"),
            "{name} must not fail open:\n{body}"
        );
    }
}

/// Every test that runs the real sign-out shares one scratch state directory, so the purge takes turns in test
/// builds (a gate inside `clear_session_impl`, before the purge), and no test retries around a busy database. A test
/// that works in that directory itself (the one that seeds it before it signs out) holds the same gate for its whole
/// body through `with_the_shared_state_dir`, and its own sign-out knows it does (a task-local marker), so it does not
/// wait for what it holds.
#[test]
fn the_sign_out_purge_takes_turns_in_tests_and_nothing_retries_around_it() {
    let source = crate::root_source::lib_rs().replace("\r\n", "\n");
    let clear = &source[source.find("async fn clear_session_impl(").unwrap()..];
    let clear = &clear[..clear.find("\n}\n").unwrap()];
    let held = "let _one_purge_at_a_time = if PURGE_GATE_HELD.try_with(|_| ()).is_ok() {\n            None\n        } else {\n            Some(SIGN_OUT_TEST_PURGE_GATE.lock().await)\n        };";
    let gate = clear
        .find(held)
        .expect("the purge takes turns, unless this test already holds the gate");
    assert!(
        clear[..gate].trim_end().ends_with("#[cfg(test)]"),
        "the gate is compiled into test builds only"
    );
    assert!(
        gate < clear.find("purge_local_data_for_sign_out(").expect("the purge"),
        "the gate is taken before the purge"
    );
    // The one test that seeds the shared database itself works under the gate, from its first write to its last check.
    let seeding_file = include_str!("../finder_lock_and_sign_out_tests.rs").replace("\r\n", "\n");
    let seeding = &seeding_file[seeding_file
        .find(concat!("fn a_sign_out_leaves_no_account_row_", "and_no_owner() {"))
        .expect("the seeding test")..];
    let seeding = &seeding[..seeding.find("\n    }\n").unwrap()];
    let under_the_gate = seeding
        .find("with_the_shared_state_dir(")
        .expect("the seeding test holds the gate");
    assert!(
        under_the_gate < seeding.find("StateDb::open(").expect("it opens the shared database"),
        "the gate comes before its first write:\n{seeding}"
    );
    assert!(
        under_the_gate < seeding.find("has_account_data()").unwrap(),
        "{seeding}"
    );
    assert!(
        source.contains("static SIGN_OUT_TEST_PURGE_GATE: tokio::sync::Mutex<()>"),
        "the gate is one process-wide lock"
    );
    let source = super::finder_setup_command_tests::with_test_modules(&source);
    assert!(
        !source.contains(concat!("sign_out_", "retrying")),
        "no retry around a busy database is left"
    );
}

/// Ruling 3 (Task 11), carried by lead ruling 1 of Task 12: the unlock re-checks the account before it writes the
/// key and the email, in the same checked turn as every other session write, and puts the session in memory before
/// that turn ends. Every writer of a session takes the turn as a parameter, so none can write outside one; only the
/// claim and the end of transitions make one.
#[test]
fn the_unlock_rechecks_the_account_and_every_session_writer_takes_the_lock() {
    let production = production_source();
    let unlock = code_of(&production, "async fn desktop_unlock_with_recovery_phrase(");
    let verified = unlock.find("verify_vault_key_from_phrase(").expect("the server check");
    let installed = unlock
        .find("install_recovered_session(&state, &acct, &mut turn,")
        .expect("the key goes through the checked install");
    assert!(
        verified < installed,
        "the key is written after the server check, through the checked install:\n{unlock}"
    );
    assert!(
        !unlock.contains("persist_vault_key_to_keychain(") && !unlock.contains("store_account_email("),
        "no unchecked write of the key or the email:\n{unlock}"
    );
    assert!(
        unlock.find("let mut turn = acct.session_generation();").unwrap()
            < unlock.find("load_session_token_from_keychain(").unwrap(),
        "captured before the Keychain is read"
    );
    let install = code_of(&production, "fn install_recovered_session(");
    let stored = install
        .find("let write = store_recovered_vault_key(acct, turn,")
        .expect("the checked writer");
    let in_memory = install
        .find("*guard = Some(Session {")
        .expect("the session goes into memory");
    let released = install.find("drop(write);").expect("and the turn ends");
    assert!(
        stored < in_memory && in_memory < released,
        "the install happens inside the unlock's turn:\n{install}"
    );
    let helper = code_of(&production, "fn store_recovered_vault_key(");
    let claim = helper
        .find("claim_session_write(acct, turn)")
        .expect("the writer takes the turn");
    let token = helper
        .find("load_session_token_from_keychain(")
        .expect("it re-reads the stored token");
    let write = helper
        .find("persist_vault_key_to_keychain(&write, ")
        .expect("then writes the key");
    assert!(claim < token && token < write, "turn, re-check, write:\n{helper}");
    assert!(
        helper.find("UNLOCK_ACCOUNT_CHANGED").unwrap() < write,
        "a changed account stops before the write:\n{helper}"
    );
    for signature in [
        "fn persist_session_to_keychain(",
        "fn persist_session_token_to_keychain(",
        "fn persist_vault_key_to_keychain(",
        "fn install_unlocked_session(",
        "fn clear_keychain_session_token(",
        "fn name_session_after_profile(",
        "fn backfill_local_data_owner(",
        "fn clear_keychain_session_holding(",
        "fn replace_session_token_in_memory(",
        "fn name_session_in_memory(",
        "fn delete_vault_key_from_keychain(",
    ] {
        let function = &production[production.find(signature).unwrap_or_else(|| panic!("{signature}"))..];
        let parameters = &function[..function.find(") ->").unwrap()];
        assert!(
            parameters.contains("write: &SessionWrite"),
            "{signature} can only write inside a checked turn:\n{parameters}"
        );
    }
    // The same for the one writer with no return value (triage 10, P3-C).
    let settle = &production[production.find("fn reauth_settle_flags(").expect("the flags writer")..];
    let parameters = &settle[..settle.find(") {").unwrap()];
    assert!(
        parameters.contains("write: &SessionWrite"),
        "reauth_settle_flags can only write inside a checked turn:\n{parameters}"
    );
    // The naming checks its turn and moves the generation on only when it named something (fix round 1, item 5).
    let naming = code_of(&production, "async fn establish_session_identity(");
    let checked = naming
        .find("check_session_turn(acct, &turn)")
        .expect("the naming checks its turn");
    let named = naming
        .find("let named = name_session_after_profile(&write,")
        .expect("then names");
    let moved = naming.find("if named {\n        acct.advance_session_generation_holding_session_write_lock(account::SessionTransition::Write);").expect("and moves the generation only then");
    assert!(checked < named && named < moved, "{naming}");
    assert!(
        !naming.contains("claim_session_write("),
        "a naming that names nothing moves nothing:\n{naming}"
    );
    assert!(
        naming.find("let turn = acct.session_generation();").unwrap() < naming.find("acct.session.lock()").unwrap(),
        "captured before the session is read"
    );
    // The Keychain email retry (fix round 2, item 3) writes in a turn captured before it read the session's email.
    let retry = code_of(&production, "async fn identify_unidentified_session(");
    let captured = retry
        .find("let turn = acct.session_generation();")
        .expect("the retry captures its turn");
    assert!(
        captured < retry.find("acct.session.lock()").unwrap(),
        "captured before the session is read:\n{retry}"
    );
    assert!(
        retry.contains("&& let Ok(write) = check_session_turn(&acct, &turn)"),
        "and checked with it:\n{retry}"
    );
    assert_eq!(
        retry.matches("session_generation()").count(),
        1,
        "never re-captured at the check:\n{retry}"
    );
    assert_eq!(
        production.matches("lock_session_keychain_writes()").count(),
        5,
        "the definition, the check, the end of transitions, the sign-out's last turn and the full Keychain clear"
    );
    // Struct literals only. Read through `squeeze`, since rustfmt wraps some of the literals over lines; the
    // definition (`struct SessionWrite { _writes: ..`) squeezes to the same text and is taken off.
    let squeezed = crate::source_pin::squeeze(&production);
    assert_eq!(
        squeezed.matches("SessionWrite{_writes:").count() - squeezed.matches("structSessionWrite{_writes:").count(),
        4,
        "only the check (the claim goes through it), the end of transitions, the sign-out's last turn and the full Keychain clear make a turn"
    );
    // The claim is the check plus the move, and every refusal names what moved the generation (item 8).
    let claim = code_of(&production, "fn claim_session_write(");
    assert!(
        claim.contains("let write = check_session_turn(acct, turn)?;"),
        "{claim}"
    );
    let check = code_of(&production, "fn check_session_turn(");
    assert!(check.contains("return Err(SessionChanged::of(acct));"), "{check}");
}

// ---- behaviour ----
