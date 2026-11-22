//! Tests of the Windows toast (`shell::platform::windows_toast`): it honours the Settings switches it reads, and each
//! notice says what its kind says.

use super::*;

fn all(on: bool) -> Switches {
    Switches {
        allow: on,
        sync_finished: on,
        needs_decision: on,
        local_space: on,
        backup_attention: on,
    }
}

/// One notice of each kind, carrying the kind of text a toast must never show.
fn notices() -> Vec<Notice> {
    vec![
        Notice::SyncComplete,
        Notice::QuotaWarning {
            usage_bytes: 9_000_000_000,
            limit_bytes: 10_000_000_000,
        },
        Notice::Conflict {
            file_name: "tax-return-2025.pdf".into(),
        },
        Notice::BackupRefusal {
            message: "C:\\Users\\anna\\Documents\\Private\\tax-return-2025.pdf was not deleted".into(),
        },
        Notice::ServiceFolderRefused,
    ]
}

#[test]
fn toast_honours_the_switch() {
    // Every switch off: no toast of any notice.
    for notice in notices() {
        assert!(
            !wanted(kind_of(&notice), &all(false)),
            "{notice:?} shown with every switch off"
        );
    }
    // "Allow notifications" off beats a row that is on.
    let mut only_rows = all(true);
    only_rows.allow = false;
    for notice in notices() {
        assert!(!wanted(kind_of(&notice), &only_rows), "{notice:?} shown with Allow off");
    }
    // Allow on and one row off: only that row's notice is silent.
    let mut no_sync = all(true);
    no_sync.sync_finished = false;
    assert!(!wanted(kind_of(&Notice::SyncComplete), &no_sync));
    assert!(wanted(kind_of(&Notice::ServiceFolderRefused), &no_sync));
    let mut no_decision = all(true);
    no_decision.needs_decision = false;
    assert!(!wanted(Kind::NeedsDecision, &no_decision));
    assert!(wanted(Kind::SyncFinished, &no_decision));
    let mut no_space = all(true);
    no_space.local_space = false;
    assert!(!wanted(Kind::LocalSpace, &no_space));
    let mut no_backup = all(true);
    no_backup.backup_attention = false;
    assert!(!wanted(Kind::BackupAttention, &no_backup));
    assert!(wanted(Kind::LocalSpace, &no_backup));
    // Everything on: every notice is shown.
    for notice in notices() {
        assert!(
            wanted(kind_of(&notice), &all(true)),
            "{notice:?} not shown with everything on"
        );
    }
}

#[test]
fn switches_are_read_from_the_settings_the_pane_writes() {
    let config = DesktopConfig {
        notify_allow: true,
        notify_sync_complete: true,
        notify_needs_decision: false,
        notify_local_space: true,
        notify_backup_attention: false,
        ..DesktopConfig::default()
    };
    let switches = Switches::from_config(&config);
    assert!(switches.allow && switches.sync_finished && switches.local_space);
    assert!(!switches.needs_decision && !switches.backup_attention);
}

#[test]
fn security_toast_always_shown() {
    assert!(wanted(Kind::Security, &all(false)));
    let mut allow_off = all(true);
    allow_off.allow = false;
    assert!(wanted(Kind::Security, &allow_off));
}

#[test]
fn toast_text_names_no_path() {
    for notice in notices() {
        let xml = toast_xml(&words_of(&notice));
        for forbidden in [
            "C:\\",
            "\\Users",
            "anna",
            "Documents",
            "Private",
            "tax-return",
            ".pdf",
            "@",
            "9000000000",
        ] {
            assert!(
                !xml.contains(forbidden),
                "{notice:?}: the toast says {forbidden:?}: {xml}"
            );
        }
    }
}

#[test]
fn toast_xml_escapes_its_words() {
    let xml = toast_xml(&Words {
        title: "a < b & \"c\"",
        body: "it's > d",
    });
    assert!(xml.contains("a &lt; b &amp; &quot;c&quot;"), "{xml}");
    assert!(xml.contains("it&apos;s &gt; d"), "{xml}");
    assert_eq!(xml.matches("<text>").count(), 2);
}

#[test]
fn the_service_folder_toast_says_the_boards_words() {
    let words = words_of(&Notice::ServiceFolderRefused);
    assert_eq!(words.title, "Beebeeb manages this folder");
    assert_eq!(
        words.body,
        "Backups and the folders Beebeeb services keep can't be moved, renamed or deleted. You can open and download what's inside."
    );
}

#[test]
fn a_refused_backups_move_says_the_boards_dw1182_words() {
    let words = words_of(&Notice::BackupRefusal {
        message: "C:\\Users\\anna\\Backups was not deleted".into(),
    });
    assert_eq!(words.title, "Beebeeb manages this folder");
    assert_eq!(
        words.body,
        "Backups and the folders Beebeeb services keep can't be moved, renamed or deleted. You can open and download what's inside."
    );
}

#[test]
fn a_refused_backups_move_ignores_the_backup_attention_switch() {
    let refusal = Notice::BackupRefusal {
        message: "C:\\Users\\anna\\Backups was not deleted".into(),
    };
    let mut no_backup = all(true);
    no_backup.backup_attention = false;
    assert!(wanted(kind_of(&refusal), &no_backup));
    let mut no_allow = no_backup;
    no_allow.allow = false;
    assert!(!wanted(kind_of(&refusal), &no_allow));
}

#[test]
fn aumid_differs_for_the_test_build() {
    let (real, test) = (aumid(AppIdentity::Real), aumid(AppIdentity::Test));
    assert_ne!(real, test);
    assert_eq!(real, "io.beebeeb.app");
    assert_eq!(test, "io.beebeeb.app.test");
    assert_eq!(aumid(AppIdentity::CURRENT), AppIdentity::CURRENT.bundle_identifier());
}

/// Ported from the 0.8.x `conflict_notification_gate_fails_open_when_config_unreadable`: a settings file that cannot be
/// read means the defaults, and the defaults show a "needs a decision" toast (a lost decision is worse than one toast
/// too many), and its Security toast too. `show` reads the file with `unwrap_or_default()`.
#[test]
fn toast_fails_open_when_the_settings_cannot_be_read() {
    // An unreadable config is read as the defaults (`show`'s fallback, pinned below).
    let switches = Switches::from_config(&DesktopConfig::default());
    assert!(
        wanted(Kind::NeedsDecision, &switches),
        "a conflict toast is shown when the settings are unreadable"
    );
    assert!(wanted(Kind::LocalSpace, &switches));
    assert!(wanted(Kind::Security, &switches));
}

/// The line above only holds while `show` falls back to the defaults. Pinned on the production text (the part above
/// the test module), because a read of the real settings file is not hermetic.
#[test]
fn toast_fails_open_show_falls_back_to_the_defaults() {
    let source = include_str!("windows_toast.rs");
    let production = source.split("#[cfg(test)]").next().unwrap();
    let show = production.split("pub(crate) fn show(").nth(1).expect("show exists");
    let body = show.split("\n}\n").next().unwrap();
    assert!(
        body.contains("DesktopConfig::load_for(crate::shell::identity::engine_identity()).unwrap_or_default()"),
        "show reads the settings and falls back to the defaults"
    );
}
