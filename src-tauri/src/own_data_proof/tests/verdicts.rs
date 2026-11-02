//! The pure parts, split out of `own_data_proof.rs`.
//!
//! `verdict`, `judge`, `probe` and `eligible` take evidence and return a value; they touch
//! no database and start no engine. The last group pins task 2279 N1: the kept custody
//! memory is not account data, so it does not count as local data when the owner is decided.

use super::*;

    // ---- the pure parts ----

    #[test]
    fn the_verdict_table() {
        let t = |own, neutral, gone, unproven, missing| Tally {
            checked: own + neutral + gone + unproven + missing,
            own,
            neutral,
            gone,
            unproven,
            missing,
            ..Tally::default()
        };
        let rows = [
            (t(0, 0, 0, 0, 0), Verdict::NotProven, "nothing checked"),
            (t(1, 0, 0, 0, 0), Verdict::Adopt, "one row, its own"),
            (t(2, 0, 0, 0, 0), Verdict::Adopt, "two own"),
            (t(1, 0, 1, 0, 0), Verdict::NotProven, "one own of two checked is not enough (P3-1)"),
            (t(3, 0, 1, 0, 0), Verdict::Adopt, "a file gone from an own folder does not block"),
            (t(0, 0, 2, 0, 0), Verdict::NotProven, "only gone files prove nothing"),
            (t(7, 0, 0, 0, 1), Verdict::NotProven, "a file this account cannot see blocks"),
            (t(7, 0, 0, 1, 0), Verdict::NotProven, "a name sealed under another key blocks"),
            (t(2, 1, 0, 0, 0), Verdict::Adopt, "fewer neutral than own"),
            (t(2, 2, 0, 0, 0), Verdict::Adopt, "as many neutral as own"),
            (t(1, 2, 0, 0, 0), Verdict::NotProven, "more neutral than own (P3-1)"),
            (t(0, 3, 0, 0, 0), Verdict::NotProven, "only neutral"),
            (t(1, 1, 0, 0, 0), Verdict::NotProven, "one own of two checked (P3-1: min(2, checked))"),
            (t(40_000, 0, 12, 0, 0), Verdict::Adopt, "a large own tree"),
        ];
        for (tally, expected, why) in rows {
            assert_eq!(verdict(&tally), expected, "{why}: {tally:?}");
        }
    }

    #[test]
    fn a_probe_is_own_only_when_the_name_decrypts_under_this_key_for_this_id() {
        let id = IDS[0];
        assert_eq!(probe(&MINE, &mine(id)), Probe::Own);
        assert_eq!(probe(&MINE, &node(THEIRS, id, None)), Probe::Unproven, "another key");
        assert_eq!(
            probe(&MINE, &with(mine(id), "name_encrypted", serde_json::json!(sealed_name(MINE, IDS[1])))),
            Probe::Unproven,
            "another file's name"
        );
        assert!(name_opens(&MINE, id, &sealed_name(MINE, id)));
        assert!(!name_opens(&THEIRS, id, &sealed_name(MINE, id)));
    }

    #[test]
    fn eligibility() {
        let me = session();
        let me_by_email = Identity::new(None, Some("owner@example.eu"));
        let old_email = Identity::new(None, Some("old@example.eu"));
        let alice = Identity::new(Some("user-alice"), Some("alice@example.eu"));
        let alice_old_email = Identity::new(Some("user-alice"), Some("old@example.eu"));
        assert!(eligible(None, &me, true), "unowned data");
        assert!(!eligible(None, &me, false), "no data");
        assert!(!eligible(Some(&me), &me, true), "already this session's");
        assert!(!eligible(Some(&me_by_email), &me, true), "the same account by email");
        assert!(eligible(Some(&old_email), &me, true), "an email-only owner that differs");
        assert!(eligible(Some(&old_email), &me_by_email, true), "an email-only owner, a session without its id yet");
        // Review P3-2: an owner recorded WITH a user id is compared by user id only. A session whose id is not known
        // yet cannot be compared: not eligible until it is.
        assert!(!eligible(Some(&alice_old_email), &me_by_email, true), "an owner with an id, a session without one");
        assert!(!eligible(Some(&alice), &me, true), "another user id: never");
        assert!(!eligible(None, &Identity::default(), true), "an unknown session: never");
    }

    // ---- task 2279 N1: the kept custody memory is not account data ----

    /// N1 (round 3): `shared_folder_custody_seen:%` survives the Windows sign-out (F2). While it was counted as
    /// account data, every start after a completed sign-out found "unowned local data" and was refused as another
    /// account's — and Windows may not reset, so it never recovered (the task 2267 symptom), and the own-data proof
    /// cannot rescue an empty database. With the row kept and not counted, the start is `Kept`.
    #[test]
    fn sf2279_a_kept_custody_row_does_not_refuse_the_windows_engine_start() {
        let pc = empty_pc();
        let root = "10000000-0000-4000-8000-000000000054";
        pc.db
            .0
            .remember_folder_custody_at_revision(root, pc.db.0.account_revision())
            .unwrap();
        pc.db.0.finish_windows_signout().unwrap();
        assert!(
            pc.db.0.has_seen_folder_custody(root).unwrap(),
            "F2: the sign-out keeps the custody row"
        );
        assert_eq!(
            bind_before_engine_start(&pc.db, &session(), WINDOWS),
            Ok(Bound::Kept),
            "no data and no owner is nothing to reset or refuse"
        );
    }

    /// The same on macOS and Linux, where a reset is allowed: the first engine start after a sign-out must not run the
    /// R10 reset (which would change the sign-in kind and ask for a Finder Repair) because of a custody row it kept.
    #[test]
    fn sf2279_a_kept_custody_row_does_not_reset_the_first_start_after_a_sign_out() {
        let dir = tempfile::tempdir().unwrap();
        let data = CountingReset::open(dir.path());
        let root = "10000000-0000-4000-8000-000000000055";
        data.db
            .remember_folder_custody_at_revision(root, data.db.account_revision())
            .unwrap();
        data.db.purge_all_local_state().unwrap();
        assert!(data.db.has_seen_folder_custody(root).unwrap(), "F2: the purge keeps it");
        assert_eq!(
            bind_before_engine_start(&data, &session(), true),
            Ok(Bound::Kept),
            "nothing here belongs to an account: no reset"
        );
        assert_eq!(data.resets.get(), 0, "the R10 reset path did not run");
    }

    /// The copy a refused Windows start shows never tells a person they have another account.
    #[test]
    fn the_refusal_never_claims_another_account() {
        for sentence in [OTHER_ACCOUNT_ON_WINDOWS, IDENTITY_UNKNOWN] {
            let lower = sentence.to_lowercase();
            assert!(!lower.contains("another"), "{sentence}");
            assert!(!lower.contains("other account"), "{sentence}");
        }
        assert!(OTHER_ACCOUNT_ON_WINDOWS.contains("Nothing was uploaded or deleted"));
    }
