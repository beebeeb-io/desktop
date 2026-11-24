//! Create file (DW-1522), from what the New file dialog leaves for the shell to the Files source: made in the folder
//! asked, a refusal said with Try again (which opens the dialog again), and nothing made when the session gate
//! closed between the press and the poll, not even once it opens again.

use std::sync::Arc;

use rune::{Env, Runtime};

use super::super::source::{Action, ActionError, Call, FilesFixture};
use super::super::tests::{Calls, Fails};
use super::super::{Actions, Toast};
use crate::frames::viewers::Intent as ViewerIntent;
use crate::shell::actions::FixtureEngine;
use crate::shell::app::GateState;
use crate::shell::host::{self, Shell};
use crate::shell::platform::fake::FakePlatform;
use crate::shell::state;
use crate::State;

/// The board's Files window on an engine the test holds, the shell's keys and poll, its file actions recorded.
fn app() -> (Runtime<State>, Arc<FixtureEngine>, Calls, Fails) {
    let engine = Arc::new(FixtureEngine::ready());
    let mut state = state::init_with(Shell::fixture_on(engine.clone(), Arc::new(FakePlatform::default())));
    let fixture = FilesFixture::default();
    let (calls, fails) = (fixture.calls.clone(), fixture.fail.clone());
    state.actions = Actions::new(Box::new(fixture));
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, crate::view, (1278.0, 798.0), env);
    rt.on_key = Some(host::on_key);
    rt.poll = Some(host::poll);
    rt.frame(0.0);
    (rt, engine, calls, fails)
}

/// What the dialog's Create file leaves for the shell (`new_file::create`), with no frame after it. Returns the
/// folder it names: the one shown.
fn asked(rt: &mut Runtime<State>, name: &str) -> Option<String> {
    let folder = rt.state.listing.folder.id.clone();
    rt.state.viewer.asked.push(ViewerIntent::CreateFile {
        folder: folder.clone(),
        name: name.into(),
    });
    folder
}

fn calls_of(calls: &Calls) -> Vec<Call> {
    calls.lock().unwrap().clone()
}

/// Mutant for the checkpoint: drop the `create_file::poll` line from `shell/host.rs` -> red (no call is made).
#[test]
fn create_file_is_made_by_the_source_in_the_folder_asked() {
    let (mut rt, _engine, calls, _) = app();
    let folder = asked(&mut rt, "Plans.md");
    rt.frame(0.0);
    assert_eq!(calls_of(&calls), [Call::NewFile(folder, "Plans.md".into())]);
    assert!(rt.state.viewer.asked.is_empty(), "nothing is left for a later poll");
    assert_eq!(rt.state.live_toast(), None, "nothing failed");
    rt.frame(0.0);
    assert_eq!(calls_of(&calls).len(), 1, "made once");
}

/// A name the engine refuses (a cloud-only name the rows did not show, an 8.3 short name): said as a failed action,
/// and Try again opens New file again. Mutant: `Err(why) => {}` in `State::create_file` -> red (no message).
#[test]
fn create_file_refused_by_the_source_is_said_and_try_again_opens_new_file_again() {
    let (mut rt, _engine, calls, fails) = app();
    fails.lock().unwrap().push((Action::NewFile, ActionError::NameTaken));
    asked(&mut rt, "Report.pdf");
    rt.frame(0.0);
    assert!(
        calls_of(&calls).is_empty(),
        "the source refused before it made anything"
    );
    assert_eq!(
        rt.state.live_toast(),
        Some(Toast::Failed(Action::NewFile, ActionError::NameTaken))
    );
    // w36: the reason is said, not only "Couldn't make the file."
    assert_eq!(
        Toast::failure(Action::NewFile, ActionError::NameTaken),
        "Couldn't make the file. Something here already has that name. Try again with another name."
    );
    assert_eq!(
        Toast::failure(Action::NewFolder, ActionError::NameTaken),
        "Couldn't make the folder.",
        "the other actions' words are unchanged"
    );
    assert!(rt.state.offers_try_again(Action::NewFile, ActionError::NameTaken));
    assert!(rt.state.viewer.new_file.is_none());
    rt.update(|s, tx| s.retry(tx));
    rt.frame(0.0);
    rt.frame(0.0);
    assert!(rt.state.viewer.new_file.is_some(), "Try again opens New file again");
}

/// A full disk and a file too large are said in their own words, not as the generic failure; a full disk offers Try
/// again (make room, then ask), a file too large does not (asking again changes nothing). Mutant: drop the `NoRoom`
/// arm of `Toast::failure` -> red.
#[test]
fn create_file_says_a_full_disk_and_a_too_large_file_in_their_own_words() {
    let (mut rt, _engine, _calls, fails) = app();
    fails.lock().unwrap().push((Action::NewFile, ActionError::NoRoom));
    asked(&mut rt, "Plans.md");
    rt.frame(0.0);
    assert_eq!(
        rt.state.live_toast(),
        Some(Toast::Failed(Action::NewFile, ActionError::NoRoom))
    );
    assert_eq!(
        Toast::failure(Action::NewFile, ActionError::NoRoom),
        concat!(
            "Couldn't make the file. There is not enough space on ",
            beebeeb_engine::device_word!(),
            "."
        )
    );
    assert_eq!(
        Toast::failure(Action::NewFile, ActionError::TooLarge),
        "Couldn't make the file. It is too large to make here."
    );
    assert!(rt.state.offers_try_again(Action::NewFile, ActionError::NoRoom));
    assert!(!rt.state.offers_try_again(Action::NewFile, ActionError::TooLarge));
}

/// Asked under one session, the gate closed before the poll (a lock, a sign-out, another account): nothing is made,
/// and the ask is dropped, so the session that opens next never makes it. Mutant: drop the
/// `refused_without_session` check in `State::create_file` -> red.
#[test]
fn create_file_asked_before_the_session_gate_closed_is_never_made() {
    for closing in ["lock", "sign out", "another account"] {
        let (mut rt, engine, calls, _) = app();
        asked(&mut rt, "Plans.md");
        match closing {
            "lock" => engine.set_gate(GateState::Locked),
            "sign out" => engine.set_gate(GateState::SignedOut),
            _ => engine.set_account("user-b"),
        }
        rt.frame(0.0);
        assert!(calls_of(&calls).is_empty(), "{closing}: {:?}", calls_of(&calls));
        assert!(rt.state.viewer.asked.is_empty(), "{closing}: the ask was kept");
        // The same account comes back: what was asked before is not made now.
        engine.set_account("fixture-account");
        engine.set_gate(GateState::Ready);
        rt.frame(0.0);
        rt.frame(0.0);
        assert!(
            calls_of(&calls).is_empty(),
            "{closing}: made later: {:?}",
            calls_of(&calls)
        );
    }
}
