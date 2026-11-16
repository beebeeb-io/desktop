//! The choke point of a person's call (`Engine::person_session`) and the census that keeps it the only one. The
//! behaviour behind a locked screen is in `tests/host_contract.rs`, against a session a real sign-in installed.

use std::sync::Arc;

use super::super::{Engine, EngineConfig};
use super::PersonRefused;
use crate::account::SessionTransition;
use crate::host::ChannelHost;
use crate::identity::REAL;
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;

fn engine_with_a_session() -> Arc<Engine<MemoryCredentials>> {
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "an-invented-slot".into(),
        state_dir: crate::state_paths::init_for_test(),
        api_base: "http://127.0.0.1:9".into(),
    };
    let engine = Arc::new(Engine::open(&REAL, config, host, MemoryCredentials::default()).expect("the handle opens"));
    {
        let _writes = engine.writes.write().unwrap();
        engine
            .acct
            .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
        *engine.acct.session.lock().unwrap() = Some(Session {
            token: "an-invented-token".into(),
            master_key: [7; 32],
            email: None,
        });
    }
    engine
}

#[test]
fn a_person_call_takes_the_token_in_the_same_hold_as_the_locks_answer() {
    let engine = engine_with_a_session();
    let held = engine.writes.write().unwrap();
    let asker = engine.clone();
    let answer = std::thread::spawn(move || asker.person_session(|session, _| session.token.len()))
        .join()
        .unwrap();
    assert_eq!(
        answer,
        Err(PersonRefused::NoSession),
        "a session change under way is never waited for"
    );
    drop(held);
    assert_eq!(
        engine.person_session(|session, _| session.token.len()),
        Ok("an-invented-token".len())
    );
}

/// Lines of one source file that are not comments, with every whitespace character removed.
fn code_of(path: &std::path::Path) -> String {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .flat_map(|line| line.chars())
        .filter(|c| !c.is_whitespace())
        .collect()
}

#[test]
fn every_person_accessor_goes_through_the_choke_point() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = vec![src.join("app.rs")];
    for entry in std::fs::read_dir(src.join("app")).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if name.ends_with(".rs") && !name.ends_with("_tests.rs") {
            files.push(path);
        }
    }
    assert!(files.len() >= 15, "{} source files scanned", files.len());
    let table: [(&str, &[(&str, usize)]); 5] = [
        (
            "acct.session.lock()",
            &[
                // lock_api.rs: the start restore's key loader, moved out of app.rs with its one lock.
                ("app.rs", 3),
                // kept_key_proof.rs: the sign-in's kept-key install (the write moved here from app.rs), inside a
                // `claim` (the session-write lock is held), only after the server confirmed the key.
                ("kept_key_proof.rs", 1),
                ("owned_state.rs", 1),
                ("lock_api.rs", 2),
                // Sign-up has no earlier session: it installs the new one (`signup_api::install_new_account`).
                ("signup_api.rs", 1),
                // Writes inside a `claim` (the session-write lock is held), never a read: a passkey's key kept for
                // the session just made, the new token after a password change, the new address after a resend, and
                // the address of a session that came back without one, named with its owner (session_owner_api.rs,
                // under the session-write lock, only for the token that was asked about).
                ("passkeys_api.rs", 1),
                ("account_password_api.rs", 1),
                ("account_email_resend_api.rs", 1),
                ("session_owner_api.rs", 1),
                // session_watch.rs: the engine's session watch (task 2535) copies the token and notes its answer under
                // the session's lock; no person call, it runs behind a locked screen too.
                ("session_watch.rs", 1),
            ],
        ),
        (
            "while_session_is(",
            &[
                ("owned_state.rs", 1),
                ("guide_mark.rs", 2),
                ("trash_api.rs", 1),
                ("photos_api.rs", 1),
                ("account_api.rs", 2),
                // Generation re-checks of the merged v2-native files (a settings write, a versions list and
                // restore, the viewer's token and key hand-over): each is asked after, or beside, a call that
                // `local_state()` / `account_call_for` already refuses under a lock.
                ("setup_api.rs", 1),
                ("transfer_limits_api.rs", 1),
                ("versions_api.rs", 2),
                ("viewer_api.rs", 2),
                // The same re-checks in the files of the binding run and Wave B/C: each is asked after, or beside, a
                // call that `person_session` / `account_call_for` / `local_state()` already refuses under a lock
                // (account_actions and account_password: the 401 run's note and an in-memory check after the call;
                // conflicts and share_link: a token copy whose `local_state*()` read, before anything is sent,
                // refuses a locked screen). Passkeys and the Trash deletes copy their token through `person_session`
                // itself (review of desktop#424): no admission here.
                ("account_actions_api.rs", 2),
                ("account_password_api.rs", 1),
                ("conflicts_api.rs", 2),
                ("share_link_api.rs", 1),
                // 2530: then local_state_for_changes() (refuses a locked screen) + a session re-check, as conflicts.
                ("parked_exit_api.rs", 1),
            ],
        ),
        ("call_may_run(", &[("lock_api.rs", 2), ("owned_state.rs", 1)]),
        // owned_state.rs: Explorer's work (`local_state_for_work`, `local_state_work_refusal`) and ONE ruled admission
        // (CP2 ruling 2.1, the lead 2026-11-15): owned_state.rs:317, `Ended::OwnSearchesWipe`, the session-end wipe
        // of the ended account's OWN recent searches; it deletes only, its owner proven read-only first. Nothing else
        // is admitted: a fourth goes red.
        ("Call::Work", &[("owned_state.rs", 3)]),
        ("screen_refuses_a_person", &[]),
    ];
    let mut failures = Vec::new();
    for (needle, allowed) in table {
        for file in &files {
            let name = file.file_name().unwrap().to_string_lossy().to_string();
            let count = code_of(file).matches(needle).count();
            let expected = allowed.iter().find(|(n, _)| *n == name).map_or(0, |(_, c)| *c);
            if count != expected {
                failures.push(format!("{needle} in {name}: {count}, expected {expected}"));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// What a person's call is told: no session in memory is `NoSession` whatever the lock says (never "locked"); a session
/// in memory behind a locked screen or a closed vault is that refusal; and the one mapping says it in an API's words.
#[test]
fn no_session_is_never_locked_and_a_session_the_lock_refuses_is() {
    use beebeeb_api::flows::{LockFlow, StartFacts};
    let engine = engine_with_a_session();
    let start = |ask_password_at_start, vault_closed| {
        LockFlow::at_start(StartFacts {
            session_stored: true,
            ask_password_at_start,
            vault_closed,
            app_lock: Some(true),
        })
    };
    *engine.lock_flow.lock().unwrap() = start(Some(false), Some(false));
    assert_eq!(engine.person_session(|_, _| ()), Err(PersonRefused::ScreenLocked));
    // A closed vault that still holds credentials (an install on its way).
    *engine.lock_flow.lock().unwrap() = start(None, None);
    assert_eq!(engine.person_session(|_, _| ()), Err(PersonRefused::VaultClosed));
    // The session leaves memory: no session, whatever the lock says.
    {
        let _writes = engine.writes.write().unwrap();
        *engine.acct.session.lock().unwrap() = None;
    }
    assert_eq!(engine.person_session(|_, _| ()), Err(PersonRefused::NoSession));
    *engine.lock_flow.lock().unwrap() = start(Some(false), Some(false));
    assert_eq!(engine.person_session(|_, _| ()), Err(PersonRefused::NoSession));
    for (refused, said) in [
        (PersonRefused::ScreenLocked, "locked"),
        (PersonRefused::VaultClosed, "locked"),
        (PersonRefused::NoSession, "not signed in"),
    ] {
        assert_eq!(refused.said_as("locked", "not signed in"), said, "{refused:?}");
    }
}
