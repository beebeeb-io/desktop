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
    let source = include_str!("mod.rs");
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

/// W2 (`v2` did not compile on Windows): a password change ends every other session and mints a new token for this
/// device. The engine holds the ended token, so it is stopped, the new token is stored and swapped in, and the engine
/// starts again with it, on every platform. Each platform's start must be one that platform compiles: `v2` called
/// `start_engine_unless_running` (macOS and Linux only) on Windows too. Windows starts through
/// `start_engine_if_possible` under the session-transition lock, the way its unlock and sign-in do (the session gate,
/// its generation and the binding's `OTHER_ACCOUNT_ON_WINDOWS`); macOS and Linux keep the start after a naming.
#[test]
fn the_password_change_restarts_the_engine_on_every_platform() {
    use crate::finder_setup_command_tests::{body_between, production_source, without_items};
    use crate::source_pin::squeeze;
    const WINDOWS: &str = "#[cfg(target_os = \"windows\")]";
    const NOT_WINDOWS: &str = "#[cfg(not(target_os = \"windows\"))]";
    const MACOS: &str = "#[cfg(target_os = \"macos\")]";
    const NOT_MACOS: &str = "#[cfg(not(target_os = \"macos\"))]";
    const LINUX: &str = "#[cfg(target_os = \"linux\")]";
    const WINDOWS_OR_TEST: &str = "#[cfg(any(target_os = \"windows\", test))]";
    const UNIX: &str = "#[cfg(unix)]";
    const NOT_UNIX: &str = "#[cfg(not(unix))]";
    const TEST: &str = "#[cfg(test)]";
    const STARTS: [&str; 3] = [
        "start_engine_unless_running(",
        "start_engine_if_possible(",
        "spawn_bound_engine(",
    ];
    let source = include_str!("mod.rs").replace("\r\n", "\n");
    let production = production_source();
    let after_a_naming = "start_engine_unless_running(app, state.inner()).await";
    for (platform, left_out, expected) in [
        (
            "macOS",
            &[WINDOWS, NOT_MACOS, LINUX, WINDOWS_OR_TEST, NOT_UNIX, TEST][..],
            after_a_naming,
        ),
        (
            "Linux",
            &[WINDOWS, MACOS, WINDOWS_OR_TEST, NOT_UNIX, TEST][..],
            after_a_naming,
        ),
        (
            "Windows",
            &[NOT_WINDOWS, MACOS, LINUX, UNIX, TEST][..],
            "start_engine_if_possible(app, state, &transition).await",
        ),
    ] {
        let compiled = without_items(&source, left_out);
        let rotate = squeeze(body_between(&compiled, "async fn install_rotated_session(", "\n}\n"));
        let called: Vec<&str> = STARTS.into_iter().filter(|start| rotate.contains(start)).collect();
        let count: usize = STARTS.iter().map(|start| rotate.matches(start).count()).sum();
        assert_eq!(
            count, 1,
            "{platform}: one engine start after the swap, found {called:?}:\n{rotate}"
        );
        let callee = called[0].trim_end_matches('(');
        // The start is one this platform builds: the crate root, as this platform compiles it, defines it.
        let platform_crate = without_items(&production, left_out);
        assert!(
            platform_crate.contains(&format!("fn {callee}(")),
            "{platform}: the password change starts the engine with `{callee}`, which {platform} does not compile"
        );
        let started = rotate
            .find(&squeeze(expected))
            .unwrap_or_else(|| panic!("{platform}: the start is `{expected}`:\n{rotate}"));
        let stopped = rotate
            .find(&squeeze("stop_engine_in_slot(acct, engine).await"))
            .unwrap_or_else(|| panic!("{platform}: the engine holding the ended token is stopped:\n{rotate}"));
        let stored = rotate
            .find(&squeeze(
                "persist_session_token_to_keychain(&write, acct.id.as_str(), new_token,",
            ))
            .unwrap_or_else(|| panic!("{platform}: the new token is stored:\n{rotate}"));
        let swapped = rotate
            .find(&squeeze("std::mem::replace(&mut session.token, new_token.to_string())"))
            .unwrap_or_else(|| panic!("{platform}: the new token is swapped into memory:\n{rotate}"));
        assert!(
            stopped < stored && stored < swapped && swapped < started,
            "{platform}: stop, store, swap, then start:\n{rotate}"
        );
        if platform == "Windows" {
            // One session transition from the stop to the start, as `apply_session` holds it for a sign-in: a Lock or
            // a sign-out waits for it, and no other start runs between the swap and this one.
            let transition = rotate
                .find(&squeeze("let transition = SESSION_TRANSITION.lock().await;"))
                .unwrap_or_else(|| panic!("Windows: the swap is one session transition:\n{rotate}"));
            let slot = rotate
                .find(&squeeze("acct.engine.lock().await"))
                .expect("the engine slot");
            assert!(
                transition < slot,
                "Windows: the transition is taken before the engine stops:\n{rotate}"
            );
        }
    }
    // The command reaches it.
    let command = squeeze(body_between(&source, "async fn account_change_password(", "\n}\n"));
    assert!(
        command.contains(&squeeze(
            "install_rotated_session(app, &state, &acct, &mut turn, &session_token).await"
        )),
        "{command}"
    );
}
