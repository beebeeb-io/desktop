//! The Mac web opener: it asks `NSWorkspace` (the seam) for each allowed address and refuses the rest; and every
//! external-link row of Settings reaches the opener with its https address on beebeeb.io.

use std::cell::RefCell;

use super::*;

struct Fake {
    asked: RefCell<Vec<String>>,
    accept: bool,
}

impl Workspace for Fake {
    fn open(&self, url: &str) -> bool {
        self.asked.borrow_mut().push(url.to_string());
        self.accept
    }
}

#[test]
fn an_https_address_goes_to_the_workspace() {
    let w = Fake {
        asked: RefCell::new(vec![]),
        accept: true,
    };
    assert!(open_with(&w, "https://beebeeb.io/legal/").is_ok());
    assert_eq!(*w.asked.borrow(), vec!["https://beebeeb.io/legal/"]);
}

#[test]
fn http_file_javascript_and_empty_are_refused_without_asking() {
    let w = Fake {
        asked: RefCell::new(vec![]),
        accept: true,
    };
    for bad in [
        "http://beebeeb.io",
        "file:///etc/passwd",
        "javascript:1",
        "/etc/hosts",
        "",
        "https://",
        "https:///x",
    ] {
        assert!(open_with(&w, bad).is_err(), "{bad}");
    }
    assert!(w.asked.borrow().is_empty());
}

#[test]
fn a_refusal_by_the_system_is_an_error() {
    let w = Fake {
        asked: RefCell::new(vec![]),
        accept: false,
    };
    assert!(open_with(&w, "https://beebeeb.io").is_err());
}

#[test]
fn the_real_opener_is_nsworkspace_open_url() {
    let src = include_str!("macos_open_url.rs");
    assert!(src.contains("NSWorkspace::sharedWorkspace().openURL("));
    for banned in ["std::process", "Command::new", "osascript"] {
        assert!(
            !src.lines()
                .filter(|l| !l.trim_start().starts_with("//"))
                .any(|l| l.contains(banned)),
            "{banned}"
        );
    }
}

#[test]
fn every_external_link_row_in_settings_opens_an_https_beebeeb_address() {
    // Rows with the external-link icon: Privacy and Export my data (`Intent::Open`), Terms and privacy policy and
    // "Plans are on beebeeb.io" (`Intent::Web`: "plan"), Help and report a problem (a mail draft, not a web row).
    for name in [
        "privacy",
        "export-my-data",
        "terms-and-privacy-policy",
        "beebeeb.io",
        "plan",
    ] {
        let url = crate::shell::web_pages::url(name).unwrap_or_else(|| panic!("{name} has no address"));
        assert!(is_openable(url), "{name}");
        let host = url
            .strip_prefix("https://")
            .unwrap_or_else(|| panic!("{name}: not https"))
            .split('/')
            .next()
            .unwrap();
        assert!(host == "beebeeb.io" || host == "app.beebeeb.io", "{name}: {host}");
        let w = Fake {
            asked: RefCell::new(vec![]),
            accept: true,
        };
        assert!(open_with(&w, url).is_ok());
        assert_eq!(*w.asked.borrow(), vec![url]);
    }
}
