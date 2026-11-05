    use super::*;

    #[test]
    fn open_folder_rejects_missing_root_without_suggestion() {
        assert_eq!(
            require_open_folder_root(None),
            Err("Not configured on this PC yet".to_string())
        );
    }

    #[test]
    fn open_folder_rejects_empty_root() {
        for root in ["", "   "] {
            assert_eq!(
                require_open_folder_root(Some(root.to_string())),
                Err("Not configured on this PC yet".to_string())
            );
        }
    }

    #[test]
    fn open_folder_preserves_explicit_root() {
        let root = "D:\\Private files\\資料\\Sync";
        assert_eq!(
            require_open_folder_root(Some(root.to_string())),
            Ok(PathBuf::from(root))
        );
    }
