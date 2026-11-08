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
