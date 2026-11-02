//! Pins for the kept folder the app saves and dismisses, and for the sentence and alert
//! the person reads.
use super::*;

    // ── round 2: the saved kept folder (review I2, lead ruling) ──────────────

    #[test]
    fn test_1882_r2_the_latest_kept_folder_is_saved_and_replaces_an_older_one() {
        let mut cfg = crate::config::DesktopConfig::default();
        assert!(record_kept_folder(&mut cfg, "/Users/someone/Kept (1)"));
        assert_eq!(cfg.kept_unsynced_folder.as_deref(), Some("/Users/someone/Kept (1)"));
        assert!(record_kept_folder(&mut cfg, "/Users/someone/Kept (2)"));
        assert_eq!(cfg.kept_unsynced_folder.as_deref(), Some("/Users/someone/Kept (2)"));
        assert!(
            !record_kept_folder(&mut cfg, "/Users/someone/Kept (2)"),
            "the same folder again changes nothing"
        );
    }

    #[test]
    fn test_1882_r2_dismissing_clears_only_the_folder_the_row_showed() {
        let mut cfg = crate::config::DesktopConfig::default();
        record_kept_folder(&mut cfg, "/Users/someone/Kept (2)");
        assert!(
            !dismiss_kept_folder(&mut cfg, "/Users/someone/Kept (1)"),
            "a row showing an older folder cannot dismiss a newer one"
        );
        assert_eq!(cfg.kept_unsynced_folder.as_deref(), Some("/Users/someone/Kept (2)"));
        assert!(dismiss_kept_folder(&mut cfg, "/Users/someone/Kept (2)"));
        assert_eq!(cfg.kept_unsynced_folder, None);
        assert!(
            !dismiss_kept_folder(&mut cfg, "/Users/someone/Kept (2)"),
            "nothing left to dismiss"
        );
    }

    #[test]
    fn test_1882_r2_the_saved_folder_survives_a_settings_save_and_a_restart() {
        let mut cfg = crate::config::DesktopConfig::default();
        record_kept_folder(
            &mut cfg,
            "/Users/someone/Library/CloudStorage/Beebeeb-Beebeeb (10-10-2026 10:50)",
        );
        let settings = crate::config::DesktopSettings::from(&crate::config::DesktopConfig::default());
        cfg.apply_settings(settings);
        assert_eq!(
            cfg.kept_unsynced_folder.as_deref(),
            Some("/Users/someone/Library/CloudStorage/Beebeeb-Beebeeb (10-10-2026 10:50)"),
            "a settings save never clears it"
        );
        let toml = toml::to_string(&cfg).expect("serialize");
        let back: crate::config::DesktopConfig = toml::from_str(&toml).expect("parse");
        assert_eq!(
            back.kept_unsynced_folder, cfg.kept_unsynced_folder,
            "it survives a restart"
        );
        let fresh = toml::to_string(&crate::config::DesktopConfig::default()).expect("serialize");
        assert!(
            !fresh.contains("kept_unsynced_folder"),
            "absent until something was kept"
        );
    }

    #[test]
    fn test_1882_r2_the_folder_states_match_the_bridge() {
        the_folder_states_match_the_bridge(include_str!("../../../macos/FileProviderBridge.m"));
    }


    // ── the sentence and the alert ───────────────────────────────────────────

    #[test]
    fn test_1882_alert_text_is_the_sentence_a_blank_line_then_the_folder() {
        let folder = "/Users/someone/Library/CloudStorage/Kept";
        assert_eq!(
            preserved_files_message(folder),
            format!("{PRESERVED_FILES_SENTENCE}\n\n{folder}")
        );
        assert_eq!(PRESERVED_FILES_TITLE, "Files kept on this Mac");
        assert_eq!(
            PRESERVED_FILES_SENTENCE,
            "Files that hadn’t reached your vault yet were kept on this Mac, in this folder:"
        );
    }

    /// Review M4: the nothing-kept side of the alert, as a decision a mutation can turn red.
    #[test]
    fn test_1882_r2_nothing_kept_raises_no_alert() {
        assert_eq!(kept_folder_alert(None), None);
        let folder = "/Users/someone/Library/CloudStorage/Beebeeb-Beebeeb (10-10-2026 10:50)";
        assert_eq!(
            kept_folder_alert(Some(folder)),
            Some((PRESERVED_FILES_TITLE, format!("{PRESERVED_FILES_SENTENCE}\n\n{folder}")))
        );
    }

    #[test]
    fn test_1882_the_sentence_names_no_provider_and_holds_no_path() {
        for text in [PRESERVED_FILES_SENTENCE, PRESERVED_FILES_TITLE] {
            let lower = text.to_lowercase();
            for word in [
                "apple",
                "icloud",
                "hetzner",
                "file provider",
                "fileprovider",
                "cloudstorage",
                "/",
            ] {
                assert!(!lower.contains(word), "{text:?} must not contain {word:?}");
            }
        }
    }

    // ── where the kept folder goes ───────────────────────────────────────────

    #[test]
    fn test_1882_kept_location_returns_the_folder_and_nothing_when_nothing_was_kept() {
        let folder = "/Users/someone/Kept".to_string();
        assert_eq!(removal(kept(&folder, true)).kept_location("sign-out"), Some(folder));
        assert_eq!(
            removal(KeptFolder::Nothing(NothingKept::NotReported)).kept_location("sign-out"),
            None
        );
    }

    #[test]
    fn test_1882_sign_out_carries_a_kept_folder_and_survives_a_failed_removal() {
        let folder = "/Users/someone/Kept".to_string();
        assert_eq!(sign_out_kept_location(Ok(removal(kept(&folder, true)))), Some(folder));
        assert_eq!(sign_out_kept_location(Ok(DomainRemoval::default())), None);
        assert_eq!(sign_out_kept_location(Err("not registered".to_string().into())), None);
    }

    #[test]
    fn test_1882_repair_carries_a_kept_folder_and_turns_a_failure_into_one_warning() {
        let folder = "/Users/someone/Kept".to_string();

        let mut warnings = Vec::new();
        assert_eq!(
            repair_removal(Ok(removal(kept(&folder, true))), &mut warnings),
            (true, Some(folder))
        );
        assert!(warnings.is_empty());

        let mut warnings = Vec::new();
        assert_eq!(
            repair_removal(Ok(DomainRemoval::default()), &mut warnings),
            (true, None)
        );
        assert!(warnings.is_empty());

        let mut warnings = Vec::new();
        assert_eq!(
            repair_removal(Err("busy".to_string().into()), &mut warnings),
            (false, None)
        );
        assert_eq!(
            warnings,
            vec!["Could not remove Finder File Provider domain: busy".to_string()]
        );
    }
