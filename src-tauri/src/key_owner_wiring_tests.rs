//! Task 2447: the call sites of the ownership rules. A rule that no command calls protects nothing, so each call
//! site has a test here that goes red when its call is deleted: the store-level cores (`*_over`, the real loader, the
//! real migration) run over the in-memory store; the commands themselves need a window system or the platform
//! store, so their calls are pinned by reading the source (order and arguments). Then the two stories end to end:
//! the upgrade from 0.8.24 without the recovery phrase, and the stale key next to another account's token.

use super::*;
use crate::source_pin::squeeze;
use crate::{layout_holding_session, settle_owner_after_probe_over, sign_in_adds_key_over};

/// The production part of the crate root, children spliced back in (`pub(super) ` dropped, as `root_source` does).
fn production() -> String {
    let source = crate::root_source::lib_rs().replace("\r\n", "\n");
    source[..source.find("#[cfg(test)]\nmod tests;").expect("the test modules")].to_string()
}

/// The function whose signature starts with `signature`, to its closing brace, without layout.
fn code_of(production: &str, signature: &str) -> String {
    let at = production
        .find(signature)
        .unwrap_or_else(|| panic!("{signature} exists"));
    let body = &production[at..];
    squeeze(&body[..body.find("\n}\n").expect("the function ends")])
}

/// Where `needle` (squeezed) starts in `code`; panics with `what` when the call is gone.
fn at(code: &str, needle: &str, what: &str) -> usize {
    code.find(&squeeze(needle))
        .unwrap_or_else(|| panic!("{what}: `{needle}` is gone"))
}

fn empty() -> MemStore {
    MemStore::default()
}

// ── the commands' calls, read from the source ──────────────────────────────────────────────

#[test]
fn a_first_sign_in_removes_a_foreign_key_then_stores_the_token_then_names_its_owner() {
    let store = code_of(&production(), "fn store_first_sign_in(");
    let turn = at(&store, "claim_session_write(acct, turn)?;", "the sign-in's turn");
    let remove = at(
        &store,
        "remove_foreign_vault_key_before_sign_in(&write, acct.id.as_str(), &profile.user_id, email)?;",
        "the foreign-key removal, with the server's id and email, refusing the sign-in when it fails",
    );
    let token = at(
        &store,
        "persist_session_token_to_keychain(&write, acct.id.as_str(), token, Some(email))?;",
        "the token's write",
    );
    let bind = at(
        &store,
        "bind_session_owner_in_keychain(&write, acct.id.as_str(), &profile.user_id);",
        "the session's owner, from this sign-in's own answer",
    );
    let flags = at(&store, "set_auth_present(state, true);", "the flags");
    assert!(
        turn < remove && remove < token && token < bind && bind < flags,
        "order: turn, removal, token, owner, flags"
    );
}

#[test]
fn the_startup_restore_finishes_a_failed_clear_first_and_judges_the_key_by_the_servers_id() {
    let restore = code_of(&production(), "async fn restore_stored_session(");
    let pending = at(
        &restore,
        "#[cfg(target_os = \"windows\")] if let Ok(write) = claim_session_write(acct, &mut turn) { complete_pending_keychain_clear(&write, acct.id.as_str()); }",
        "Windows finishes a failed clear in the restore's turn",
    );
    let load = at(
        &restore,
        "load_session_from_keychain(acct.id.as_str(), email)",
        "the load",
    );
    assert!(pending < load, "the pending clear runs before anything is read");
    let probe = at(
        &restore,
        "probe_startup_session(base_url, &session.token).await",
        "the probe",
    );
    let judge = at(
        &restore,
        "if let Some(profile) = probed.as_ref() && !settle_owner_after_probe_in_keychain(&write, acct.id.as_str(), token.as_str(), &profile.user_id).may_load() {",
        "the key is judged against the id the probe returned for the stored token",
    );
    let install = at(&restore, "*guard = Some(session)", "the install");
    assert!(
        probe < judge && judge < install,
        "judged after the probe and before the install"
    );
    let refused = &restore[judge..install];
    assert!(
        refused.contains("returnfalse;"),
        "a key the server's id does not match is not installed"
    );
}

#[test]
fn a_sign_in_while_signed_in_is_refused_on_windows_unless_it_brings_the_same_accounts_key() {
    let apply = code_of(&production(), "pub(crate) async fn apply_session(");
    let present = at(
        &apply,
        "#[cfg(target_os = \"windows\")] if state.auth_present.lock().map(|present| *present).unwrap_or(true) {",
        "Windows asks whether a session is present",
    );
    let locked = at(
        &apply,
        "let vault_unlocked = acct.session.lock().map(|session| session.is_some()).unwrap_or(true);",
        "locked is read from the session in memory, and a poisoned lock counts as unlocked",
    );
    let exception = at(
        &apply,
        "if !sign_in_adds_key_to_signed_in_session(acct.id.as_str(), profile.as_ref(), vault_unlocked) { return Err(\"Sign out of the current account before signing in again.\".into()); }",
        "the equal-ids exception, refusing with the unchanged sentence",
    );
    let install = at(&apply, "install_new_session_or_revoke(", "the install");
    assert!(present < locked && locked < exception && exception < install);
}

#[test]
fn a_session_that_arrives_with_its_key_is_bound_to_the_servers_id_before_it_reaches_memory() {
    let install = code_of(&production(), "fn install_new_session(");
    let write = at(
        &install,
        "persist_session_to_keychain(&write, acct.id.as_str(), token, master_key, email.as_deref())?;",
        "the token and key write",
    );
    let bind = at(
        &install,
        "if let Some(profile) = profile.as_ref() { bind_session_and_key_owner_in_keychain(&write, acct.id.as_str(), &profile.user_id); }",
        "both owners, from the profile fetched for the arriving token",
    );
    let memory = at(&install, "*guard = Some(Session {", "the session in memory");
    assert!(write < bind && bind < memory);
}

#[test]
fn a_recovered_key_takes_the_stored_sessions_owner_in_the_turn_that_checked_the_token() {
    let store = code_of(&production(), "fn store_recovered_vault_key(");
    let checked = at(
        &store,
        "if stored.as_ref().map(|token| token.as_str()) != Some(started_token)",
        "the stored token is the one the proof was made with",
    );
    let key = at(
        &store,
        "persist_vault_key_to_keychain(&write, account_id, master_key)?;",
        "the key's write",
    );
    let bind = at(
        &store,
        "bind_vault_key_to_session_owner_in_keychain(&write, account_id);",
        "the key's owner",
    );
    assert!(checked < key && key < bind);
}

#[test]
fn both_re_sign_in_swaps_name_the_new_token() {
    let production = production();
    for swap in ["fn reauth_swap_token(", "fn reauth_swap_token_without_key("] {
        let code = code_of(&production, swap);
        let token = at(
            &code,
            "persist_session_token_to_keychain(&write, acct.id.as_str(), token, email.as_deref())",
            swap,
        );
        let bind = at(
            &code,
            "bind_session_owner_in_keychain(&write, acct.id.as_str(), &profile.user_id);",
            swap,
        );
        assert!(token < bind, "{swap}: the owner is bound after the token is stored");
        if let Some(load) = code.find(&squeeze("load_session_from_keychain(")) {
            assert!(bind < load, "{swap}: and before the kept key is loaded");
        }
    }
}

#[test]
fn the_one_guard_sits_inside_the_loader_before_the_key_is_read() {
    let production = production();
    let loader = code_of(&production, "fn load_session_from_vault<");
    let guard = at(
        &loader,
        "if !crate::key_owner::stored_key_ownership(&vault).map_err(|e| keychain_error(\"read the vault key's owner\", e))?.may_load() { return Err(crate::key_owner::KEY_NOT_THIS_ACCOUNTS.to_string()); }",
        "the loader's guard",
    );
    assert!(guard < at(&loader, ".unlock()", "the key's read"));
    // Every reader of token + key ends in that loader: the two layouts of `load_session_from_keychain`.
    let keychain = code_of(&production, "fn load_session_from_keychain(");
    assert_eq!(keychain.matches("returnload_session_from_vault(").count(), 2);
    assert!(
        !keychain.contains(".unlock()"),
        "no layout reads the key outside the loader"
    );
    // The one other read of a stored key (`read_vault_key`, macOS and Linux) reads no token and installs nothing: it
    // hands the key to the server proof that decides whether the signing-in account may use it at all.
    assert_eq!(squeeze(&production).matches(".unlock()").count(), 2);
    let proof = code_of(&production, "fn read_vault_key<");
    assert!(proof.contains(".unlock()") && !proof.contains("session_token") && !proof.contains("Session{"));
}

#[test]
fn a_refused_key_sends_a_mac_to_the_recovery_phrase_like_a_missing_one() {
    assert!(
        code_of(&production(), "fn unlock_failure(").contains(&squeeze(
            "#[cfg(target_os = \"macos\")] if error.contains(\"secret not found\") || error == crate::key_owner::KEY_NOT_THIS_ACCOUNTS { return NO_VAULT_KEY_UNLOCK_ERROR.to_string(); }"
        )),
        "the Mac's unlock maps the loader's refusal to its no-key answer"
    );
    let refused = crate::unlock_failure(KEY_NOT_THIS_ACCOUNTS.to_string());
    if cfg!(target_os = "macos") {
        assert_eq!(refused, crate::NO_VAULT_KEY_UNLOCK_ERROR);
    } else {
        assert_eq!(
            refused, KEY_NOT_THIS_ACCOUNTS,
            "Windows and Linux show the loader's sentence"
        );
    }
}

#[test]
fn no_owner_id_comes_from_a_cached_profile_and_none_is_logged() {
    for (name, source) in [
        ("key_owner.rs", include_str!("key_owner.rs")),
        ("session_owner.rs", include_str!("session_owner.rs")),
    ] {
        assert!(
            !source.contains("cached_profile"),
            "{name}: ids come from a server answer"
        );
        let code = squeeze(source);
        let mut rest = code.as_str();
        while let Some(start) = rest.find("tracing::") {
            let call = &rest[start..];
            let call = &call[..call.find(");").map_or(call.len(), |end| end + 2)];
            for value in [
                "user_id",
                "expose_for",
                "probed_token",
                "%token",
                "?token",
                "%owner",
                "?owner",
                "%signed_in",
                "?signed_in",
            ] {
                assert!(!call.contains(value), "{name}: a log line carries `{value}`: {call}");
            }
            rest = &rest[start + "tracing::".len()..];
        }
    }
}

/// The platform forms are one call each: the core, over the id-keyed layout (a write) or both layouts (a read).
#[test]
fn every_platform_form_calls_its_core_on_the_right_layout() {
    let source = squeeze(include_str!("session_owner.rs"));
    for (form, call) in [
        (
            "fn remove_foreign_vault_key_before_sign_in(",
            "let mut vault = AuthVault::new(platform_keychain_store_for(account_id)); remove_foreign_key_or_refuse(&mut vault, signing_in_user_id, signing_in_email)",
        ),
        (
            "fn bind_session_owner_in_keychain(",
            "AuthVault::new(platform_keychain_store_for(account_id)).bind_session_owner(user_id)",
        ),
        (
            "fn bind_session_and_key_owner_in_keychain(",
            "key_owner::bind_session_and_key(&AuthVault::new(platform_keychain_store_for(account_id)), user_id)",
        ),
        (
            "fn bind_vault_key_to_session_owner_in_keychain(",
            "key_owner::bind_key_to_session_owner(&AuthVault::new(platform_keychain_store_for(account_id)))",
        ),
        (
            "fn settle_owner_after_probe_in_keychain(",
            "settle_owner_after_probe_over(both_layouts(account_id), probed_token, server_user_id)",
        ),
        (
            "fn complete_pending_keychain_clear(",
            "complete_pending_clear_over(both_layouts(account_id));",
        ),
        (
            "fn sign_in_adds_key_to_signed_in_session(",
            "sign_in_adds_key_over(both_layouts(account_id), profile.map(|profile| profile.user_id.as_str()), vault_unlocked)",
        ),
    ] {
        let at = source.find(&squeeze(form)).unwrap_or_else(|| panic!("{form} exists"));
        let body = &source[at..];
        let body = &body[..body[1..].find("pub(crate)fn").map_or(body.len(), |end| end + 1)];
        assert!(body.contains(&squeeze(call)), "{form}: `{call}` is gone");
    }
    assert!(
        source.contains(&squeeze(
            "[AuthVault::new(platform_keychain_store_for(account_id)), AuthVault::new(keychain::legacy_platform_keychain_store())]"
        )),
        "both layouts: the id-keyed one first, as the loader reads them"
    );
}

/// The sign-in's removal, as the command gets it: a key that cannot be removed is the refusal, nothing else is.
#[test]
fn a_foreign_key_that_cannot_be_removed_refuses_the_sign_in_with_its_own_sentence() {
    let store = signed_in_as_a();
    store.vault().clear_session_token().unwrap();
    store.fail_deletes(KEY, true);
    assert_eq!(
        crate::remove_foreign_key_or_refuse(&mut store.vault(), "user-b", "b@beebeeb.io"),
        Err(FOREIGN_KEY_NOT_REMOVED.to_string())
    );
    assert!(store.has(KEY));
    store.fail_deletes(KEY, false);
    assert_eq!(
        crate::remove_foreign_key_or_refuse(&mut store.vault(), "user-b", "b@beebeeb.io"),
        Ok(())
    );
    assert!(!store.has(KEY) && !store.has(KEY_OWNER));
    // The account's own key, and an empty store: nothing to refuse, nothing removed.
    let own = signed_in_as_a();
    assert_eq!(
        crate::remove_foreign_key_or_refuse(&mut own.vault(), "user-a", "a@beebeeb.io"),
        Ok(())
    );
    assert!(own.has(KEY));
    assert_eq!(
        crate::remove_foreign_key_or_refuse(&mut empty().vault(), "user-b", "b@beebeeb.io"),
        Ok(())
    );
}

// ── the store-level cores the commands call ────────────────────────────────────────────────

#[test]
fn the_owner_is_read_from_the_layout_that_holds_the_token() {
    let (id_keyed, flat) = (empty(), signed_in_as_a());
    let holding = layout_holding_session([id_keyed.vault(), flat.vault()])
        .unwrap()
        .expect("the flat layout holds the session");
    assert_eq!(holding.session_owner().unwrap().as_deref(), Some("user-a"));
    assert!(
        layout_holding_session([empty().vault(), empty().vault()])
            .unwrap()
            .is_none()
    );
    // The id-keyed layout wins when it holds a token, as in the loader.
    let newer = MemStore::default();
    put_token(&newer, "tok-b").unwrap();
    newer.vault().bind_session_owner("user-b").unwrap();
    let holding = layout_holding_session([newer.vault(), flat.vault()]).unwrap().unwrap();
    assert_eq!(holding.session_owner().unwrap().as_deref(), Some("user-b"));
    // A layout that cannot answer is an error, never "no token here, ask the next one".
    let unreadable = signed_in_as_a();
    unreadable.fail_reads(TOKEN);
    assert!(layout_holding_session([unreadable.vault(), flat.vault()]).is_err());
}

#[test]
fn the_restore_settles_the_owner_in_the_layout_the_loader_read() {
    // A flat (pre-0800) layout from before the tags: named there, the key stays unknown and loadable.
    let (id_keyed, flat) = (empty(), legacy_a());
    let settled = settle_owner_after_probe_over([id_keyed.vault(), flat.vault()], "tok-a", "user-a");
    assert_eq!(settled, KeyOwnership::Unknown);
    assert!(settled.may_load());
    assert_eq!(flat.vault().session_owner().unwrap().as_deref(), Some("user-a"));
    assert!(
        !id_keyed.has(SESSION_OWNER),
        "nothing is written to the layout without the token"
    );

    // The server names another account than the key's: not installed.
    let foreign = signed_in_as_a();
    assert!(!settle_owner_after_probe_over([foreign.vault(), empty().vault()], "tok-a", "user-b").may_load());

    // A store that cannot say, and a token that is gone: not a match, nothing installed.
    let unreadable = signed_in_as_a();
    unreadable.fail_reads(KEY_OWNER);
    assert_eq!(
        settle_owner_after_probe_over([unreadable.vault(), empty().vault()], "tok-a", "user-a"),
        KeyOwnership::SessionUnknown
    );
    assert_eq!(
        settle_owner_after_probe_over([empty().vault(), empty().vault()], "tok-a", "user-a"),
        KeyOwnership::SessionUnknown
    );
}

#[test]
fn a_failed_clear_is_finished_in_every_layout_whatever_the_first_one_answers() {
    let (id_keyed, flat) = (signed_in_as_a(), signed_in_as_a());
    for store in [&id_keyed, &flat] {
        store.fail_deletes(KEY, true);
        assert!(store.vault().clear_session().is_err());
    }
    flat.fail_deletes(KEY, false);
    assert_eq!(
        crate::complete_pending_clear_over([id_keyed.vault(), flat.vault()]),
        1,
        "the id-keyed layout still refuses"
    );
    assert!(id_keyed.has(KEY), "still pending");
    assert!(
        !flat.has(KEY) && !flat.has(KEY_OWNER),
        "the flat layout was finished all the same"
    );
    id_keyed.fail_deletes(KEY, false);
    assert_eq!(crate::complete_pending_clear_over([id_keyed.vault(), flat.vault()]), 0);
    assert!(!id_keyed.has(KEY) && !id_keyed.has(KEY_OWNER));
    // A live session is left alone.
    let live = signed_in_as_a();
    assert_eq!(crate::complete_pending_clear_over([live.vault(), empty().vault()]), 0);
    assert!(live.has(TOKEN) && live.has(KEY));
}

#[test]
fn a_key_is_added_to_a_signed_in_session_only_for_the_id_recorded_at_its_sign_in() {
    // A password sign-in waiting for its key, as `store_first_sign_in` leaves it.
    let waiting = MemStore::default();
    token_only_sign_in(&waiting, "tok-a", "user-a", "a@beebeeb.io").unwrap();
    let layouts = || [waiting.vault(), empty().vault()];
    assert!(sign_in_adds_key_over(layouts(), Some("user-a"), false));
    assert!(
        !sign_in_adds_key_over(layouts(), Some("user-b"), false),
        "another account"
    );
    assert!(
        !sign_in_adds_key_over(layouts(), Some("user-a"), true),
        "already unlocked"
    );
    assert!(
        !sign_in_adds_key_over(layouts(), None, false),
        "the arriving session has no profile"
    );
    // A session nobody named (stored by 0.8.24, never probed since) allows nothing.
    let unnamed = legacy_a();
    assert!(!sign_in_adds_key_over(
        [unnamed.vault(), empty().vault()],
        Some("user-a"),
        false
    ));
    // Nor does a store that cannot answer, or one with no session at all.
    waiting.fail_reads(SESSION_OWNER);
    assert!(!sign_in_adds_key_over(layouts(), Some("user-a"), false));
    assert!(!sign_in_adds_key_over(
        [empty().vault(), empty().vault()],
        Some("user-a"),
        false
    ));
}

#[test]
fn the_layout_migration_never_moves_a_session_under_the_tags_of_the_one_before() {
    // The id-keyed layout kept A's key and tags after a rejected token (a Mac does); the flat one holds B whole.
    let id_keyed = signed_in_as_a();
    id_keyed.vault().clear_session_token().unwrap();
    id_keyed.vault().bind_session_owner("user-a").unwrap();
    let flat = MemStore::default();
    put_token(&flat, "tok-b").unwrap();
    put_key(&flat, &KEY_B).unwrap();
    crate::keychain::migrate_legacy_keychain_between(&id_keyed, &flat).unwrap();
    assert_eq!(id_keyed.text(TOKEN).unwrap().as_deref(), Some("tok-b"));
    assert_eq!(id_keyed.load(KEY).unwrap().unwrap(), KEY_B.to_vec());
    assert!(
        !id_keyed.has(KEY_OWNER) && !id_keyed.has(SESSION_OWNER),
        "B's token and key are not tagged user-a"
    );
    assert_eq!(stored_key_ownership(&id_keyed.vault()), Ok(KeyOwnership::Unknown));

    // A tag that cannot be removed stops the migration before the flat token is written.
    for stuck_tag in [KEY_OWNER, SESSION_OWNER] {
        let id_keyed = signed_in_as_a();
        id_keyed.vault().clear_session_token().unwrap();
        id_keyed.vault().bind_session_owner("user-a").unwrap();
        let flat = MemStore::default();
        put_token(&flat, "tok-b").unwrap();
        put_key(&flat, &KEY_B).unwrap();
        id_keyed.fail_deletes(stuck_tag, true);
        assert!(crate::keychain::migrate_legacy_keychain_between(&id_keyed, &flat).is_err());
        assert!(
            !id_keyed.has(TOKEN),
            "{stuck_tag}: B's token did not land under A's tag"
        );
        assert_eq!(id_keyed.load(KEY).unwrap().unwrap(), KEY_A.to_vec());
        assert!(
            flat.has(TOKEN) && flat.has(KEY),
            "{stuck_tag}: the flat layout is untouched"
        );
    }
}

// ── the upgrade from 0.8.24: no recovery phrase, online or offline ─────────────────────────

/// What 0.8.24 leaves for a signed-in, unlocked account: token, key and email, no tags (`legacy_a`). 0.8.25 starts on
/// it: the loader gives the session (an engine start reads only the session in memory), with and without the server.
#[test]
fn an_install_upgraded_from_0_8_24_starts_unlocked_online_and_offline_without_the_phrase() {
    for flat_layout in [false, true] {
        let store = legacy_a();
        let layouts = || {
            if flat_layout {
                [empty().vault(), store.vault()]
            } else {
                [store.vault(), empty().vault()]
            }
        };
        // Offline: the restore loads, the probe is inconclusive, nothing is judged, the session is installed.
        let session = load(&store).unwrap().expect("offline: the stored session loads");
        assert_eq!(
            (session.token.as_str(), session.master_key, session.email.as_deref()),
            ("tok-a", KEY_A, Some("a@beebeeb.io"))
        );
        assert!(
            !store.has(KEY_OWNER) && !store.has(SESSION_OWNER),
            "a load writes no tag"
        );
        // Online: the probe names the account; the untagged key may still be installed.
        assert!(settle_owner_after_probe_over(layouts(), "tok-a", "user-a").may_load());
        assert_eq!(store.vault().session_owner().unwrap().as_deref(), Some("user-a"));
        // Every later start, offline again: still loads (the session is named, the key is of unknown owner).
        assert_eq!(stored_key_ownership(&store.vault()), Ok(KeyOwnership::Unknown));
        assert_eq!(load(&store).unwrap().expect("the next start loads").master_key, KEY_A);
        // A verified install (the recovery check stores the key it proved) tags it; it keeps loading.
        put_key(&store, &KEY_A).unwrap();
        assert_eq!(bind_key_to_session_owner(&store.vault()), Ok(true));
        assert_eq!(stored_key_ownership(&store.vault()), Ok(KeyOwnership::Matches));
        assert_eq!(load(&store).unwrap().expect("and after").master_key, KEY_A);
    }
}

/// The same upgrade for a pre-0800 flat layout: 0.8.25's first start migrates it, and the migrated session loads.
#[test]
fn an_upgraded_flat_layout_migrates_and_still_loads_without_the_phrase() {
    let (id_keyed, flat) = (empty(), legacy_a());
    crate::keychain::migrate_legacy_keychain_between(&id_keyed, &flat).unwrap();
    assert!(!flat.has(TOKEN) && !flat.has(KEY));
    assert_eq!(
        load(&id_keyed).unwrap().expect("the migrated session").master_key,
        KEY_A
    );
    assert!(settle_owner_after_probe_over([id_keyed.vault(), flat.vault()], "tok-a", "user-a").may_load());
}

// ── the attack (see task 2447): another account's token next to a key that stayed behind ───

/// 0.8.24's clear stopped at the first refusal, so what it could leave is A's key (and email) with no token, and no
/// tags. Each defence alone ends it: the next start's pending clear (Windows), and the sign-in's own removal.
#[test]
fn a_key_0_8_24_left_behind_never_reaches_the_next_accounts_session() {
    let left_behind = || {
        let store = legacy_a();
        store.delete(TOKEN).unwrap();
        store
    };
    // The start before the sign-in (Windows): no token, a key: finished.
    let store = left_behind();
    assert_eq!(crate::complete_pending_clear_over([store.vault(), empty().vault()]), 0);
    assert!(!store.has(KEY) && !store.has(EMAIL));

    // Without that start (the store refused again, or macOS): B's password sign-in removes it.
    let store = left_behind();
    assert_eq!(token_only_sign_in(&store, "tok-b", "user-b", "b@beebeeb.io"), Ok(true));
    assert!(!store.has(KEY));
    assert!(
        load(&store).unwrap_err().contains("secret not found"),
        "no key: the recovery phrase"
    );
    assert!(settle_owner_after_probe_over([store.vault(), empty().vault()], "tok-b", "user-b").may_load());
    assert!(!store.has(KEY), "and nothing came back");

    // A, signing in again on the key it left: kept, loads, no phrase.
    let store = left_behind();
    assert_eq!(
        token_only_sign_in(&store, "tok-a2", "user-a", "a@beebeeb.io"),
        Ok(false)
    );
    assert_eq!(load(&store).unwrap().expect("A's own key").master_key, KEY_A);
}

/// A tagged key (stored by 0.8.25) next to another account's token, however the token got there: the sign-in removes
/// it; a token written past the sign-in's check is stopped by the loader, online by the probe as well.
#[test]
fn a_tagged_key_is_never_loaded_for_another_accounts_token() {
    let store = signed_in_as_a();
    store.fail_deletes(KEY, true);
    assert!(store.vault().clear_session().is_err());
    store.fail_deletes(KEY, false);
    assert_eq!(token_only_sign_in(&store, "tok-b", "user-b", "a@beebeeb.io"), Ok(true));
    assert!(!store.has(KEY) && !store.has(KEY_OWNER));

    // Past the sign-in's check: B's token and name beside A's tagged key.
    let paired = signed_in_as_a();
    put_token(&paired, "tok-b").unwrap();
    paired.vault().bind_session_owner("user-b").unwrap();
    assert_eq!(load(&paired).unwrap_err(), KEY_NOT_THIS_ACCOUNTS);
    // The token is B's whatever the store says of it (the tag was lost): unnamed is refused offline, and the
    // server's answer refuses it online.
    paired.delete(SESSION_OWNER).unwrap();
    assert_eq!(load(&paired).unwrap_err(), KEY_NOT_THIS_ACCOUNTS);
    assert!(!settle_owner_after_probe_over([paired.vault(), empty().vault()], "tok-b", "user-b").may_load());
    // The recovery phrase then replaces the refused key (its tag goes first) and the session loads.
    put_key(&paired, &KEY_B).unwrap();
    assert_eq!(bind_key_to_session_owner(&paired.vault()), Ok(true));
    assert_eq!(load(&paired).unwrap().expect("B's own key").master_key, KEY_B);
}
