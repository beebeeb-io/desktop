//! `suppression` tests: the engine-delete suppression set (task 0806).

use super::*;

#[test]
fn engine_delete_suppression_is_consumed_once() {
    // task 0806: an engine-originated delete is registered, then the FIRST
    // matching Delete event consumes the entry (returns true → drop it). A
    // SECOND event for the same path is a genuine user delete (false →
    // propagate). Use unique paths so the process-global static can't collide
    // with another test.
    let p = std::path::Path::new("/sync/engine-deleted-0806-unique-a.txt");
    assert!(!take_engine_delete_suppressed(p), "unregistered path is not suppressed");

    suppress_engine_delete(p);
    assert!(
        take_engine_delete_suppressed(p),
        "first event after register is suppressed"
    );
    assert!(
        !take_engine_delete_suppressed(p),
        "suppression is consumed once — a later user delete of the same path propagates"
    );
}

#[test]
fn engine_delete_suppression_distinguishes_paths() {
    // Registering one path must not suppress a delete of a DIFFERENT path.
    let registered = std::path::Path::new("/sync/engine-deleted-0806-unique-b.txt");
    let user = std::path::Path::new("/sync/user-deleted-0806-unique-c.txt");
    suppress_engine_delete(registered);
    assert!(
        !take_engine_delete_suppressed(user),
        "a genuine user delete of an unregistered path must NOT be suppressed"
    );
    // The registered entry is still there (untouched by the miss above).
    assert!(take_engine_delete_suppressed(registered));
}

#[test]
fn engine_delete_suppression_prunes_stale_entries() {
    // A registered delete whose NOTIFY never arrives must eventually be swept
    // so it can't shadow a later user delete. Force-insert with an OLD instant
    // and confirm the prune evicts it.
    let p = std::path::Path::new("/sync/engine-deleted-0806-unique-d.txt");
    {
        let mut set = engine_delete_suppress().lock().unwrap();
        set.insert(
            p.to_path_buf(),
            Instant::now() - ENGINE_SUPPRESS_TTL - Duration::from_secs(1),
        );
    }
    prune_stale_engine_suppressions();
    assert!(
        !take_engine_delete_suppressed(p),
        "a stale suppression entry must be pruned so a later user delete propagates"
    );
}

#[test]
fn engine_remove_of_one_case_twin_is_not_a_user_delete_of_the_other_spelling() {
    // Report.pdf / report.pdf are two server rows but ONE entry on NTFS/APFS. The engine removes the entry by
    // the pruned row's path (`report.pdf`); the volume's delete notice names the entry as spelled on disk
    // (`Report.pdf`). That notice must consume the registration; a miss would reach `handle_delete`, find the
    // OTHER twin's row at `Report.pdf` and queue a server trash of it.
    use crate::drive_v2::case_place::{CaseRule, with_rule_for_test};
    let removed = std::path::Path::new("/sync/w2-case-r3-a/report.pdf");
    let notice = std::path::Path::new("/sync/w2-case-r3-a/Report.pdf");
    with_rule_for_test(CaseRule::Insensitive, || {
        suppress_engine_delete(removed);
        assert!(
            take_engine_delete_suppressed(notice),
            "the notice under the other spelling is the engine's own"
        );
        assert!(!take_engine_delete_suppressed(removed), "consumed once");
    });
    // A case-sensitive volume holds two entries: a delete of the other spelling is the person's.
    let removed = std::path::Path::new("/sync/w2-case-r3-b/report.pdf");
    let other = std::path::Path::new("/sync/w2-case-r3-b/Report.pdf");
    with_rule_for_test(CaseRule::Sensitive, || {
        suppress_engine_delete(removed);
        assert!(!take_engine_delete_suppressed(other));
        assert!(take_engine_delete_suppressed(removed));
    });
}
