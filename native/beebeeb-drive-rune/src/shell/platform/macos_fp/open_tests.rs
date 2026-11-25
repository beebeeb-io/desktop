//! Tests for `open.rs`, ported from the tag's `finder_open.rs` and `macos_workspace.rs`, over a fake workspace: no test
//! asks the real `NSWorkspace` or the File Provider.

use std::cell::RefCell;

use beebeeb_engine::finder_setup::error::{bridge_code, FpError, BRIDGE_DOMAIN};
use beebeeb_engine::state_db::Namespace;

use super::super::bridge::{Gate, GATE_TESTS};
use super::*;

/// What the fake saw, in order; `open`/`reveal` also say whether the bridge gate was free at that moment.
#[derive(Default)]
struct Fake {
    calls: RefCell<Vec<String>>,
    location: Option<bool>,
    item: Option<bool>,
    fail_open: Option<FpError>,
}

impl Fake {
    fn with_location() -> Self {
        Fake {
            location: Some(true),
            item: Some(true),
            ..Fake::default()
        }
    }
    fn gate_free() -> bool {
        Gate::try_enter().is_ok()
    }
}

impl Workspace for Fake {
    type Url = &'static str;
    fn resolve_location(&self) -> Result<Option<&'static str>, FpError> {
        self.calls
            .borrow_mut()
            .push(format!("resolve_location gate_free={}", Fake::gate_free()));
        Ok(self.location.filter(|has| *has).map(|_| "location"))
    }
    fn resolve_item(&self, item_id: &str) -> Result<Option<&'static str>, FpError> {
        self.calls
            .borrow_mut()
            .push(format!("resolve_item {item_id} gate_free={}", Fake::gate_free()));
        Ok(self.item.filter(|has| *has).map(|_| "item"))
    }
    fn open(&self, _url: &'static str) -> Result<(), FpError> {
        self.calls
            .borrow_mut()
            .push(format!("open gate_free={}", Fake::gate_free()));
        self.fail_open.clone().map_or(Ok(()), Err)
    }
    fn reveal(&self, _url: &'static str) -> Result<(), FpError> {
        self.calls
            .borrow_mut()
            .push(format!("reveal gate_free={}", Fake::gate_free()));
        Ok(())
    }
}

#[test]
fn the_gate_is_held_for_the_resolve_and_free_for_the_open() {
    let _one = GATE_TESTS.lock().unwrap_or_else(|p| p.into_inner());
    let fake = Fake::with_location();
    open_location_with(&fake).unwrap();
    assert_eq!(
        *fake.calls.borrow(),
        ["resolve_location gate_free=false", "open gate_free=true"]
    );
    let fake = Fake::with_location();
    reveal_item_with(&fake, "f1").unwrap();
    assert_eq!(
        *fake.calls.borrow(),
        ["resolve_item f1 gate_free=false", "reveal gate_free=true"]
    );
}

#[test]
fn no_location_is_an_error_with_its_own_sentence_never_a_success() {
    let _one = GATE_TESTS.lock().unwrap_or_else(|p| p.into_inner());
    let none = Fake::default();
    assert_eq!(open_location_with(&none), Err(OpenFailure::NoLocation));
    assert_eq!(reveal_item_with(&none, "f1"), Err(OpenFailure::NoLocation));
    assert_eq!(none.calls.borrow().len(), 2, "nothing was opened or revealed");
    assert_eq!(OpenFailure::NoLocation.sentence(true), NOTHING_TO_OPEN);
    assert_eq!(OpenFailure::NoLocation.sentence(false), NOTHING_TO_SHOW);
    assert_ne!(NOTHING_TO_OPEN, NOTHING_TO_SHOW);
}

#[test]
fn a_bridge_failure_reaches_the_person_as_domain_and_code_only() {
    let _one = GATE_TESTS.lock().unwrap_or_else(|p| p.into_inner());
    let mut fake = Fake::with_location();
    fake.fail_open = Some(FpError::new(
        "NSCocoaErrorDomain",
        256,
        "couldn’t open /Users/jo/Library/CloudStorage/Beebeeb-Beebeeb",
    ));
    let Err(failure) = open_location_with(&fake) else {
        panic!("an open that failed is not Ok")
    };
    let sentence = failure.sentence(true);
    for leak in ["Users", "CloudStorage", "couldn", "Beebeeb-Beebeeb"] {
        assert!(!sentence.contains(leak), "{sentence}");
    }
    assert!(
        sentence.contains("NSCocoaErrorDomain") && sentence.contains("256"),
        "{sentence}"
    );
    let timeout = FpError::new(
        BRIDGE_DOMAIN,
        bridge_code::OPEN_TIMEOUT,
        "Timed out waiting for LaunchServices",
    );
    assert!(!OpenFailure::Bridge(timeout).sentence(true).contains("Timed out"));
}

#[test]
fn a_busy_gate_is_an_error_not_an_open_and_not_a_hang() {
    let _one = GATE_TESTS.lock().unwrap_or_else(|p| p.into_inner());
    let held = Gate::try_enter().unwrap();
    let fake = Fake::with_location();
    let started = std::time::Instant::now();
    assert!(matches!(open_location_with(&fake), Err(OpenFailure::Bridge(_))));
    assert!(started.elapsed() < std::time::Duration::from_secs(6));
    assert!(
        fake.calls.borrow().is_empty(),
        "no resolve and no open behind a busy gate"
    );
    drop(held);
}

#[test]
fn a_shared_with_me_row_is_refused_before_any_lookup_or_bridge_call() {
    // The pseudo folder: refused without reading a row.
    let refused = may_show("namespace:shared_with_me", |_| {
        panic!("no lookup for the pseudo folder")
    });
    assert_eq!(refused, Err(OpenFailure::Shared));
    // A shared row, and the sentence says the web app.
    assert_eq!(
        may_show("s1", |_| Ok(Some(Namespace::SharedWithMe))),
        Err(OpenFailure::Shared)
    );
    assert_eq!(
        OpenFailure::Shared.sentence(false),
        "Files shared with you are not in Finder; they open in the web app."
    );
    // Own namespaces pass; an unknown id and a failing lookup do not.
    for own in [Namespace::MyFiles, Namespace::Offline, Namespace::Conflicts] {
        assert_eq!(may_show("f1", |_| Ok(Some(own))), Ok(()));
    }
    assert_eq!(may_show("nope", |_| Ok(None)), Err(OpenFailure::NotKnown));
    assert_eq!(
        may_show("f1", |_| Err("read item: boom".into())),
        Err(OpenFailure::NotKnown)
    );
}

/// The URL stays a URL and the open is NSWorkspace's: no `Command::new("open")` anywhere in the crate (a child
/// inherits the sandbox but not the extension on the URL; LaunchServices refused with -54 while the spawn said Ok).
#[test]
fn no_code_in_the_crate_spawns_the_open_command() {
    fn walk(dir: &std::path::Path, hits: &mut Vec<String>, files: &mut usize) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, hits, files);
            } else if path.extension().is_some_and(|e| e == "rs") {
                *files += 1;
                let q = '"';
                let needles = [
                    format!("Command::new({q}open{q})"),
                    format!("Command::new({q}/usr/bin/open{q})"),
                ];
                for (n, line) in std::fs::read_to_string(&path).unwrap().lines().enumerate() {
                    let code = line.trim_start();
                    if !code.starts_with("//") && needles.iter().any(|needle| code.contains(needle.as_str())) {
                        hits.push(format!("{}:{}", path.display(), n + 1));
                    }
                }
            }
        }
    }
    let (mut hits, mut files) = (Vec::new(), 0);
    walk(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut hits,
        &mut files,
    );
    assert!(files > 50, "the scan read only {files} files");
    assert!(hits.is_empty(), "a child `open` came back: {hits:?}");
}

#[test]
fn a_finder_that_is_not_set_up_answers_with_a_line() {
    use beebeeb_engine::finder_setup::driver::FinderSetupView;
    use beebeeb_engine::finder_setup::launch_location::LaunchLocation;
    let view = FinderSetupView::initial(LaunchLocation::Applications);
    assert!(not_ready_line(&view).is_some());
}

#[test]
fn a_plain_file_opens_with_its_default_app_and_a_folder_is_revealed() {
    let _one = GATE_TESTS.lock().unwrap_or_else(|p| p.into_inner());
    let fake = Fake::with_location();
    open_item_with(&fake, "f1").unwrap();
    assert_eq!(
        *fake.calls.borrow(),
        ["resolve_item f1 gate_free=false", "open gate_free=true"]
    );

    let (file, folder) = (
        std::path::Path::new("/t/dm38/plain.txt"),
        std::path::Path::new("/t/dm38/folder"),
    );
    register_open_item(file, "f1");
    register_item(folder, "d1");
    let fake = Fake::with_location();
    start_registered_with(&fake, file).unwrap();
    start_registered_with(&fake, folder).unwrap();
    assert_eq!(
        *fake.calls.borrow(),
        [
            "resolve_item f1 gate_free=false",
            "open gate_free=true",
            "resolve_item d1 gate_free=false",
            "reveal gate_free=true"
        ]
    );
    assert_eq!(
        start_registered_with(&fake, std::path::Path::new("/t/dm38/nobody")),
        Err(OpenFailure::NoLocation)
    );
}

#[test]
fn the_login_items_address_is_a_system_settings_pane() {
    assert!(LOGIN_ITEMS_URL.starts_with("x-apple.systempreferences:"));
}
