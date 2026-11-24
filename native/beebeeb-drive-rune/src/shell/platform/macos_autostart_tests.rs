//! Fakes only: nothing is registered and no LaunchAgents folder is read.

use std::cell::Cell;

use super::*;

struct Fake {
    status: Cell<Status>,
    fail: bool,
    calls: Cell<u32>,
}

fn fake(status: Status, fail: bool) -> Fake {
    Fake {
        status: Cell::new(status),
        fail,
        calls: Cell::new(0),
    }
}

impl LoginItems for Fake {
    fn status(&self) -> Status {
        self.status.get()
    }
    fn register(&self) -> Result<(), ()> {
        self.calls.set(self.calls.get() + 1);
        if self.fail {
            Err(())
        } else {
            self.status.set(Status::Enabled);
            Ok(())
        }
    }
    fn unregister(&self) -> Result<(), ()> {
        self.calls.set(self.calls.get() + 1);
        if self.fail {
            Err(())
        } else {
            self.status.set(Status::NotRegistered);
            Ok(())
        }
    }
}

#[test]
fn status_maps_to_on_or_off() {
    for (raw, on) in [(0, false), (1, true), (2, true), (3, false), (9, false), (-1, false)] {
        assert_eq!(Status::from_raw(raw).is_on(), on, "raw {raw}");
    }
}

#[test]
fn turning_on_and_off_registers_and_unregisters() {
    let items = fake(Status::NotRegistered, false);
    set_with(&items, true).unwrap();
    assert!(enabled_with(&items));
    set_with(&items, false).unwrap();
    assert!(!enabled_with(&items));
    assert_eq!(items.calls.get(), 2);
}

#[test]
fn asking_for_what_is_so_does_nothing() {
    let items = fake(Status::Enabled, true);
    set_with(&items, true).unwrap();
    assert_eq!(items.calls.get(), 0);
}

#[test]
fn a_refused_register_answers_os_with_fixed_words() {
    let items = fake(Status::NotRegistered, true);
    match set_with(&items, true) {
        Err(PlatformError::Os(words)) => assert_eq!(words, REGISTER_FAILED),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_legacy_agent_is_found_in_the_given_folder_only() {
    let dir = std::env::temp_dir().join(format!("bb-dm05-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    assert!(legacy_agent_in(&dir).is_none());
    std::fs::write(dir.join("Beebeeb.plist"), "x").unwrap();
    assert!(legacy_agent_in(&dir).is_some());
    std::fs::remove_dir_all(&dir).unwrap();
}
