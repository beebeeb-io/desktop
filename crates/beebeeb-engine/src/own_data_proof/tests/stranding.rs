//! The stranding paths, split out of `own_data_proof.rs`.
//!
//! A Windows PC refused every sync start while its local data was unowned, or owned by a
//! session whose stored email no longer matched. These tests reproduce that against the real
//! state.db: the start is refused, then the one-shot adoption proves the data is this
//! account's own and the start goes through. The second group holds R10: another account's
//! local data is never adopted, and nothing is deleted.

use super::*;

// ---- the stranding paths, reproduced against the real state.db ----

/// Path 1 (the report): a session stored before its email was kept names no adoption candidate at the first
/// startup, so the one-shot window closes with no owner, and every later start refuses with the
/// "another account" sentence although the data is this account's own. The proof adopts it; the held write is
/// kept and the next start proceeds.
#[test]
fn a_closed_window_with_no_candidate_no_longer_strands_the_owner() {
    let pc = pc_with(&IDS);
    assert_eq!(adopt_unbound(&pc.db, None), Ok(false), "the startup had no candidate");
    assert_eq!(pc.db.0.owner().unwrap(), None);
    assert_eq!(
        bind_before_engine_start(&pc.db, &session(), WINDOWS),
        Err(OTHER_ACCOUNT_ON_WINDOWS.to_string()),
        "stranded before the proof"
    );

    let server = own_server();
    let outcome = prove(&pc, &session(), MINE, &server);

    assert!(
        matches!(outcome, Outcome::Adopted(t) if t.own == 3 && t.checked == 3 && t.proven_changes == 1),
        "{outcome:?}"
    );
    assert_eq!(server.listed.get(), 1, "one listing, no question per file");
    assert_eq!(pc.db.0.owner().unwrap(), Some(session()));
    assert_eq!(bind_before_engine_start(&pc.db, &session(), WINDOWS), Ok(Bound::Kept));
    assert_eq!(
        due(&pc),
        vec!["held-1".to_string()],
        "the held write is kept, to upload"
    );
}

/// Path 2: the startup had a candidate but the window was already shut (an engine start or a sign-in opened the
/// database first: `StateDbLocalData::for_engine_start` closes it). The late candidate is refused, as designed;
/// the proof still adopts.
#[test]
fn a_window_shut_before_the_candidate_arrived_no_longer_strands_the_owner() {
    let pc = pc_with(&IDS);
    pc.db.0.close_adoption_window().unwrap();
    assert_eq!(
        adopt_unbound(&pc.db, Some(&session())),
        Ok(false),
        "the window was already shut"
    );
    assert_eq!(
        bind_before_engine_start(&pc.db, &session(), WINDOWS),
        Err(OTHER_ACCOUNT_ON_WINDOWS.to_string())
    );

    assert!(matches!(
        prove(&pc, &session(), MINE, &own_server()),
        Outcome::Adopted(_)
    ));
    assert_eq!(bind_before_engine_start(&pc.db, &session(), WINDOWS), Ok(Bound::Kept));
    assert_eq!(due(&pc).len(), 1);
}

/// Path 3: signing out cannot break the loop. The Windows sign-out refuses while a change waits to upload, and
/// the change cannot upload while sync is refused, so sign-out and sign-in go round. After the proof the held
/// write is still there, and the start proceeds, so it can upload.
#[test]
fn the_sign_out_loop_is_broken_by_the_proof_not_by_deleting_the_held_write() {
    let pc = pc_with(&IDS);
    adopt_unbound(&pc.db, None).unwrap();
    assert!(
        pc.db.0.windows_signout_preflight().is_err(),
        "sign-out refuses: a change waits to upload"
    );
    assert!(pc.db.0.finish_windows_signout().is_err(), "and the purge refuses too");

    assert!(matches!(
        prove(&pc, &session(), MINE, &own_server()),
        Outcome::Adopted(_)
    ));
    assert_eq!(bind_before_engine_start(&pc.db, &session(), WINDOWS), Ok(Bound::Kept));
    assert_eq!(due(&pc).len(), 1, "nothing was deleted to get here");
}

/// Path 4: the owner was recorded by email only, and the account's email changed since. The emails differ, so the
/// binding decided "another account". The proof adopts and records the full identity.
#[test]
fn an_email_only_owner_whose_email_changed_is_proven_and_rerecorded() {
    let pc = pc_with(&IDS);
    pc.db
        .0
        .set_owner(&Identity::new(None, Some("old-address@example.eu")))
        .unwrap();
    assert_eq!(
        bind_before_engine_start(&pc.db, &session(), WINDOWS),
        Err(OTHER_ACCOUNT_ON_WINDOWS.to_string())
    );

    assert!(matches!(
        prove(&pc, &session(), MINE, &own_server()),
        Outcome::Adopted(_)
    ));
    assert_eq!(pc.db.0.owner().unwrap(), Some(session()));
    assert_eq!(bind_before_engine_start(&pc.db, &session(), WINDOWS), Ok(Bound::Kept));
}

/// A session whose user id is not known yet (its profile was not fetched) over unowned data: the same proof
/// applies, and the email-only record it writes is completed by the next start.
#[test]
fn a_session_without_its_user_id_yet_is_proven_by_its_email() {
    let pc = pc_with(&IDS);
    adopt_unbound(&pc.db, None).unwrap();
    let by_email = Identity::new(None, Some("owner@example.eu"));
    assert!(matches!(
        prove(&pc, &by_email, MINE, &own_server()),
        Outcome::Adopted(_)
    ));
    assert_eq!(bind_before_engine_start(&pc.db, &session(), WINDOWS), Ok(Bound::Kept));
    assert_eq!(pc.db.0.owner().unwrap(), Some(session()), "the id completes the record");
}

// ---- R10 holds: another account's data is never adopted, and nothing is deleted ----

/// Another account's files are not in this account's tree. Nothing is adopted, the start still refuses, and every
/// row and the held write are still there, not held for review either.
#[test]
fn another_accounts_data_is_never_adopted() {
    let pc = pc_with(&IDS);
    adopt_unbound(&pc.db, None).unwrap();

    let outcome = prove(&pc, &session(), MINE, &Server::new(Vec::new()));

    assert!(
        matches!(outcome, Outcome::NotProven(t) if t.missing == 3 && t.own == 0),
        "{outcome:?}"
    );
    assert_eq!(pc.db.0.owner().unwrap(), None);
    assert_eq!(
        bind_before_engine_start(&pc.db, &session(), WINDOWS),
        Err(OTHER_ACCOUNT_ON_WINDOWS.to_string())
    );
    assert_eq!(
        due(&pc).len(),
        1,
        "the held write is untouched: not deleted, not held for review"
    );
    assert_eq!(pc.db.0.list_files().unwrap().len(), 3);
}

/// A node of this account's tree whose name does not open under this account's key blocks the adoption.
#[test]
fn a_name_sealed_under_another_key_blocks_adoption() {
    let pc = pc_with(&IDS);
    adopt_unbound(&pc.db, None).unwrap();
    let server = Server::new(vec![mine(IDS[0]), mine(IDS[1]), node(THEIRS, IDS[2], None)]);
    let outcome = prove(&pc, &session(), MINE, &server);
    assert!(
        matches!(outcome, Outcome::NotProven(t) if t.unproven == 1 && t.own == 2),
        "{outcome:?}"
    );
    assert_eq!(pc.db.0.owner().unwrap(), None);
}

/// The lead's case: a database that is 95% this account's own and 5% another account's (rows at the top level
/// that are not in this account's tree) is refused EVERY time, because every row is checked, none sampled.
#[test]
fn a_five_percent_mixed_database_is_refused_every_time() {
    let all = ids("0c000000", 100);
    let theirs: HashSet<usize> = [3, 27, 51, 74, 99].into_iter().collect();
    for _ in 0..25 {
        let pc = empty_pc();
        for id in &all {
            add_row(&pc, id, None, Shape::Snapshot);
        }
        pc.db.0.enqueue_operation(&edit("held-1", &all[0])).unwrap();
        adopt_unbound(&pc.db, None).unwrap();
        let tree = all
            .iter()
            .enumerate()
            .filter(|(n, _)| !theirs.contains(n))
            .map(|(_, id)| mine(id))
            .collect();

        let outcome = prove(&pc, &session(), MINE, &Server::new(tree));

        assert!(
            matches!(outcome, Outcome::NotProven(t) if t.checked == 100 && t.own == 95 && t.missing == 5),
            "{outcome:?}"
        );
        assert_eq!(pc.db.0.owner().unwrap(), None);
        assert_eq!(due(&pc), vec!["held-1".to_string()], "nothing held, nothing deleted");
    }
}

/// Every server-named row is judged, wherever it sits: one row of another account among 300 of this account's is
/// found, at any position.
#[test]
fn every_row_the_server_named_is_judged_not_a_sample() {
    let all = ids("0c000001", 300);
    for theirs in [0, 150, 299] {
        let pc = empty_pc();
        for id in &all {
            add_row(&pc, id, None, Shape::Snapshot);
        }
        adopt_unbound(&pc.db, None).unwrap();
        let tree = all
            .iter()
            .enumerate()
            .filter(|(n, _)| *n != theirs)
            .map(|(_, id)| mine(id))
            .collect();
        let outcome = prove(&pc, &session(), MINE, &Server::new(tree));
        assert!(
            matches!(outcome, Outcome::NotProven(t) if t.checked == 300 && t.missing == 1),
            "{outcome:?}"
        );
    }
}

/// Round 1 sampled only rows with `remote_updated_at > 0`, which the snapshot never writes. A row written by the
/// snapshot, and one written by a delta, are both server-named and both judged.
#[test]
fn rows_written_by_the_snapshot_or_a_delta_are_both_judged() {
    let pc = empty_pc();
    add_row(&pc, IDS[0], None, Shape::Snapshot);
    add_row(&pc, IDS[1], None, Shape::Delta);
    add_row(&pc, IDS[2], None, Shape::Snapshot); // another account's: not in this tree
    adopt_unbound(&pc.db, None).unwrap();
    let outcome = prove(&pc, &session(), MINE, &Server::new(vec![mine(IDS[0]), mine(IDS[1])]));
    assert!(
        matches!(outcome, Outcome::NotProven(t) if t.checked == 3 && t.own == 2 && t.missing == 1),
        "{outcome:?}"
    );
    let named: Vec<(String, bool)> = pc
        .db
        .0
        .own_data_proof_rows()
        .unwrap()
        .into_iter()
        .map(|row| (row.file_id, row.server_named))
        .collect();
    assert!(named.iter().all(|(_, named)| *named), "{named:?}");
}

/// An owner recorded WITH a user id that differs from this session's is another account by R10's strongest
/// signal. No proof is attempted: the server is not even asked.
#[test]
fn an_owner_recorded_by_another_user_id_is_never_questioned() {
    let pc = pc_with(&IDS);
    let alice = Identity::new(Some("user-alice"), Some("alice@example.eu"));
    pc.db.0.set_owner(&alice).unwrap();
    let server = own_server();

    assert_eq!(prove(&pc, &session(), MINE, &server), Outcome::NotEligible);
    assert_eq!(server.listed.get(), 0, "nothing asked");
    assert_eq!(pc.db.0.owner().unwrap(), Some(alice));
}
