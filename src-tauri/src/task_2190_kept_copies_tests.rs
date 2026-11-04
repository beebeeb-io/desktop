    use super::*;

    /// Review-169 blocker: two "Keep the other" copies and a sign-out's kept folder are all listed
    /// and revealable; the sign-out's one folder slot and its unfinished list still name only the
    /// sign-out's folder, so its prompt never takes a decision's copy for an interrupted sign-out.
    #[test]
    fn task_2190_two_kept_copies_and_a_sign_out_folder_are_all_listed() {
        use sign_out::kept_record::{
            dismiss_saved_conflict_copy_at, save_kept_unsent_durably, saved_conflict_copies_at, saved_kept_unsent_at,
            unfinished_at,
        };
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Changes not sent");
        let config_path = dir.path().join("desktop.toml");
        let sign_out_folder = root.join("2026-10-28 at 09.00.00").to_string_lossy().into_owned();
        save_kept_unsent_durably(&config_path, &sign_out_folder, &config::RealConfigFs).unwrap();
        let (a, b) = (dir.path().join("a.staged"), dir.path().join("b.staged"));
        std::fs::write(&a, b"edited on this Mac (a)").unwrap();
        std::fs::write(&b, b"edited on this Mac (b)").unwrap();
        let clone = sign_out::kept_writer::SystemClone;
        let copy_a = keep_parked_copy_at("op-a", &a, "docs/a.txt", &root, &config_path, &clone, &config::RealConfigFs).unwrap();
        let copy_b = keep_parked_copy_at("op-b", &b, "b.txt", &root, &config_path, &clone, &config::RealConfigFs).unwrap();
        assert_eq!(std::fs::read(&copy_a).unwrap(), b"edited on this Mac (a)");
        assert_eq!(std::fs::read(&copy_b).unwrap(), b"edited on this Mac (b)");
        let listed = saved_conflict_copies_at(&config_path).unwrap();
        assert_eq!(
            listed,
            vec![copy_a.to_string_lossy().into_owned(), copy_b.to_string_lossy().into_owned()],
            "both copies listed, oldest first"
        );
        for copy in &listed {
            assert!(conflict_copy_reveal_target(&listed, copy).is_ok(), "revealable: {copy}");
        }
        assert!(conflict_copy_reveal_target(&listed, &sign_out_folder).is_err(), "only a listed copy is revealed as one");
        assert_eq!(saved_kept_unsent_at(&config_path).unwrap(), Some(sign_out_folder.clone()), "the sign-out's folder is still saved");
        assert!(reveal_target(Some(&sign_out_folder), &sign_out_folder).is_ok(), "and still revealable");
        assert_eq!(unfinished_at(&config_path).unwrap(), vec![sign_out_folder.clone()], "the sign-out prompt names only its own folder");
        assert!(dismiss_saved_conflict_copy_at(&config_path, &listed[0]).unwrap());
        assert_eq!(saved_conflict_copies_at(&config_path).unwrap(), vec![listed[1].clone()]);
        assert!(std::path::Path::new(&listed[0]).exists(), "Dismiss never deletes the copy");
    }

    /// Review-169 #8: a decision that changed something tells an open Sync tab to read its rows again.
    #[test]
    fn task_2190_a_decision_tells_an_open_sync_tab() {
        let source = include_str!("lib.rs").replace("\r\n", "\n");
        let decide = &source[source.find("async fn resolve_parked_conflict(").expect("the decision command")..];
        let decide = &decide[..decide.find("\n}\n").expect("its end")];
        assert!(decide.contains(
            "if done != engine_bridge::ParkedResolution::Moved {\n            emit_kept_folder_changed(&app);"
        ));
    }
