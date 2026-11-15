//! "Sign in with another device": the endings the board has no frame for (CP2 row 3.4). A request lost after its code
//! was shown is said as lost, not as a code that expired; an approval this PC did not take is said for what it is, not
//! as keys that did not arrive. Every name and code is the board's or invented.

use rune::Runtime;

use super::source::{DeviceStatus, FixtureSignInSource, Script, SignInEvent};
use super::tests::{info, runtime_of, ui};
use super::{Screen, Ui};
use crate::State;

/// A request whose code is on the screen (DW-A07), on the fixture.
fn waiting() -> Runtime<State> {
    let mut ui_ = Ui::new(Box::new(FixtureSignInSource::new(Script::default())));
    ui_.ask_other_device();
    let mut rt = runtime_of(ui_, false);
    assert_eq!(ui(&mut rt).screen, Screen::Device { expired: false });
    assert!(ui(&mut rt).request().is_some(), "the code is shown");
    rt
}

/// The engine's ending of the request on the screen, as `poll` hands it over. Through `Runtime::update`, so the window
/// is drawn again from the new state.
fn ended(rt: &mut Runtime<State>, status: DeviceStatus) {
    rt.update(|s, _| {
        s.signin
            .as_mut()
            .expect("the sign-in window is open")
            .apply(SignInEvent::Device(status));
    });
    rt.frame(0.0);
}

fn banner(rt: &Runtime<State>) -> (Option<String>, Option<String>) {
    (info(rt, "banner-0-title").text, info(rt, "banner-0-text").text)
}

fn tree(rt: &Runtime<State>) -> String {
    rt.accessibility_json().to_string()
}

/// Mutant: `device` reads `ui.lost()` as `false` (every spent code then says "This code expired").
#[test]
fn a_request_lost_after_its_code_is_said_as_lost_and_a_new_code_is_offered() {
    let mut rt = waiting();
    ended(&mut rt, DeviceStatus::Lost);
    assert_eq!(ui(&mut rt).screen, Screen::Device { expired: true });
    assert_eq!(
        banner(&rt),
        (
            Some("This request was lost".to_string()),
            Some("Nothing was signed in. Ask for a new code.".to_string())
        )
    );
    assert!(!tree(&rt).contains("This code expired"), "the code did not expire");

    // "Get a new code", and this time the code runs out: DW-A10, word for word.
    ui(&mut rt).ask_other_device();
    rt.frame(0.0);
    assert_eq!(ui(&mut rt).screen, Screen::Device { expired: false });
    ended(&mut rt, DeviceStatus::Expired);
    assert_eq!(
        banner(&rt),
        (
            Some("This code expired".to_string()),
            Some("Nothing was signed in. Ask for a new code.".to_string())
        )
    );
    assert!(!tree(&rt).contains("This request was lost"));
}

/// Mutant: `apply` sends `DeviceStatus::OtherAccount` to `Screen::KeysNotReceived` (the keys DID arrive).
#[test]
fn an_approval_this_pc_did_not_take_is_said_for_what_it_is() {
    for (status, title, words, other_account) in [
        (
            DeviceStatus::OtherAccount,
            "Approved, but this PC holds another account",
            "Nothing was signed in on this PC, and no keys were kept.",
            true,
        ),
        (
            DeviceStatus::NotSignedIn,
            "Approved, but this PC did not sign in",
            "Nothing was signed in. Ask again.",
            false,
        ),
    ] {
        let mut rt = waiting();
        ended(&mut rt, status.clone());
        assert_eq!(ui(&mut rt).screen, Screen::NotSignedIn { other_account }, "{status:?}");
        assert_eq!(info(&rt, "banner-0").role, "alert", "{status:?}");
        assert_eq!(
            banner(&rt),
            (Some(title.to_string()), Some(words.to_string())),
            "{status:?}"
        );
        assert!(!tree(&rt).contains("keys did not arrive"), "{status:?}: they did");
        assert!(
            ui(&mut rt).take_intents().is_empty(),
            "{status:?}: nothing was signed in"
        );
        assert_eq!(info(&rt, "btn-0-label").text.as_deref(), Some("Ask again"));
        ui(&mut rt).ask_other_device();
        assert_eq!(
            ui(&mut rt).screen,
            Screen::Device { expired: false },
            "{status:?}: Ask again"
        );
    }
}
