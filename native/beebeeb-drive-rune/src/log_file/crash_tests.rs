use super::crash::*;

#[test]
fn record_has_no_name_path_or_message() {
    let t = render(
        "0.3.1",
        Some("sync-Alice's-laptop"),
        r"C:\Users\alice\code\src\shell\run.rs",
        42,
        "2026-11-22T10:00:00Z",
    );
    assert_eq!(
        t,
        "2026-11-22T10:00:00Z CRASH version=0.3.1 thread=other location=run.rs:42\n"
    );
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
    let rec = "2026-11-22T10:00:00Z CRASH version=0.9.0 thread=main location=run.rs:1\n";
    write(&dir, rec);
    let r = take_pending(&dir).expect("record");
    assert_eq!(r.text, rec);
    assert!(take_pending(&dir).is_some(), "stays until cleared");
    clear(&dir);
    assert!(take_pending(&dir).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

const SECRET_PATH: &str = r"C:\Users\alice\Documents\Tax 2026\aangifte.pdf";
const SECRET_EMAIL: &str = "alice.jansen@example.com";

fn holds_nothing_of_the_person(t: &str) {
    for secret in [
        SECRET_PATH,
        SECRET_EMAIL,
        "alice",
        "Users",
        "Documents",
        "aangifte",
        "example.com",
        "@",
        "token",
        "\\",
    ] {
        assert!(!t.contains(secret), "the record holds {secret:?}: {t}");
    }
}

thread_local! {
    static CAUGHT: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    static CATCHING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// A real panic whose message names a path, an email and a token: the record made of it (by the hook's own
/// `record_for`) holds none of them, and neither does what the reader hands to a notice or a report.
#[test]
fn a_crash_record_from_a_panic_naming_a_path_and_an_email_holds_neither() {
    static HOOK: std::sync::Once = std::sync::Once::new();
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if CATCHING.with(std::cell::Cell::get) {
                CAUGHT.with(|c| *c.borrow_mut() = Some(record_for(info, "2026-11-22T10:00:00Z")));
            } else {
                previous(info);
            }
        }));
    });
    CATCHING.with(|c| c.set(true));
    let result = std::panic::catch_unwind(|| {
        panic!("could not open {SECRET_PATH} for {SECRET_EMAIL} (token=eyJhbGciOi.secret)");
    });
    CATCHING.with(|c| c.set(false));
    assert!(result.is_err());
    let record = CAUGHT.with(|c| c.borrow_mut().take()).expect("the hook made a record");
    assert!(
        record.contains(" CRASH ") && record.contains("location=crash_tests.rs:"),
        "{record}"
    );
    holds_nothing_of_the_person(&record);

    let dir = std::env::temp_dir().join(format!("bb-crash-panic-{}", std::process::id()));
    write(&dir, &record);
    let read = take_pending(&dir).expect("the record reads back");
    assert_eq!(read.text, record);
    holds_nothing_of_the_person(&read.text);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The reader trusts nothing in the file: a line that is not of the record's own shape (a message, a path, an
/// email, a forged field) is dropped; a file with nothing else is no record at all.
#[test]
fn the_crash_reader_drops_every_line_not_of_the_records_shape() {
    let dir = std::env::temp_dir().join(format!("bb-crash-reader-{}", std::process::id()));
    let good = "2026-11-22T10:00:00Z CRASH version=0.9.0 thread=main location=run.rs:42\n";
    let forged = [
        format!("{SECRET_PATH}\n"),
        format!("panicked at 'open {SECRET_PATH}' for {SECRET_EMAIL}\n"),
        format!("2026-11-22T10:00:00Z CRASH version=0.9.0 thread={SECRET_EMAIL} location=run.rs:42\n"),
        "2026-11-22T10:00:00Z CRASH version=0.9.0 thread=main location=C:\\Users\\alice\\run.rs:42\n".to_string(),
        format!("2026-11-22T10:00:00Z CRASH version=0.9.0 thread=main location=run.rs:42 {SECRET_EMAIL}\n"),
    ];
    write(&dir, &forged.concat());
    assert!(take_pending(&dir).is_none(), "nothing of the record's shape: no record");
    write(&dir, &format!("{}{good}", forged.concat()));
    let read = take_pending(&dir).expect("record");
    assert_eq!(read.text, good);
    holds_nothing_of_the_person(&read.text);
    read.clear();
    assert!(take_pending(&dir).is_none(), "cleared");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Send a report: the record goes into the saved support bundle as `crash_record`, the bundle's own fields kept.
#[test]
fn the_crash_record_is_attached_to_the_support_bundle() {
    let dir = std::env::temp_dir().join(format!("bb-crash-bundle-{}", std::process::id()));
    let good = "2026-11-22T10:00:00Z CRASH version=0.9.0 thread=other location=main.rs:80\n";
    write(&dir, good);
    let record = take_pending(&dir).expect("record");
    let bundle = dir.join("beebeeb-support-bundle-1.json");
    std::fs::write(&bundle, r#"{"queue":{"pending":2}}"#).unwrap();
    assert!(attach_to_bundle(&bundle, &record));
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&bundle).unwrap()).unwrap();
    assert_eq!(v["crash_record"], good);
    assert_eq!(v["queue"]["pending"], 2);
    std::fs::write(&bundle, "not json").unwrap();
    assert!(
        !attach_to_bundle(&bundle, &record),
        "a bundle that is not the engine's object is left alone"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
