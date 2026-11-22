use super::crash::*;

#[test]
fn record_has_no_name_path_or_message() {
    let t = render("0.3.1", Some("sync-Alice's-laptop"), r"C:\Users\alice\code\src\shell\run.rs", 42, "2026-11-22T10:00:00Z");
    assert_eq!(t, "2026-11-22T10:00:00Z CRASH version=0.3.1 thread=other location=run.rs:42\n");
    assert!(!t.contains("alice") && !t.contains("Alice") && !t.contains("Users"));
}

#[test]
fn odd_file_names_become_unknown_and_main_is_kept() {
    let t = render("1.0", Some("main"), "/home/bob/secret passwords.rs", 1, "t");
    assert!(t.contains("thread=main") && t.contains("location=unknown:1"));
}

#[test]
fn write_read_clear_round_trip() {
    let dir = std::env::temp_dir().join(format!("bb-crash-{}", std::process::id()));
    assert!(take_pending(&dir).is_none());
    write(&dir, "x CRASH\n");
    let r = take_pending(&dir).expect("record");
    assert_eq!(r.text, "x CRASH\n");
    assert!(take_pending(&dir).is_some(), "stays until cleared");
    clear(&dir);
    assert!(take_pending(&dir).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}
