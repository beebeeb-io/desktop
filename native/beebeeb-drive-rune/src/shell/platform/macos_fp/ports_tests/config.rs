use super::super::*;
use beebeeb_engine::surfaces::phase::FinderFailureReason;

fn record() -> FailureRecord {
    FailureRecord {
        reason: FinderFailureReason::FolderTaken,
        domain: "NSCocoaErrorDomain".into(),
        code: 516,
        at: 1_791_291_909,
    }
}

#[test]
fn an_unreadable_config_never_counts_as_signed_out_by_choice() {
    let unreadable = facts_from(false, false, false, Err("could not read".to_string()));
    assert_eq!(unreadable, SessionFacts::default());
    let cfg = DesktopConfig {
        finder_signed_out_by_choice: true,
        ..DesktopConfig::default()
    };
    assert!(facts_from(false, false, false, Ok(cfg)).signed_out_by_choice);
    let readable = facts_from(true, true, false, Ok(DesktopConfig::default()));
    assert_eq!(
        readable,
        SessionFacts {
            vault_unlocked: true,
            auth_present: true,
            signed_out_by_choice: false,
            removal_owed: false
        }
    );
}

#[test]
fn the_config_is_written_only_when_a_value_changes() {
    let mut cfg = DesktopConfig::default();
    assert!(!set_failure(&mut cfg, None));
    assert!(set_failure(&mut cfg, Some(record())));
    assert_eq!(cfg.finder_last_failure, Some(record()));
    assert!(!set_failure(&mut cfg, Some(record())));
    assert!(set_failure(&mut cfg, None));
    assert!(!set_signed_out_by_choice(&mut cfg, false));
    assert!(set_signed_out_by_choice(&mut cfg, true));
    assert!(!set_signed_out_by_choice(&mut cfg, true));
    assert!(set_signed_out_by_choice(&mut cfg, false));
}

#[test]
fn a_kept_folder_saved_by_another_writer_survives_a_failure_record_write() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("desktop.toml");
    DesktopConfig::update_at(&path, |cfg| {
        cfg.kept_unsynced_folder = Some("/kept".into());
        (true, ())
    })
    .unwrap();
    update_config_at(&path, |cfg| set_failure(cfg, Some(record())));
    update_config_at(&path, |cfg| set_signed_out_by_choice(cfg, true));
    let saved = DesktopConfig::load_from(&path).unwrap();
    assert_eq!(saved.kept_unsynced_folder.as_deref(), Some("/kept"));
    assert_eq!(saved.finder_last_failure, Some(record()));
    assert!(saved.finder_signed_out_by_choice);
}

#[test]
fn an_unchanged_value_leaves_the_file_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("desktop.toml");
    update_config_at(&path, |cfg| set_failure(cfg, None));
    assert!(!path.exists());
}
