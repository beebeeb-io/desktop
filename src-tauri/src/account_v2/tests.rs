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
        let stored = rotate
            .find(&squeeze(
                "persist_session_token_to_keychain(&write, acct.id.as_str(), new_token,",
            ))
            .unwrap_or_else(|| panic!("{platform}: the new token is stored:\n{rotate}"));
        let swapped = rotate
            .find(&squeeze("std::mem::replace(&mut session.token, new_token.to_string())"))
            .unwrap_or_else(|| panic!("{platform}: the new token is swapped into memory:\n{rotate}"));
        assert!(
            stored < swapped && swapped < started,
            "{platform}: store, swap, then start:\n{rotate}"
        );
        let stop = squeeze("stop_engine_in_slot(acct, engine).await");
        if platform == "Windows" {
            // Nothing awaits between the server's answer and the store: no await at all before it in this function.
            assert!(
                !rotate[..stored].contains(".await"),
                "Windows: an await before the new token is stored can cancel the command and lose it:\n{rotate}"
            );
            // The start stops the old engine itself (and refuses when that stop is unconfirmed); the running
            // ownership proofs, which hold the ended token too, are stopped under the same lock first.
            assert!(
                !rotate.contains(&stop),
                "Windows: the start stops the engine:\n{rotate}"
            );
            let transition = rotate
                .find(&squeeze("let transition = SESSION_TRANSITION.lock().await;"))
                .unwrap_or_else(|| panic!("Windows: the start holds the session-transition lock:\n{rotate}"));
            let proofs = rotate
                .find("stop_own_data_proofs();")
                .unwrap_or_else(|| panic!("Windows: the ownership proofs are stopped:\n{rotate}"));
            assert!(
                swapped < transition && transition < proofs && proofs < started,
                "Windows: swap, lock, stop the proofs, then start:\n{rotate}"
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
    // The command reaches it.
    let command = squeeze(body_between(&source, "async fn account_change_password(", "\n}\n"));
    assert!(
        command.contains(&squeeze(
            "install_rotated_session(app, &state, &acct, &mut turn, &session_token).await"
        )),
        "{command}"
    );
}
