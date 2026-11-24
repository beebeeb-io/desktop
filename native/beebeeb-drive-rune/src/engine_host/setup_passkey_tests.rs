//! Step 1, "Add a passkey", on the engine double of `setup_tests.rs`: the password, then the list read under the
//! session of the press, then the add (Settings › Passkeys' path) on a worker; the window is never held while
//! Windows' dialog is open, and nothing is claimed before the engine answered. Every name and password is invented.
//!
//! One filter runs them all: `setup_add_passkey`.

use super::*;
use crate::frames::signin::setup::source::PasskeyRefused;
use crate::frames::signin::setup::PasskeyStep;

/// What the person types into step 1's field.
const TYPED: &str = "an invented one";

/// The double adds a passkey, and answers `answer`.
fn binds(set: &Set, answer: Result<(), PasskeyProblem>) {
    *set.engine.passkey.lock().unwrap() = Some(answer);
}

/// The first run in the window, on step 1, with the password typed into its field.
fn on_step_one(set: Set) -> (Runtime<State>, Arc<Fake>) {
    set.engine.set(Start::Running, EngineExplorer::Added);
    let (mut rt, engine, fixture, ..) = window(set);
    fixture.set_gate_for_scene(ShellGate::FirstRun);
    wait_for(&mut rt, "the first run", |rt| rt.state.setup.is_some());
    type_in(&mut rt);
    (rt, engine)
}

/// The password, typed into step 1's field (a press wipes it).
fn type_in(rt: &mut Runtime<State>) {
    rt.update(|s, _| {
        if let Some(ui) = s.setup.as_mut() {
            ui.passkey_password.replace_text(TYPED);
        }
    });
    rt.frame(0.1);
}

fn step_of(rt: &Runtime<State>) -> Option<Step> {
    rt.state.setup.as_ref().map(|ui| ui.step)
}

fn adds(engine: &Fake) -> usize {
    engine
        .passkey_calls
        .lock()
        .unwrap()
        .iter()
        .filter(|call| call.0 == "add")
        .count()
}

/// Step 1, as the engine: the list is handed over only for the session in memory, and an add for another session sends
/// nothing (checked before Windows' dialog, which stays open until the test lets go).
impl SetupPasskeyEngine for Fake {
    fn binds_passkey(&self) -> bool {
        self.passkey.lock().unwrap().is_some()
    }
    fn read_passkeys_for(&self, session: SessionGeneration) -> Result<SessionGeneration, PasskeyProblem> {
        self.passkey_calls
            .lock()
            .unwrap()
            .push(("read", session, String::new()));
        (*self.passkey_list.lock().unwrap())?;
        if session != self.stamp().0 {
            return Err(PasskeyProblem::NotSignedIn);
        }
        Ok(session)
    }
    fn add_passkey_for(
        &self,
        listed_under: SessionGeneration,
        password: zeroize::Zeroizing<String>,
    ) -> Result<(), PasskeyProblem> {
        self.passkey_calls
            .lock()
            .unwrap()
            .push(("add", listed_under, password.as_str().to_owned()));
        if listed_under != self.stamp().0 {
            return Err(PasskeyProblem::NotSignedIn);
        }
        let _dialog = self.passkey_held.lock().unwrap();
        let answer = *self.passkey.lock().unwrap();
        answer.unwrap_or(Err(PasskeyProblem::Unavailable))
    }
}

/// The mutant's test: an add that answers "Added" at the press (without waiting for Windows' dialog) turns this red.
#[test]
fn setup_add_passkey_step_one_stays_until_the_answer_arrives() {
    let set = set();
    binds(&set, Ok(()));
    let (mut rt, engine) = on_step_one(set);
    // Step 1 is offered: its own words and its password field, never "not in this version".
    assert!(says(&rt, "Saved by Windows") && says(&rt, "To confirm it's you."));
    assert!(!says(&rt, "This isn't in this version of Beebeeb yet"));
    // Windows' dialog stays open until this drops.
    let dialog = engine.passkey_held.lock().unwrap();
    press(&mut rt, "Add a passkey");
    for _ in 0..10 {
        rt.frame(0.1);
    }
    let ui = rt.state.setup.as_ref().expect("the first run is open");
    assert_eq!(ui.step, Step::Passkey, "no step 2 before the engine answered");
    assert_eq!(ui.passkey_step, PasskeyStep::Waiting);
    assert!(!ui.results.passkey, "nothing is claimed before the answer");
    let add = rt
        .elements()
        .into_iter()
        .find(|e| !e.exiting && e.role == "button" && e.label.as_deref() == Some("Add a passkey"))
        .expect("the Add button");
    assert!(!add.focusable, "busy while Windows' dialog is open: no second press");
    assert!(says(&rt, "Not now"), "the step can still be left");
    drop(dialog);
    wait_for(&mut rt, "the answer", |rt| {
        step_of(rt).is_some_and(|step| step > Step::Passkey)
    });
    assert_eq!(step_of(&rt), Some(Step::Lock), "added: step 2 follows");
    assert!(rt.state.setup.as_ref().is_some_and(|ui| ui.results.passkey));
    assert_eq!(adds(&engine), 1);
}

/// "You're set up" says Added only once the add was answered; left while the answer was out, it says so, and the
/// answer does not move the window.
#[test]
fn setup_add_passkey_added_says_added_on_youre_set_up() {
    let set = set();
    binds(&set, Ok(()));
    let (mut rt, engine) = on_step_one(set);
    let dialog = engine.passkey_held.lock().unwrap();
    press(&mut rt, "Add a passkey");
    rt.update(|s, _| {
        if let Some(ui) = s.setup.as_mut() {
            ui.skip();
        }
    });
    wait_for(&mut rt, "the last page", |rt| says(rt, "You're set up"));
    assert!(says(&rt, "Passkey, Adding\u{2026}"), "asked, not answered: no value");
    assert!(!says(&rt, "Passkey, Added") && !says(&rt, "Passkey, Not added"));
    drop(dialog);
    wait_for(&mut rt, "the answer", |rt| says(rt, "Passkey, Added"));
    assert_eq!(step_of(&rt), Some(Step::Done), "the answer does not move the window");
    assert_eq!(adds(&engine), 1);
}

/// A refusal is said on step 1 in one line, the step stays and can be left; "You're set up" then says Not added.
#[test]
fn setup_add_passkey_a_refusal_is_said_and_step_one_stays_skippable() {
    let set = set();
    binds(&set, Err(PasskeyProblem::Cancelled));
    let (mut rt, engine) = on_step_one(set);
    press(&mut rt, "Add a passkey");
    wait_for(&mut rt, "the refusal", |rt| {
        says(rt, "Adding the passkey was cancelled. Nothing was changed.")
    });
    assert_eq!(step_of(&rt), Some(Step::Passkey), "step 1 stays");
    let line = rt
        .elements()
        .into_iter()
        .find(|e| !e.exiting && e.key.as_deref() == Some("passkey-refused"))
        .expect("the refusal line");
    assert_eq!(line.role, "alert");
    // Tried again: the step takes the press (the dialog is one at a time, not once).
    type_in(&mut rt);
    press(&mut rt, "Add a passkey");
    wait_for(&mut rt, "the second answer", |_| adds(&engine) == 2);
    press(&mut rt, "Not now");
    assert_eq!(step_of(&rt), Some(Step::Lock));
    rt.update(|s, _| {
        if let Some(ui) = s.setup.as_mut() {
            ui.skip();
        }
    });
    // The second refusal may land after the step was left: the row waits for it, then says Not added.
    wait_for(&mut rt, "the last page", |rt| says(rt, "Passkey, Not added"));
    assert!(!says(&rt, "Passkey, Added"));
}

/// No password: refused at once, and nothing is asked of Beebeeb or of Windows.
#[test]
fn setup_add_passkey_needs_the_password() {
    let mut set = set();
    binds(&set, Ok(()));
    assert!(set.source.available(Step::Passkey), "offered where the engine adds one");
    let before = set.source.workers_started();
    assert!(!set.source.add_passkey(Password::from_field("")), "nothing started");
    assert_eq!(set.source.workers_started(), before, "no worker");
    assert!(
        set.engine.passkey_calls.lock().unwrap().is_empty(),
        "no list read, no add, no dialog"
    );
    assert_eq!(
        set.source.take_passkey_answer(),
        Some(PasskeyAnswer::Refused(PasskeyRefused::NoPassword))
    );
    assert_eq!(set.source.take_passkey_answer(), None, "said once");
    // The control: with the password the same press starts the add (the refusal above was the missing password's).
    assert!(set.source.add_passkey(Password::from_field(TYPED)));
    assert_eq!(set.source.workers_started(), before + 1);
    settle(&mut set, 1);
    assert_eq!(set.source.take_passkey_answer(), Some(PasskeyAnswer::Added));
}

/// Settings › Passkeys' path: the list read under the session of the press first, then the add under that session
/// with the password as typed; a list that cannot be read adds nothing. The real engine takes Settings' own calls.
#[test]
fn setup_add_passkey_uses_the_settings_path() {
    let mut set = set();
    binds(&set, Ok(()));
    assert!(set.source.add_passkey(Password::from_field(TYPED)));
    settle(&mut set, 1);
    assert_eq!(set.source.take_passkey_answer(), Some(PasskeyAnswer::Added));
    assert_eq!(
        *set.engine.passkey_calls.lock().unwrap(),
        [
            ("read", session(1), String::new()),
            ("add", session(1), TYPED.to_string())
        ]
    );
    // The list does not read: nothing is added, and the step says why.
    set.engine.passkey_calls.lock().unwrap().clear();
    *set.engine.passkey_list.lock().unwrap() = Err(PasskeyProblem::Offline);
    set.engine.wakes.store(0, Ordering::SeqCst);
    assert!(set.source.add_passkey(Password::from_field(TYPED)));
    settle(&mut set, 1);
    assert_eq!(
        set.source.take_passkey_answer(),
        Some(PasskeyAnswer::Refused(PasskeyRefused::Offline))
    );
    assert_eq!(adds(&set.engine), 0, "no add without the list");
    // The real engine: its list read, then Settings' own worker function over Windows' dialog.
    let squeezed = |text: &str| -> String { text.chars().filter(|c| !c.is_whitespace()).collect() };
    let slice = squeezed(include_str!("setup_passkey.rs"));
    assert!(slice.contains("letlisted=self.passkeys()?;"));
    assert!(slice.contains("crate::engine_host::passkeys::add_with_dialog(self,hwnd,listed_under,password)"));
    let settings = squeezed(include_str!("passkeys.rs"));
    assert!(settings.contains("pub(crate)fnadd_with_dialog<C:CredentialStores>("));
    assert!(settings.contains("engine.add_passkey(hwnd,session,password,&new_name())"));
}

/// Each refusal of the engine is said as its kind; step 1 can be tried again after it.
#[test]
fn setup_add_passkey_each_refusal_is_said_as_its_kind() {
    for (problem, why) in [
        (PasskeyProblem::WrongPassword, PasskeyRefused::WrongPassword),
        (PasskeyProblem::Cancelled, PasskeyRefused::Cancelled),
        (PasskeyProblem::Offline, PasskeyRefused::Offline),
        (PasskeyProblem::RateLimited, PasskeyRefused::TooManyTries),
        (PasskeyProblem::Unavailable, PasskeyRefused::NotOnThisPc),
        (PasskeyProblem::NotSignedIn, PasskeyRefused::NotAdded),
        (PasskeyProblem::SessionEnded, PasskeyRefused::NotAdded),
        (PasskeyProblem::InvalidInput, PasskeyRefused::NotAdded),
        (PasskeyProblem::Refused, PasskeyRefused::NotAdded),
        (PasskeyProblem::Platform, PasskeyRefused::NotAdded),
    ] {
        let mut set = set();
        binds(&set, Err(problem));
        assert!(set.source.add_passkey(Password::from_field(TYPED)), "{problem:?}");
        settle(&mut set, 1);
        assert_eq!(
            set.source.take_passkey_answer(),
            Some(PasskeyAnswer::Refused(why)),
            "{problem:?}"
        );
        set.engine.wakes.store(0, Ordering::SeqCst);
        assert!(
            set.source.add_passkey(Password::from_field(TYPED)),
            "{problem:?}: tried again"
        );
        settle(&mut set, 1);
    }
}

/// An answer that lands under another session is dropped (never said in it), and another session's press on this
/// first run sends nothing; one add at a time.
#[test]
fn setup_add_passkey_late_answer_dropped() {
    let mut set = set();
    binds(&set, Ok(()));
    let dialog = set.engine.passkey_held.lock().unwrap();
    assert!(set.source.add_passkey(Password::from_field(TYPED)));
    // One at a time: a second press while the first is out starts nothing.
    let before = set.source.workers_started();
    assert!(!set.source.add_passkey(Password::from_field(TYPED)));
    assert_eq!(set.source.workers_started(), before);
    // The add was sent (Windows' dialog is open), then the session moves on.
    let began = Instant::now();
    while adds(&set.engine) == 0 {
        assert!(began.elapsed() < Duration::from_secs(10), "the add never started");
        std::thread::sleep(Duration::from_millis(5));
    }
    *set.engine.stamp.lock().unwrap() = (session(2), Some(FIRST.into()));
    drop(dialog);
    settle(&mut set, 1);
    assert_eq!(
        set.source.take_passkey_answer(),
        None,
        "an answer of another session is dropped"
    );

    // Another session's press on a first run made for the first: nothing is sent, nothing is said.
    let mut other = super::set();
    binds(&other, Ok(()));
    *other.engine.stamp.lock().unwrap() = (session(3), Some(SECOND.into()));
    let before = other.source.workers_started();
    assert!(!other.source.add_passkey(Password::from_field(TYPED)));
    assert_eq!(other.source.workers_started(), before);
    assert!(other.engine.passkey_calls.lock().unwrap().is_empty());
    assert_eq!(other.source.take_passkey_answer(), None);
}

/// Step 1 logs a kind, never the password; what comes down to the window holds no text.
#[test]
fn setup_add_passkey_password_never_logged() {
    let text = include_str!("setup_passkey.rs");
    // Each call of `log(`, to its `;` (not `add_with_dialog(` and the like).
    let mut lines = 0;
    for (at, _) in text.match_indices("log(") {
        if text[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let call = &text[at..at + text[at..].find(';').expect("a log call ends")];
        assert!(!call.contains("password"), "a log line names the password: {call}");
        lines += 1;
    }
    assert!(
        lines >= 1,
        "the scan found step 1's log line (a scan that matches nothing proves nothing)"
    );
    for printer in ["println!", "eprintln!", "dbg!", "{password", "password:?"] {
        assert!(!text.contains(printer), "{printer}");
    }
    let mut set = set();
    binds(&set, Err(PasskeyProblem::WrongPassword));
    assert!(set.source.add_passkey(Password::from_field(TYPED)));
    settle(&mut set, 1);
    let answer = set.source.take_passkey_answer();
    assert_eq!(answer, Some(PasskeyAnswer::Refused(PasskeyRefused::WrongPassword)));
    assert!(!format!("{answer:?}").contains(TYPED));
}
