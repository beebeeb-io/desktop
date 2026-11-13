//! The extended no-secret DTO test (crypto review 2026-11-08, rule 10).

use super::*;
use serde_json::Value;

const FORBIDDEN: &[&str] = &[
    "password",
    "phrase",
    "mnemonic",
    "secret",
    "prf",
    "master",
    "backup_codes",
    "otpauth",
    "fragment",
    "token",
];

/// (DTO, field, reason): the only secrets a B14 command returns, each shown once by design.
const ALLOW: &[(&str, &str, &str)] = &[
    (
        "TotpSetupDto",
        "secret",
        "DW-A69 shows the authenticator key once, in mono, for an authenticator on Windows",
    ),
    ("TotpSetupDto", "otpauth_uri", "DW-A69 draws it as the QR code once"),
    // `qr_rows` (the same URI as QR modules) matches no forbidden name; it is listed here for the record.
    (
        "TotpSetupDto",
        "qr_rows",
        "DW-A69 the QR code of otpauth_uri, shown once",
    ),
    (
        "BackupCodesDto",
        "backup_codes",
        "DW-A70 shows the 8 single-use codes once",
    ),
];

fn keys(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(m) => {
            for (k, v) in m {
                out.push(k.clone());
                keys(v, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|v| keys(v, out)),
        _ => {}
    }
}

fn offending(name: &str, dto: &impl Serialize) -> Vec<String> {
    let mut all = vec![];
    keys(&serde_json::to_value(dto).unwrap(), &mut all);
    all.into_iter()
        .filter(|k| FORBIDDEN.iter().any(|f| k.to_lowercase().contains(f)))
        .filter(|k| !ALLOW.iter().any(|(d, f, _)| *d == name && f == k))
        .collect()
}

#[test]
fn b14_dtos_carry_no_secret_outside_the_allow_list() {
    assert!(
        offending(
            "TotpSetupDto",
            &TotpSetupDto {
                secret: "S".into(),
                otpauth_uri: "o".into(),
                qr_rows: vec![]
            }
        )
        .is_empty()
    );
    assert!(
        offending(
            "BackupCodesDto",
            &BackupCodesDto {
                backup_codes: vec!["1".into()]
            }
        )
        .is_empty()
    );
    assert!(offending("EmailChangedDto", &EmailChangedDto { email: "a@b.eu".into() }).is_empty());
    assert!(
        offending(
            "DisplayNameDto",
            &DisplayNameDto {
                display_name: Some("Anna".into())
            }
        )
        .is_empty()
    );
    assert!(offending("ActivityLoggingDto", &ActivityLoggingDto { on: true }).is_empty());
    assert!(
        offending(
            "DeleteRequestedDto",
            &DeleteRequestedDto {
                email_link_sent: true,
                link_expires_at: "t".into()
            }
        )
        .is_empty()
    );
    assert!(
        offending(
            "DeletionScheduledDto",
            &DeletionScheduledDto {
                shred_after: "t".into(),
                cancel_until: Some("c".into())
            }
        )
        .is_empty()
    );
    assert!(
        offending(
            "DeletionCancelledDto",
            &DeletionCancelledDto {
                not_restored: vec!["sessions".into()]
            }
        )
        .is_empty()
    );
    assert!(
        offending(
            "PasswordChangedDto",
            &PasswordChangedDto {
                status: "changed",
                signed_in: true,
                restart: "ok"
            }
        )
        .is_empty()
    );
}

/// The check goes red on a planted field, and the allow-list is per DTO.
#[test]
fn the_no_secret_check_fails_on_a_planted_field() {
    #[derive(Serialize)]
    struct Planted {
        email: String,
        session_token: String,
    }
    #[derive(Serialize)]
    struct PlantedDeep {
        inner: Vec<Planted>,
    }
    let bad = offending(
        "EmailChangedDto",
        &Planted {
            email: "a".into(),
            session_token: "t".into(),
        },
    );
    assert_eq!(bad, ["session_token"]);
    let deep = offending(
        "EmailChangedDto",
        &PlantedDeep {
            inner: vec![Planted {
                email: "a".into(),
                session_token: "t".into(),
            }],
        },
    );
    assert_eq!(deep, ["session_token"], "nested fields are checked too");
    // The TOTP allow-list entry does not cover another DTO.
    assert_eq!(
        offending(
            "DeletionScheduledDto",
            &TotpSetupDto {
                secret: "S".into(),
                otpauth_uri: "o".into(),
                qr_rows: vec![]
            }
        )
        .len(),
        2
    );
}

/// The delete-link routes are public: they run without a session, so Undo works after the link revoked it.
#[test]
fn the_delete_link_commands_need_no_session() {
    let source = include_str!("mod.rs").replace("\r\n", "\n");
    for signature in ["fn account_delete_confirm_link(", "fn account_delete_undo("] {
        let start = source.find(signature).expect(signature);
        let body = &source[start..start + source[start..].find("\n}\n").unwrap()];
        assert!(body.contains("drive(") && body.contains(", None)"), "{signature}");
        assert!(
            !body.contains("session_parts") && !body.contains("run_simple"),
            "{signature} needs a session"
        );
    }
}

/// Core v0.2.8 item 6: the password change runs sign-up's gate with the server's policy and the
/// breach check this app made for the new password. The check is always answered (an endpoint
/// that cannot be reached is recorded as an outage), so the policy's own fail-open rule decides.
#[test]
fn the_new_password_passes_sign_ups_gate_with_the_check_this_app_made() {
    use beebeeb_core::signup_flow::SignupPolicy;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let binding = || AccountBinding::from_master_key(&MasterKey::from_bytes([1; 32]));
    let (current, new) = ("the old password 2025", "a new long password 2026");

    // No breach endpoint declared: no check is made, and the minimum still applies.
    let plain = SignupPolicy::default();
    assert!(rt.block_on(breach_check(&plain, new)).is_none());
    assert!(AccountFlow::change_password(current, new, binding(), &plain, None).is_ok());
    let short = AccountFlow::change_password(current, "short", binding(), &plain, None);
    assert_eq!(short.err().map(code).as_deref(), Some("weak_password"));

    // A declared endpoint this unit-test build cannot reach (its API base is a closed port).
    let closed = SignupPolicy {
        breach_endpoint: Some("/api/v1/onboarding/breach-range/{prefix}".into()),
        breach_fail_open: false,
        ..SignupPolicy::default()
    };
    let check = rt
        .block_on(breach_check(&closed, new))
        .expect("a declared check is made");
    assert!(check.is_answered(), "an unreachable endpoint is recorded as an outage");
    let refused = AccountFlow::change_password(current, new, binding(), &closed, Some(&check));
    assert_eq!(refused.err().map(code).as_deref(), Some("breach_check_blocked"));
    let missing = AccountFlow::change_password(current, new, binding(), &closed, None);
    assert_eq!(missing.err().map(code).as_deref(), Some("breach_check_missing"));
    let other = AccountFlow::change_password(current, "another long password 1", binding(), &closed, Some(&check));
    assert_eq!(
        other.err().map(code).as_deref(),
        Some("breach_check_missing"),
        "a check made for another password does not count"
    );

    // The server allows an outage: the same answered check passes the gate.
    let open = SignupPolicy {
        breach_fail_open: true,
        ..closed.clone()
    };
    assert!(AccountFlow::change_password(current, new, binding(), &open, Some(&check)).is_ok());
}

/// W2 (`v2` did not compile on Windows): a password change ends every other session and mints a new token for this
/// device. The engine holds the ended token, so the new token is stored and swapped in and the engine starts again with
/// it, on every platform. Each platform's start must be one that platform compiles: `v2` called
/// `start_engine_unless_running` (macOS and Linux only) on Windows too. macOS and Linux stop the engine first and keep
/// the start after a naming. Windows stores the token before its first await (a Lock or a sign-out cancels a Windows
/// session command at an await, after the server has already ended the old token), then starts through
/// `start_engine_if_possible` under the session-transition lock, the way its unlock and sign-in do.
#[test]
fn the_password_change_restarts_the_engine_on_every_platform() {
    use crate::finder_setup_command_tests::{body_between, macos_compiled, production_source, without_items};
    use crate::source_pin::squeeze;
    const TEST: [&str; 2] = ["#[cfg(test)]", "#[cfg(all(test"];
    // What `macos_compiled` (the scanners' one macOS view) leaves out, plus the spellings it does not name.
    const MACOS_ALSO_LEAVES_OUT: [&str; 3] = [
        "#[cfg(windows)]",
        "#[cfg(not(target_os = \"macos\"))]",
        "#[cfg(any(target_os = \"windows\", target_os = \"linux\"))]",
    ];
    const LINUX_LEAVES_OUT: [&str; 7] = [
        "#[cfg(target_os = \"windows\")]",
        "#[cfg(windows)]",
        "#[cfg(any(target_os = \"windows\", test))]",
        "#[cfg(target_os = \"macos\")]",
        "#[cfg(not(target_os = \"linux\"))]",
        "#[cfg(not(unix))]",
        "#[cfg(any(target_os = \"macos\", target_os = \"windows\"))]",
    ];
    const WINDOWS_LEAVES_OUT: [&str; 7] = [
        "#[cfg(not(target_os = \"windows\"))]",
        "#[cfg(not(windows))]",
        "#[cfg(target_os = \"macos\")]",
        "#[cfg(target_os = \"linux\")]",
        "#[cfg(unix)]",
        "#[cfg(all(unix",
        "#[cfg(any(target_os = \"macos\", target_os = \"linux\"))]",
    ];
    const STARTS: [&str; 3] = [
        "start_engine_unless_running(",
        "start_engine_if_possible(",
        "spawn_bound_engine(",
    ];
    let compile_for = |platform: &str, source: &str| -> String {
        let source = without_items(source, &TEST);
        match platform {
            "macOS" => macos_compiled(&without_items(&source, &MACOS_ALSO_LEAVES_OUT)),
            "Linux" => without_items(&source, &LINUX_LEAVES_OUT),
            _ => without_items(&source, &WINDOWS_LEAVES_OUT),
        }
    };
    let source = include_str!("mod.rs").replace("\r\n", "\n");
    let production = production_source();
    let after_a_naming = "start_engine_unless_running(app, state.inner()).await";
    for (platform, expected) in [
        ("macOS", after_a_naming),
        ("Linux", after_a_naming),
        ("Windows", "start_engine_if_possible(app, state, &transition).await"),
    ] {
        let compiled = compile_for(platform, &source);
        let rotate = squeeze(body_between(&compiled, "async fn install_rotated_session(", "\n}\n"));
        let called: Vec<&str> = STARTS.into_iter().filter(|start| rotate.contains(start)).collect();
        let count: usize = STARTS.iter().map(|start| rotate.matches(start).count()).sum();
        assert_eq!(
            count, 1,
            "{platform}: one engine start after the swap, found {called:?}:\n{rotate}"
        );
        let callee = called[0].trim_end_matches('(');
        // The start is one this platform builds: the crate root, as this platform compiles it, defines it.
        assert!(
            compile_for(platform, &production).contains(&format!("fn {callee}(")),
            "{platform}: the password change starts the engine with `{callee}`, which {platform} does not compile"
        );
        let started = rotate
            .find(&squeeze(expected))
            .unwrap_or_else(|| panic!("{platform}: the start is `{expected}`:\n{rotate}"));
        // The store and the swap live in `store_rotated_session_with`; `install_rotated_session` claims the turn by
        // calling `store_rotated_session` before the engine comes back.
        let store = squeeze(body_between(&compiled, "fn store_rotated_session_with", "\n}\n"));
        let stored = rotate
            .find(&squeeze("store_rotated_session(acct, turn, new_token)"))
            .unwrap_or_else(|| panic!("{platform}: the new token is stored:\n{rotate}"));
        let persisted = store
            .find(&squeeze("store.store_token(&write, acct.id.as_str(), new_token,"))
            .unwrap_or_else(|| panic!("{platform}: the new token is stored in the credential store:\n{store}"));
        let swapped = store
            .find(&squeeze("std::mem::replace(&mut session.token, new_token.to_string())"))
            .unwrap_or_else(|| panic!("{platform}: the new token is swapped into memory:\n{store}"));
        assert!(
            persisted < swapped,
            "{platform}: store, then swap into memory:\n{store}"
        );
        assert!(stored < started, "{platform}: store, then start:\n{rotate}");
        let stop = squeeze("stop_engine_in_slot(acct, engine).await");
        if platform == "Windows" {
            // Nothing awaits between the server's answer and the store: no await at all before it in this function.
            assert!(
                !rotate[..stored].contains(".await"),
                "Windows: an await before the new token is stored can cancel the command and lose it:\n{rotate}"
            );
            // The old engine is stopped under the session-transition lock before the start (task 2439 item 1):
            // the start's config read can fail before it takes the slot. The running ownership proofs, which
            // hold the ended token too, are stopped under the same lock first.
            let stopped = rotate
                .find(&stop)
                .unwrap_or_else(|| panic!("Windows: the old engine is stopped under the lock:\n{rotate}"));
            let transition = rotate
                .find(&squeeze("let transition = SESSION_TRANSITION.lock().await;"))
                .unwrap_or_else(|| panic!("Windows: the start holds the session-transition lock:\n{rotate}"));
            let proofs = rotate
                .find("stop_own_data_proofs();")
                .unwrap_or_else(|| panic!("Windows: the ownership proofs are stopped:\n{rotate}"));
            assert!(
                stored < transition && transition < proofs && proofs < stopped && stopped < started,
                "Windows: store, lock, stop the proofs, stop the old engine, then start:\n{rotate}"
            );
        } else {
            let stopped = rotate
                .find(&stop)
                .unwrap_or_else(|| panic!("{platform}: the engine holding the ended token is stopped:\n{rotate}"));
            assert!(
                stopped < stored,
                "{platform}: the engine stops before the store:\n{rotate}"
            );
        }
    }
    // The command hands the turn to the detached commit; the task reaches `install_rotated_session`.
    let command = squeeze(body_between(&source, "async fn account_change_password(", "\n}\n"));
    assert!(
        command.contains(&squeeze("drive(flow, Some(&token), Some(commit)).await")),
        "{command}"
    );
    let commit = squeeze(body_between(&source, "async fn commit_rotated_session(", "\n}\n"));
    assert!(
        commit.contains(&squeeze(
            "install_rotated_session(app.clone(), &state, &acct, &mut turn, session_token.as_str()).await"
        )),
        "{commit}"
    );
}

/// Task 2439 item 1: on Windows the engine holding the ended token is stopped under the session lock
/// before the start. `start_engine_if_possible` begins with `if let Ok(cfg) = DesktopConfig::load()`,
/// so a config read failure returns `Ok` before it takes the engine slot: without this stop the old
/// engine keeps the ended token and the app shows "signed out" after three 401s.
#[test]
fn the_password_change_stops_the_old_engine_before_a_config_failure() {
    use crate::finder_setup_command_tests::{body_between, without_items};
    use crate::source_pin::squeeze;
    const TEST: [&str; 2] = ["#[cfg(test)]", "#[cfg(all(test"];
    const WINDOWS_LEAVES_OUT: [&str; 7] = [
        "#[cfg(not(target_os = \"windows\"))]",
        "#[cfg(not(windows))]",
        "#[cfg(target_os = \"macos\")]",
        "#[cfg(target_os = \"linux\")]",
        "#[cfg(unix)]",
        "#[cfg(all(unix",
        "#[cfg(any(target_os = \"macos\", target_os = \"linux\"))]",
    ];
    let source = include_str!("mod.rs").replace("\r\n", "\n");
    let windows = without_items(&without_items(&source, &TEST), &WINDOWS_LEAVES_OUT);
    let rotate = squeeze(body_between(&windows, "async fn install_rotated_session(", "\n}\n"));
    let stop = rotate
        .find(&squeeze("stop_engine_in_slot(acct, engine).await"))
        .unwrap_or_else(|| panic!("Windows: the old engine is stopped under the session lock:\n{rotate}"));
    let start = rotate
        .find("start_engine_if_possible(")
        .unwrap_or_else(|| panic!("Windows: the engine starts again:\n{rotate}"));
    assert!(
        stop < start,
        "Windows: the old engine stops before the start, so a config read failure cannot leave it on the ended token:\n{rotate}"
    );
}

// ── Task 2439 item 5: the rotated token survives a cancelled command ──────────

use crate::account::{AccountId, synthesize_single_account};

/// A signed-in account with `token` in memory, for the commit tests.
fn account_with_session(token: &str) -> (AppState, Arc<account::AccountRuntime>, String) {
    let state = AppState::default();
    synthesize_single_account(&state, AccountId::new_v4());
    let acct = state.active_account().unwrap();
    let id = acct.id.as_str().to_string();
    *acct.session.lock().unwrap() = Some(Session {
        token: token.to_string(),
        master_key: [9u8; 32],
        email: Some("a@b.eu".to_string()),
    });
    (state, acct, id)
}

/// Records what a rotated-session commit asked the credential store to keep; no OS vault is touched (Linux has none).
#[derive(Default)]
struct RecordingStore {
    stored: std::sync::Mutex<Vec<String>>,
    owners: std::sync::Mutex<Vec<String>>,
}

impl RecordingStore {
    fn stored(&self) -> Vec<String> {
        self.stored.lock().unwrap().clone()
    }
}

impl RotatedSessionStore for RecordingStore {
    fn store_token(
        &self,
        _write: &SessionWrite,
        _account_id: &str,
        token: &str,
        _email: Option<&str>,
    ) -> Result<(), String> {
        self.stored.lock().unwrap().push(token.to_string());
        Ok(())
    }

    fn carry_owner(&self, _write: &SessionWrite, _account_id: &str, owner: &str) {
        self.owners.lock().unwrap().push(owner.to_string());
    }
}

fn memory_token(acct: &account::AccountRuntime) -> Option<String> {
    acct.session.lock().unwrap().as_ref().map(|s| s.token.clone())
}

/// The server has rotated the token, then a Lock or a sign-out drops the command's future while the response body is
/// still in flight. The body read and the store run in a task of their own (`detached`), so they finish: the new token
/// is in the credential store and in memory, and the turn is spent exactly once (the captured turn can never write
/// again).
#[tokio::test]
async fn session_commit_survives_a_dropped_caller() {
    let (_state, acct, _id) = account_with_session("old-token");
    let turn = acct.session_generation();
    let store = Arc::new(RecordingStore::default());

    let (body_tx, body_rx) = tokio::sync::oneshot::channel::<String>();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel::<()>();
    let (done_tx, done_rx) = tokio::sync::oneshot::channel::<Result<(), RotatedStoreError>>();
    let task_acct = acct.clone();
    let task_store = store.clone();
    // The shape `drive` uses for a commit: the body read and the store in a task of its own, awaited by handle.
    let commit = detached(async move {
        started_tx.send(()).unwrap();
        let body = body_rx.await.unwrap_or_default();
        let mut turn = turn;
        let result = store_rotated_session_with(&task_acct, &mut turn, &body, task_store.as_ref());
        let _ = done_tx.send(result);
        Ok(())
    });
    let caller = tokio::spawn(commit);
    started_rx.await.expect("the commit task started");
    // The command is cancelled: its future is dropped here (dropping a bare handle would only detach it).
    caller.abort();
    let _ = caller.await;
    // Only now does the server's body arrive.
    body_tx.send("new-token".to_string()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), done_rx)
        .await
        .expect("the dropped caller took the commit with it")
        .expect("the commit task answered")
        .expect("the new token was stored");

    assert_eq!(store.stored(), ["new-token"], "the credential store has the new token");
    assert_eq!(
        memory_token(&acct).as_deref(),
        Some("new-token"),
        "the session in memory has the new token"
    );
    // Spent exactly once: the same captured turn writes nothing a second time, and the store was written once.
    let mut replay = turn;
    assert_eq!(
        store_rotated_session_with(&acct, &mut replay, "third-token", store.as_ref()),
        Err(RotatedStoreError::TurnSpent)
    );
    assert_eq!(store.stored(), ["new-token"]);
    assert_eq!(memory_token(&acct).as_deref(), Some("new-token"));
}

/// A Lock (or another sign-in) moves the account's generation before the body arrives. The detached commit must write
/// nothing, must not panic, and its answer must say the turn was spent; the next command sees a signed-out session, not
/// a silent sign-in.
#[tokio::test]
async fn session_commit_refuses_a_stale_turn() {
    let (_state, acct, _id) = account_with_session("old-token");
    let turn = acct.session_generation();
    let store = Arc::new(RecordingStore::default());

    let (body_tx, body_rx) = tokio::sync::oneshot::channel::<String>();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel::<()>();
    let task_acct = acct.clone();
    let task_store = store.clone();
    let commit = detached(async move {
        started_tx.send(()).unwrap();
        let body = body_rx.await.unwrap_or_default();
        let mut turn = turn;
        Ok(store_rotated_session_with(
            &task_acct,
            &mut turn,
            &body,
            task_store.as_ref(),
        ))
    });
    let task = tokio::spawn(commit);
    started_rx.await.expect("the commit task started");
    // The generation moves before the body arrives: a Lock, or another sign-in, happened.
    {
        let _lock = end_sessions_in_flight(&acct, account::SessionTransition::Lock);
    }
    let after_the_move = acct.session_generation();
    body_tx.send("new-token".to_string()).unwrap();
    let answer = task.await.unwrap().unwrap();
    assert_eq!(
        answer,
        Err(RotatedStoreError::TurnSpent),
        "the answer says the turn was spent"
    );
    assert_eq!(
        acct.session_generation(),
        after_the_move,
        "the stale commit moved nothing"
    );
    assert!(store.stored().is_empty(), "the credential store was not written");
    assert_eq!(
        memory_token(&acct).as_deref(),
        Some("old-token"),
        "the session in memory was not written"
    );
}

/// Source guard: the command's future (`drive`/`drive_steps`) reads no response body itself; the one read is the
/// helper, and for a password change's last answer it runs inside the detached task. The store claims the captured
/// turn before it writes anything.
#[test]
fn session_commit_is_the_only_body_reader() {
    use crate::finder_setup_command_tests::body_between;
    use crate::source_pin::squeeze;
    let source = include_str!("mod.rs").replace("\r\n", "\n");
    let drive = squeeze(body_between(&source, "async fn drive(", "\n}\n"));
    let steps = squeeze(body_between(&source, "async fn drive_steps(", "\n}\n"));
    let helper = squeeze(body_between(&source, "async fn read_body(", "\n}\n"));
    assert!(
        !drive.contains(".text().await"),
        "the command future reads no body: {drive}"
    );
    assert!(
        !drive.contains("detached("),
        "the command's future does not move the flow into a detached task: {drive}"
    );
    assert!(
        !steps.contains(".text().await"),
        "the command future reads no body: {steps}"
    );
    assert!(
        helper.contains("resp.text().await"),
        "the one body read lives in the helper: {helper}"
    );
    assert!(
        steps.contains("read_body(resp)"),
        "every step reads through the helper: {steps}"
    );
    assert!(
        steps.contains("detached(") && steps.contains("commit_rotated_session("),
        "the final commit's body read and store run in the detached task: {steps}"
    );
    assert!(
        steps.contains("drop(flow)"),
        "the flow is dropped in the command's future, not moved into the detached task: {steps}"
    );
    let send = steps
        .find("rb.send().await")
        .unwrap_or_else(|| panic!("the request is sent in the command's future: {steps}"));
    let detach = steps
        .find("detached(")
        .unwrap_or_else(|| panic!("the answer is handed to the detached task: {steps}"));
    assert!(
        send < detach,
        "the request is sent before the answer is detached: {steps}"
    );
    let commit = squeeze(body_between(&source, "async fn commit_rotated_session(", "\n}\n"));
    assert!(!commit.contains("flow"), "the detached task carries no flow: {commit}");
    assert!(
        commit.contains("install_rotated_session("),
        "the detached task stores the rotated token: {commit}"
    );
    let store = squeeze(body_between(&source, "fn store_rotated_session_with", "\n}\n"));
    let claim = store
        .find("claim_session_write_for_commit(")
        .unwrap_or_else(|| panic!("the store claims the turn first: {store}"));
    let persist = store
        .find("store_token(")
        .unwrap_or_else(|| panic!("the store writes the token: {store}"));
    assert!(
        claim < persist,
        "the turn is claimed before anything is stored: {store}"
    );
}
