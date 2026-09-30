use super::*;

#[test]
fn slice1_g1_schema_identity() {
    let dir = tempfile::tempdir().unwrap();
    let db = schema_open(&dir.path().join("state-v2.db")).unwrap();
    let count: i64 = db.query_row("SELECT count(*) FROM sqlite_schema WHERE type='table' AND name LIKE 'v2_%'", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 11, "P1 must create exactly eleven isolated account tables");
}
#[test]
fn slice1_g2_barrier_bootstrap() {
    assert!(!storage_proof(false, true, true, true), "NORMAL cannot authorize the fake destructive sink");
    assert!(!storage_proof(true, false, true, true));
    assert!(!storage_proof(true, true, false, true));
    assert!(!storage_proof(true, true, true, false));
    assert!(storage_proof(true, true, true, true));
}
#[test]
fn slice1_g3_recovery_admission() {
    let admission = Admission::default();
    for denied in [false, true] {
        assert!(!admission.recovered_can_submit(denied), "restored records cannot grant submission even with valid L0");
    }
}
#[test]
fn slice1_g4_capacity_reservations() {
    let mut budget = Budget::default();
    assert!(budget.reserve(12 * MIB, 20 * MIB));
    assert!(!budget.reserve(12 * MIB, 20 * MIB), "concurrent reservations must share quota");
    assert_eq!(budget.reserved, 12 * MIB);
}
#[test]
fn slice1_g5_pinned_reader_backpressure() {
    assert!(payload_admitted(64 * MIB));
    assert!(!payload_admitted(128 * MIB), "payload admission stops at 128 MiB");
    assert!(!payload_admitted(256 * MIB));
    assert!(!payload_admitted(320 * MIB));
}
#[test]
fn slice1_g6_privacy_ciphertext() {
    let canary = b"unique-A-account-name-digest-canary-1640";
    let bytes = protect_canary(canary).unwrap();
    assert_eq!(bytes.windows(canary.len()).filter(|v| *v == canary).count(), 0, "ledger must contain zero plaintext canaries");
}
#[test]
fn slice1_g7_bounded_processing() {
    assert_eq!(batch_limit(4096), 256, "metadata batches are bounded independently of input length");
    assert_eq!(batch_limit(1), 1);
}
#[test]
fn slice1_g8_dormancy_source_gate() {
    let source = include_str!("../lib.rs").replace("\r\n", "\n");
    assert!(source.contains("#[cfg(test)]\nmod content_v2;"), "V2 must be excluded from shipping builds");
    assert_eq!(source.matches("content_v2").count(), 1, "zero production invocations");
}
