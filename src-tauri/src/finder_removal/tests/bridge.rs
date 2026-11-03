//! Pins for the bridge reply: what the decoded removal says arrived back from
//! `FileProviderBridge.m`, and the bounded re-check of a folder that reads as missing.
use super::*;

    #[test]
    fn test_1882_bridge_reply_with_a_kept_folder_reports_that_exact_folder() {
        let folder = "/Users/someone/Library/CloudStorage/Beebeeb (kept) /Notes ";
        assert_eq!(
            removal_from_bridge(0, KEPT_HAS_ENTRIES, Some(folder.to_string()), None),
            Ok(removal(kept(folder, true))),
            "the folder is reported byte for byte, trailing space included"
        );
    }

    #[test]
    fn test_1882_bridge_reply_with_nothing_kept_reports_no_folder() {
        assert_eq!(
            removal_from_bridge(0, KEPT_NONE_REPORTED, None, None),
            Ok(removal(KeptFolder::Nothing(NothingKept::NotReported)))
        );
        // A stale buffer never turns "nothing kept" into a folder.
        assert_eq!(
            removal_from_bridge(0, KEPT_NONE_REPORTED, Some("/leftover".to_string()), None),
            Ok(removal(KeptFolder::Nothing(NothingKept::NotReported)))
        );
    }

    #[test]
    fn test_1882_bridge_errors_never_invent_a_folder() {
        assert_eq!(
            removal_from_bridge(
                -1,
                KEPT_NONE_REPORTED,
                Some("/leftover".to_string()),
                Some("no provider (NSFileProviderErrorDomain -2001)".to_string())
            ),
            Err(RemovalFailure::from(
                "no provider (NSFileProviderErrorDomain -2001)".to_string()
            ))
        );
        assert!(removal_from_bridge(-1, KEPT_NONE_REPORTED, None, None).is_err());
        assert!(
            removal_from_bridge(1, KEPT_HAS_ENTRIES, Some("/x".to_string()), None).is_err(),
            "round 1's reply 1 is gone: the bridge answers 0 or -1"
        );
        assert!(
            removal_from_bridge(7, KEPT_NONE_REPORTED, None, None).is_err(),
            "an unknown reply is an error"
        );
    }

    // ── round 2: is anything actually there? (device K-F2, review I3, M3) ───

    #[test]
    fn test_1882_r2_a_reported_folder_that_is_missing_keeps_nothing() {
        // Round 3 (re-review D1): an EMPTY folder is no longer in this list; it is kept (below).
        let folder = "/Users/someone/Library/CloudStorage/Beebeeb-Drive (10-10-2026 10:52)";
        for (state, reason) in [
            (KEPT_NONE_REPORTED, NothingKept::NotReported),
            (KEPT_MISSING, NothingKept::Missing),
        ] {
            let decoded = kept_folder_from_bridge(state, Some(folder.to_string()));
            assert_eq!(decoded, KeptFolder::Nothing(reason), "state {state}");
            assert_eq!(
                removal(decoded).kept_location("sign-out"),
                None,
                "state {state}: no alert, no row"
            );
        }
        let mut warnings = Vec::new();
        assert_eq!(
            repair_removal(
                Ok(removal(kept_folder_from_bridge(KEPT_MISSING, Some(folder.to_string())))),
                &mut warnings
            ),
            (true, None)
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_1882_r2_a_folder_with_an_entry_or_that_cannot_be_checked_is_kept() {
        let folder = "/Users/someone/Library/CloudStorage/Beebeeb-Beebeeb (10-10-2026 10:50)";
        assert_eq!(
            kept_folder_from_bridge(KEPT_HAS_ENTRIES, Some(folder.to_string())),
            kept(folder, true)
        );
        assert_eq!(
            kept_folder_from_bridge(KEPT_UNCHECKED, Some(folder.to_string())),
            kept(folder, false),
            "spec §4's fallback: an existing folder the sandbox would not list is shown"
        );
        for state in [KEPT_HAS_ENTRIES, KEPT_UNCHECKED] {
            assert_eq!(
                removal(kept_folder_from_bridge(state, Some(folder.to_string()))).kept_location("sign-out"),
                Some(folder.to_string())
            );
        }
    }

    // ── round 3: the kept-folder decision must never hide kept files (re-review D1) ─

    #[test]
    fn test_1882_r3_an_existing_empty_folder_is_kept_like_an_unlistable_one() {
        let folder = "/Users/someone/Library/CloudStorage/Beebeeb-Beebeeb (10-10-2026 10:50)";
        let decoded = kept_folder_from_bridge(KEPT_EMPTY, Some(folder.to_string()));
        assert_eq!(
            decoded,
            kept_empty(folder),
            "an empty folder that exists is reported as kept"
        );
        assert_eq!(
            removal(decoded.clone()).kept_location("sign-out"),
            Some(folder.to_string()),
            "sign-out raises the alert for it"
        );
        let mut warnings = Vec::new();
        assert_eq!(
            repair_removal(Ok(removal(decoded.clone())), &mut warnings),
            (true, Some(folder.to_string())),
            "Repair reports it for the row"
        );
        assert!(warnings.is_empty());
        // The same on a failed removal (review M2).
        let failure = removal_from_bridge(-1, KEPT_EMPTY, Some(folder.to_string()), Some("busy".to_string()))
            .expect_err("the removal reported an error");
        assert_eq!(failure.kept, kept_empty(folder));
        // An empty folder is told apart from a full one only in the log, never to the person.
        assert_ne!(kept_empty(folder), kept(folder, true));
        // Empty with no path is "folder unknown", as any state without a path.
        assert_eq!(kept_folder_from_bridge(KEPT_EMPTY, None), KeptFolder::Unknown);
    }

    /// What `settle_kept_state` did, with the answers it was given. No sleeping: the wait is
    /// injected and only recorded.
    struct Settled {
        state: i32,
        asked: Vec<String>,
        waits: Vec<std::time::Duration>,
    }

    fn settle(state: i32, location: Option<&str>, answers: &[i32]) -> Settled {
        let asked = std::cell::RefCell::new(Vec::<String>::new());
        let waits = std::cell::RefCell::new(Vec::<std::time::Duration>::new());
        let state = settle_kept_state(
            state,
            location,
            |path| {
                let mut asked = asked.borrow_mut();
                asked.push(path.to_string());
                answers.get(asked.len() - 1).copied().unwrap_or(KEPT_MISSING)
            },
            |wait| waits.borrow_mut().push(wait),
        );
        Settled {
            state,
            asked: asked.into_inner(),
            waits: waits.into_inner(),
        }
    }

    #[test]
    fn test_1882_r3_a_missing_folder_that_appears_within_the_window_is_kept() {
        let folder = "/Users/someone/Library/CloudStorage/Beebeeb-Beebeeb (10-10-2026 10:50) ";
        let settled = settle(
            KEPT_MISSING,
            Some(folder),
            &[KEPT_MISSING, KEPT_MISSING, KEPT_HAS_ENTRIES],
        );
        assert_eq!(settled.state, KEPT_HAS_ENTRIES, "the third look found the files");
        assert_eq!(
            settled.asked,
            vec![folder.to_string(); 3],
            "each look is at the exact reported path"
        );
        assert_eq!(
            settled.waits,
            vec![KEPT_MISSING_RECHECK_INTERVAL; 3],
            "one wait before each look"
        );
        assert_eq!(
            removal_from_bridge(0, settled.state, Some(folder.to_string()), None)
                .map(|removal| removal.kept_location("sign-out")),
            Ok(Some(folder.to_string())),
            "and the files reach the alert"
        );

        // It may appear empty first: it exists, so it is kept too (above).
        let empty_first = settle(KEPT_MISSING, Some(folder), &[KEPT_EMPTY]);
        assert_eq!(empty_first.state, KEPT_EMPTY);
        assert_eq!(
            empty_first.waits.len(),
            1,
            "it stops looking at the first sight of the folder"
        );
    }

    #[test]
    fn test_1882_r3_a_folder_missing_throughout_is_nothing_kept_after_a_bounded_wait() {
        let folder = "/Users/someone/Library/CloudStorage/Beebeeb-Drive (10-10-2026 10:52)";
        let settled = settle(KEPT_MISSING, Some(folder), &[]);
        assert_eq!(settled.state, KEPT_MISSING);
        assert_eq!(
            settled.asked.len(),
            KEPT_MISSING_RECHECKS as usize,
            "a few looks, then it stops"
        );
        assert_eq!(settled.waits.len(), KEPT_MISSING_RECHECKS as usize);
        let total: std::time::Duration = settled.waits.iter().sum();
        assert!(
            total >= std::time::Duration::from_millis(750) && total <= std::time::Duration::from_millis(1500),
            "about one second in all, not instant and not long: {total:?}"
        );
        // And then it is silent, as before (spec §5).
        let decoded = kept_folder_from_bridge(settled.state, Some(folder.to_string()));
        assert_eq!(decoded, KeptFolder::Nothing(NothingKept::Missing));
        assert_eq!(removal(decoded).kept_location("sign-out"), None);
    }

    #[test]
    fn test_1882_r3_only_a_missing_folder_with_a_path_is_looked_at_again() {
        for state in [
            KEPT_NONE_REPORTED,
            KEPT_EMPTY,
            KEPT_HAS_ENTRIES,
            KEPT_UNCHECKED,
            KEPT_NO_PATH,
            9,
        ] {
            let settled = settle(state, Some("/Users/someone/Kept"), &[KEPT_HAS_ENTRIES]);
            assert_eq!(settled.state, state, "state {state} is final");
            assert!(
                settled.asked.is_empty() && settled.waits.is_empty(),
                "state {state}: no look, no wait"
            );
        }
        for location in [None, Some(""), Some("   ")] {
            let settled = settle(KEPT_MISSING, location, &[KEPT_HAS_ENTRIES]);
            assert_eq!(settled.state, KEPT_MISSING, "no path to look at: {location:?}");
            assert!(
                settled.asked.is_empty() && settled.waits.is_empty(),
                "{location:?}: no look, no wait"
            );
        }
    }

    /// Re-review D1: both removal paths (Repair/sign-out/rollback through `remove()`, and the
    /// app-start sweep through `remove_domain`) decode the bridge's reply in ONE place, and that
    /// place settles a missing folder first and decodes what the settle found. A path that skips
    /// it would hide a late-filled folder. The behaviour is tested in `macos_file_provider`
    /// (`test_1882_r3_the_decode_uses_what_the_settle_found`); this pins that the real decode is
    /// the one every removal goes through, with the real look and the real wait.
    #[test]
    fn test_1882_r3_every_removal_settles_a_missing_folder_before_it_decides() {
        every_removal_settles_a_missing_folder(include_str!("../../macos_file_provider.rs"));
    }


    #[test]
    fn test_1882_r2_a_url_without_a_path_is_removed_with_the_folder_unknown() {
        assert_eq!(kept_folder_from_bridge(KEPT_NO_PATH, None), KeptFolder::Unknown);
        for blank in [None, Some(String::new()), Some("   ".to_string())] {
            assert_eq!(
                kept_folder_from_bridge(KEPT_HAS_ENTRIES, blank.clone()),
                KeptFolder::Unknown
            );
            assert_eq!(kept_folder_from_bridge(KEPT_UNCHECKED, blank), KeptFolder::Unknown);
        }
        // Its own outcome: the domain IS removed, so Repair says so, with no warning and no folder.
        let mut warnings = Vec::new();
        assert_eq!(
            repair_removal(Ok(removal(KeptFolder::Unknown)), &mut warnings),
            (true, None)
        );
        assert!(
            warnings.is_empty(),
            "removed; folder unknown is not a failed removal: {warnings:?}"
        );
        assert_eq!(sign_out_kept_location(Ok(removal(KeptFolder::Unknown))), None);
    }

    #[test]
    fn test_1882_r2_an_unknown_state_never_hides_a_path() {
        assert_eq!(
            kept_folder_from_bridge(9, Some("/Users/someone/Kept".to_string())),
            kept("/Users/someone/Kept", false)
        );
        assert_eq!(
            kept_folder_from_bridge(-4, Some("/Users/someone/Kept".to_string())),
            kept("/Users/someone/Kept", false)
        );
        assert_eq!(kept_folder_from_bridge(9, None), KeptFolder::Unknown);
    }


    // ── round 2: an error that carries a folder (M2) ───────────────────────────

    #[test]
    fn test_1882_r2_a_reply_with_an_error_and_a_folder_keeps_the_folder() {
        let folder = "/Users/someone/Library/CloudStorage/Beebeeb-Beebeeb (10-10-2026 10:50)";
        let failure = removal_from_bridge(
            -1,
            KEPT_HAS_ENTRIES,
            Some(folder.to_string()),
            Some("busy (NSFileProviderErrorDomain -1001)".to_string()),
        )
        .expect_err("the removal reported an error");
        assert_eq!(failure.message, "busy (NSFileProviderErrorDomain -1001)");
        assert_eq!(failure.kept, kept(folder, true), "the folder rides with the error");

        // Sign-out: the failure is still only logged, but the folder reaches the alert.
        assert_eq!(sign_out_kept_location(Err(failure.clone())), Some(folder.to_string()));
        // Repair: one warning, as before, and the folder for the row.
        let mut warnings = Vec::new();
        assert_eq!(
            repair_removal(Err(failure), &mut warnings),
            (false, Some(folder.to_string()))
        );
        assert_eq!(
            warnings,
            vec!["Could not remove Finder File Provider domain: busy (NSFileProviderErrorDomain -1001)".to_string()]
        );

        // An error with a missing or empty folder is still "nothing kept".
        let failure =
            removal_from_bridge(-1, KEPT_MISSING, Some(folder.to_string()), Some("busy".to_string())).unwrap_err();
        assert_eq!(failure.kept, KeptFolder::Nothing(NothingKept::Missing));
    }

    /// Review M2: the bridge checks the reported folder BEFORE it looks at the error, so a reply
    /// that carries both reaches Rust with the folder's state.
    #[test]
    fn test_1882_r2_the_bridge_checks_the_folder_before_the_error() {
        the_bridge_checks_the_folder_before_the_error(include_str!("../../../macos/FileProviderBridge.m"));
    }
