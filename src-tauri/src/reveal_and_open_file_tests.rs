    use super::*;
    use std::path::Path;

    // ── reject_unsafe_rel_path ───────────────────────────────────────────────

    #[test]
    fn safe_paths_are_accepted() {
        assert!(reject_unsafe_rel_path("Documents/report.pdf").is_ok());
        assert!(reject_unsafe_rel_path("a/b/c.txt").is_ok());
        assert!(reject_unsafe_rel_path("Photos/2024/vacation.jpg").is_ok());
        assert!(reject_unsafe_rel_path("file.txt").is_ok());
    }

    #[test]
    fn rejects_dotdot_forward_slash() {
        assert!(reject_unsafe_rel_path("../etc/passwd").is_err());
    }

    #[test]
    fn rejects_dotdot_mid_path() {
        assert!(reject_unsafe_rel_path("a/../../b").is_err());
    }

    #[test]
    fn rejects_current_dir_component() {
        assert!(reject_unsafe_rel_path("a/./b").is_err());
    }

    #[test]
    fn rejects_empty_component() {
        assert!(reject_unsafe_rel_path("a//b").is_err());
    }

    #[test]
    fn rejects_unix_absolute() {
        assert!(reject_unsafe_rel_path("/etc/passwd").is_err());
    }

    #[test]
    fn rejects_unc_path_backslash() {
        assert!(reject_unsafe_rel_path("\\\\server\\share").is_err());
    }

    #[test]
    fn rejects_unc_path_forward_slash() {
        assert!(reject_unsafe_rel_path("//server/share").is_err());
    }

    #[test]
    fn rejects_windows_drive_absolute() {
        assert!(reject_unsafe_rel_path("C:\\Windows\\x").is_err());
    }

    #[test]
    fn rejects_windows_drive_relative() {
        // 'C:foo' is still drive-prefixed, not a plain relative path.
        assert!(reject_unsafe_rel_path("C:foo").is_err());
    }

    #[test]
    fn rejects_empty() {
        assert!(reject_unsafe_rel_path("").is_err());
    }

    #[test]
    fn rejects_whitespace_only() {
        assert!(reject_unsafe_rel_path("   ").is_err());
    }

    #[test]
    fn rejects_dotdot_backslash_separator() {
        // Backslash-separated traversal must also be caught.
        assert!(reject_unsafe_rel_path("a\\..\\b").is_err());
    }

    // ── is_dangerous_executable ──────────────────────────────────────────────

    #[test]
    fn exe_is_dangerous() {
        assert!(is_dangerous_executable(Path::new("evil.exe")));
    }

    #[test]
    fn bat_is_dangerous() {
        assert!(is_dangerous_executable(Path::new("run.bat")));
    }

    #[test]
    fn lnk_is_dangerous() {
        assert!(is_dangerous_executable(Path::new("shortcut.lnk")));
    }

    #[test]
    fn ps1_is_dangerous() {
        assert!(is_dangerous_executable(Path::new("script.ps1")));
    }

    #[test]
    fn uppercase_exe_is_dangerous() {
        // Extension check must be case-insensitive.
        assert!(is_dangerous_executable(Path::new("evil.EXE")));
    }

    #[test]
    fn pdf_is_safe() {
        assert!(!is_dangerous_executable(Path::new("report.pdf")));
    }

    #[test]
    fn txt_is_safe() {
        assert!(!is_dangerous_executable(Path::new("notes.txt")));
    }

    #[test]
    fn png_is_safe() {
        assert!(!is_dangerous_executable(Path::new("photo.png")));
    }

    #[test]
    fn no_extension_is_safe() {
        assert!(!is_dangerous_executable(Path::new("Makefile")));
    }

    // ── is_contained ─────────────────────────────────────────────────────────

    #[test]
    fn file_inside_root_is_contained() {
        // Build a real temp dir so canonicalize works.
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let file_path = root.join("subdir").join("file.txt");
        std::fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        std::fs::write(&file_path, b"").unwrap();

        assert!(is_contained(root, &file_path));
    }

    #[test]
    fn file_outside_root_is_not_contained() {
        let tmp_root = tempfile::tempdir().expect("root tempdir");
        let tmp_outside = tempfile::tempdir().expect("outside tempdir");

        // Put a real file in the "outside" dir.
        let outside_file = tmp_outside.path().join("other.txt");
        std::fs::write(&outside_file, b"").unwrap();

        assert!(!is_contained(tmp_root.path(), &outside_file));
    }

    #[test]
    fn sibling_dir_is_not_contained() {
        // root = /tmp/abc, target = /tmp/abcEvil — starts_with must not match
        // on the string level but only on path-component boundaries.
        let tmp = tempfile::tempdir().expect("tempdir");
        let parent = tmp.path();

        let root = parent.join("root");
        let sibling = parent.join("rootEvil");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();

        // sibling exists; is_contained must return false.
        assert!(!is_contained(&root, &sibling));
    }
