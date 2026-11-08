//! The extended no-secret DTO test (crypto review 2026-11-08, rule 10).

use super::*;
use serde_json::Value;

const FORBIDDEN: &[&str] = &[
    "password", "phrase", "mnemonic", "secret", "prf", "master", "backup_codes", "otpauth", "fragment", "token",
];

/// (DTO, field, reason): the only secrets a B14 command returns, each shown once by design.
const ALLOW: &[(&str, &str, &str)] = &[
    ("TotpSetupDto", "secret", "DW-A69 shows the authenticator key once, in mono, for an authenticator on Windows"),
    ("TotpSetupDto", "otpauth_uri", "DW-A69 draws it as the QR code once"),
    // `qr_rows` (the same URI as QR modules) matches no forbidden name; it is listed here for the record.
    ("TotpSetupDto", "qr_rows", "DW-A69 the QR code of otpauth_uri, shown once"),
    ("BackupCodesDto", "backup_codes", "DW-A70 shows the 8 single-use codes once"),
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
    assert!(offending("TotpSetupDto", &TotpSetupDto { secret: "S".into(), otpauth_uri: "o".into(), qr_rows: vec![] }).is_empty());
    assert!(offending("BackupCodesDto", &BackupCodesDto { backup_codes: vec!["1".into()] }).is_empty());
    assert!(offending("EmailChangedDto", &EmailChangedDto { email: "a@b.eu".into() }).is_empty());
    assert!(offending("DisplayNameDto", &DisplayNameDto { display_name: Some("Anna".into()) }).is_empty());
    assert!(offending("ActivityLoggingDto", &ActivityLoggingDto { on: true }).is_empty());
    assert!(offending("DeletionDto", &DeletionDto { shred_after: "t".into(), email_link_sent: false }).is_empty());
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
    let bad = offending("EmailChangedDto", &Planted { email: "a".into(), session_token: "t".into() });
    assert_eq!(bad, ["session_token"]);
    let deep = offending("EmailChangedDto", &PlantedDeep { inner: vec![Planted { email: "a".into(), session_token: "t".into() }] });
    assert_eq!(deep, ["session_token"], "nested fields are checked too");
    // The TOTP allow-list entry does not cover another DTO.
    assert_eq!(offending("DeletionDto", &TotpSetupDto { secret: "S".into(), otpauth_uri: "o".into(), qr_rows: vec![] }).len(), 2);
}
