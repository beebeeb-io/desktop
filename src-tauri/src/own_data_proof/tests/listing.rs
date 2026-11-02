//! The listing, split out of `own_data_proof.rs`.
//!
//! One `GET /api/v1/sync/snapshot` decides what "the server's answer" means: the tree, a
//! signed-out session, or an unavailable server. These tests pin the mapping from a status
//! code and from the snapshot/trash pair, and what a row that is gone from a folder this
//! account owns counts as.

use super::*;

    // ---- the listing ----

    /// Offline, or the server failing: the listing is asked twice, then nothing is decided and nothing recorded; a
    /// later start asks again.
    #[test]
    fn no_answer_decides_nothing() {
        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        let server = own_server().failing_first(vec![Listing::Unavailable, Listing::Unavailable]);

        let outcome = prove(&pc, &session(), MINE, &server);

        assert!(matches!(outcome, Outcome::Unavailable(t) if t.unavailable == 1 && t.signed_out == 0), "{outcome:?}");
        assert_eq!(server.listed.get(), 2);
        assert_eq!(pc.db.0.owner().unwrap(), None);
        assert_eq!(due(&pc).len(), 1);
    }

    /// A listing that does not answer the first time is asked once more.
    #[test]
    fn a_listing_that_does_not_answer_is_asked_once_more() {
        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        let server = own_server().failing_first(vec![Listing::Unavailable]);
        assert!(matches!(prove(&pc, &session(), MINE, &server), Outcome::Adopted(t) if t.own == 3));
        assert_eq!(server.listed.get(), 2);
    }

    /// A token the server refuses (401) is not "offline": it is told apart, so the refusal does not tell the person to
    /// connect to the internet, and it is not asked again.
    #[test]
    fn a_refused_token_is_told_apart_from_no_connection() {
        assert!(matches!(listing_from_status(Some(401)), Listing::SignedOut));
        for status in [None, Some(404), Some(429), Some(500), Some(503)] {
            assert!(matches!(listing_from_status(status), Listing::Unavailable), "{status:?}");
        }
        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        let server = own_server().failing_first(vec![Listing::SignedOut]);
        let outcome = prove(&pc, &session(), MINE, &server);
        assert!(matches!(outcome, Outcome::Unavailable(t) if t.signed_out == 1 && t.unavailable == 0), "{outcome:?}");
        assert_eq!(server.listed.get(), 1);
    }

    /// A session that changed before the listing (a Lock, a sign-out) asks nothing: the key and token are not used.
    #[test]
    fn a_session_that_changed_before_the_listing_asks_nothing() {
        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        let server = own_server();
        let outcome = block_on(prove_and_adopt(&pc.db.0, &session(), &MINE, || server.list(), || false)).unwrap();
        assert!(matches!(outcome, Outcome::Changed(_)), "{outcome:?}");
        assert_eq!(server.listed.get(), 0);
        assert_eq!(pc.db.0.owner().unwrap(), None);
    }

    /// A session that changed after the listing records nothing, and holds nothing.
    #[test]
    fn a_session_that_changed_during_the_proof_records_nothing() {
        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        let server = own_server();
        let outcome = block_on(prove_and_adopt(
            &pc.db.0,
            &session(),
            &MINE,
            || server.list(),
            || server.listed.get() == 0,
        ))
        .unwrap();
        assert!(matches!(outcome, Outcome::Changed(_)), "{outcome:?}");
        assert_eq!(pc.db.0.owner().unwrap(), None);
        assert_eq!(due(&pc).len(), 1, "nothing held");
    }

    /// An owner recorded by someone else while the proof ran is never replaced, and nothing is held.
    #[test]
    fn an_owner_recorded_during_the_proof_is_never_replaced() {
        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        add_folder(&pc, "new-folder", None, Shape::Local);
        pc.db.0.enqueue_operation(&create("op-new", "new-folder", None, true)).unwrap();
        let server = own_server();
        let racer = Identity::new(Some("user-racer"), None);
        let outcome = block_on(prove_and_adopt(
            &pc.db.0,
            &session(),
            &MINE,
            || {
                pc.db.0.set_owner(&racer).unwrap();
                server.list()
            },
            || true,
        ))
        .unwrap();
        assert!(matches!(outcome, Outcome::Changed(_)), "{outcome:?}");
        assert_eq!(pc.db.0.owner().unwrap(), Some(racer));
        assert_eq!(due(&pc).len(), 2, "nothing held when nothing was recorded");
    }

    /// A Windows sign-out that emptied the database while the proof ran (its purge also removes the owner record):
    /// nothing is recorded, so the next account to sign in on this PC finds no owner and no data.
    #[test]
    fn an_owner_is_never_recorded_on_a_database_emptied_during_the_proof() {
        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        let server = own_server();
        let outcome = block_on(prove_and_adopt(
            &pc.db.0,
            &session(),
            &MINE,
            || {
                if pc.db.0.has_account_data().unwrap() {
                    pc.db.0.remove_operation("held-1").unwrap();
                    pc.db.0.finish_windows_signout().unwrap();
                }
                server.list()
            },
            || true,
        ))
        .unwrap();
        assert!(matches!(outcome, Outcome::Changed(_) | Outcome::NotProven(_)), "{outcome:?}");
        assert_eq!(pc.db.0.owner().unwrap(), None);
        assert!(!pc.db.0.has_account_data().unwrap());
    }

    /// Nothing the server named on this PC: nothing can be proven, so nothing is adopted.
    #[test]
    fn data_without_any_server_row_is_never_adopted() {
        let pc = empty_pc();
        add_row(&pc, "new-file", None, Shape::Local);
        pc.db.0.enqueue_operation(&create("op-new", "new-file", None, false)).unwrap();
        adopt_unbound(&pc.db, None).unwrap();
        let outcome = prove(&pc, &session(), MINE, &own_server());
        assert!(matches!(outcome, Outcome::NotProven(t) if t.checked == 0), "{outcome:?}");
        assert_eq!(pc.db.0.owner().unwrap(), None);
        assert_eq!(due(&pc).len(), 1);
    }

    // ---- rows gone since ----

    /// A file deleted since from one of this account's own folders does not block adoption: it is gone.
    #[test]
    fn a_file_deleted_from_one_of_this_accounts_folders_does_not_block_adoption() {
        let folder = "0a000000-0000-4000-8000-0000000000f0";
        let pc = pc_with(&IDS);
        add_folder(&pc, folder, None, Shape::Snapshot);
        add_row(&pc, "0a000000-0000-4000-8000-0000000000f1", Some(folder), Shape::Snapshot);
        adopt_unbound(&pc.db, None).unwrap();
        let mut tree: Vec<_> = IDS.iter().map(|id| mine(id)).collect();
        tree.push(mine(folder));
        let outcome = prove(&pc, &session(), MINE, &Server::new(tree));
        assert!(matches!(outcome, Outcome::Adopted(t) if t.gone == 1 && t.missing == 0), "{outcome:?}");
    }

    /// The nearest ancestor that IS in the tree decides: a file under a folder deleted since, inside a folder of this
    /// account's own, is gone.
    #[test]
    fn a_file_under_a_deleted_folder_inside_an_own_folder_is_gone() {
        let (top, deleted, file) = ("folder-top", "folder-deleted", "file-deleted");
        let pc = pc_with(&IDS);
        add_folder(&pc, top, None, Shape::Snapshot);
        add_folder(&pc, deleted, Some(top), Shape::Snapshot);
        add_row(&pc, file, Some(deleted), Shape::Snapshot);
        adopt_unbound(&pc.db, None).unwrap();
        let mut tree: Vec<_> = IDS.iter().map(|id| mine(id)).collect();
        tree.push(mine(top));
        let outcome = prove(&pc, &session(), MINE, &Server::new(tree));
        assert!(matches!(outcome, Outcome::Adopted(t) if t.gone == 2 && t.missing == 0), "{outcome:?}");
    }

    /// A file not in this account's tree whose folders are not in it either (up to the top) is not explained: it
    /// blocks. So does one whose nearest ancestor in the tree proves nothing (a plaintext name).
    #[test]
    fn a_missing_file_that_no_own_folder_explains_blocks_adoption() {
        let pc = pc_with(&IDS);
        add_folder(&pc, "their-folder", None, Shape::Snapshot);
        add_row(&pc, "their-file", Some("their-folder"), Shape::Snapshot);
        add_folder(&pc, "plain-folder", None, Shape::Snapshot);
        add_row(&pc, "file-in-plain", Some("plain-folder"), Shape::Snapshot);
        adopt_unbound(&pc.db, None).unwrap();
        let mut tree: Vec<_> = IDS.iter().map(|id| mine(id)).collect();
        tree.push(with(mine("plain-folder"), "name_encrypted", serde_json::json!("Photos")));
        let outcome = prove(&pc, &session(), MINE, &Server::new(tree));
        assert!(
            matches!(outcome, Outcome::NotProven(t) if t.missing == 3 && t.gone == 0 && t.neutral == 1),
            "{outcome:?}"
        );
    }

    /// Local parents that point at each other end the walk as missing; the proof terminates.
    #[test]
    fn a_cycle_in_local_parents_ends_as_missing() {
        let pc = pc_with(&IDS);
        add_folder(&pc, "loop-a", Some("loop-b"), Shape::Snapshot);
        add_folder(&pc, "loop-b", Some("loop-a"), Shape::Snapshot);
        adopt_unbound(&pc.db, None).unwrap();
        let outcome = prove(&pc, &session(), MINE, &own_server());
        assert!(matches!(outcome, Outcome::NotProven(t) if t.missing == 2), "{outcome:?}");
    }
