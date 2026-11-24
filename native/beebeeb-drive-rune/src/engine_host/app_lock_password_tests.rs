//! "Use your password" for app lock (no board frame: D-2) on the real engine and the parent modules' fake: Windows
//! Hello did not confirm turning app lock off or a longer delay, and the account password does it instead. The engine
//! checks and counts the password (never mocked here). Every name and password is invented; nothing waits on Windows.

use std::time::Duration;

use beebeeb_engine::app::lock_password_api::{AppLockProof, AppLockRefused, LockAfter};
use rune::{Env, Runtime, Tx};
use zeroize::Zeroizing;

use super::*;
use crate::State;
use crate::engine_host::hello::HelloResult;
use crate::frames::account::app_lock_password::{LINK_KEY, LockPassword, TITLE};
use crate::shell::app::{Event as ShellEvent, SettingsPane};
use crate::shell::host;

const WRONG: &str = "not the password at all";
/// A password nothing else in the app could print by chance.
const MARKER: &str = "marker-7Qv-password-never-printed";

fn password(text: &str) -> LockPassword {
    LockPassword::new(Zeroizing::new(text.to_string()))
}

/// A source on the engine with no Windows Hello prompt on this PC (`NoHello`), Sign-in and security open.
fn without_hello(server: &Server) -> (TestEngine, Source) {
    let engine = signed_in(server);
    let mut source = Source::new(engine.clone());
    opened(&mut source, Pane::Security);
    (engine, source)
}

fn offered(source: &Source) -> Option<crate::frames::account::app_lock_password::PasswordOffer> {
    source.security_facts().app_lock_password
}

/// R1, R3: no prompt on this PC. The switch stays on, the toast says to use the password, the row offers it, and the
/// right password turns app lock off; the row then reads off and the offer ends.
#[test]
fn no_hello_offers_the_password_and_the_password_turns_app_lock_off() {
    let server = server();
    let (engine, mut source) = without_hello(&server);
    source.set_app_lock(false);
    settle(&mut source);
    assert_eq!(switch(&source), Some(true));
    assert_eq!(
        source.notice(),
        Some("App lock is still on: Windows Hello is not set up on this PC. Use your password instead.")
    );
    let offer = offered(&source).expect("Use your password is offered");
    assert!(
        offer.turning_off && !offer.checking && offer.said.is_none(),
        "{offer:?}"
    );
    source.app_lock_with_password(password(PASSWORD));
    settle_computed(&mut source);
    assert!(!stored(&engine), "the engine stored off");
    assert_eq!(switch(&source), Some(false), "the row reads off");
    assert_eq!(offered(&source), None, "the step ends");
    assert_eq!(source.notice(), None);
}

/// R1, R3: a cancelled prompt for a longer delay offers the password, and the right one stores the delay. A failed
/// prompt and one that cannot be shown offer it too.
#[test]
fn a_cancelled_hello_offers_the_password_for_a_longer_delay() {
    let server = server();
    let (engine, mut source, prompts) = with_prompts(&server);
    source.set_lock_after("30 min");
    assert_eq!(prompts.asked(), 1);
    prompts.answer(HelloResult::Cancelled);
    settle(&mut source);
    assert_eq!(
        source.notice(),
        Some("The lock delay was not changed: Windows Hello did not confirm it's you.")
    );
    let offer = offered(&source).expect("Use your password is offered");
    assert!(!offer.turning_off, "for the delay: {offer:?}");
    source.app_lock_with_password(password(PASSWORD));
    settle_computed(&mut source);
    assert_eq!(stored_after(&engine), Some(LockAfter::Minutes30));
    assert_eq!(delay(&source).as_deref(), Some("30 min"));
    assert_eq!(offered(&source), None);
    for result in [HelloResult::Failed, HelloResult::Unavailable] {
        source.set_lock_after("60 min");
        prompts.answer(result);
        settle(&mut source);
        assert!(offered(&source).is_some(), "{result:?} offers the password");
        assert_eq!(stored_after(&engine), Some(LockAfter::Minutes30), "{result:?}");
    }
    assert_eq!(
        source.notice(),
        Some("The lock delay was not changed: Windows Hello is not set up on this PC. Use your password instead.")
    );
}

/// R4: a wrong password changes nothing, says so under the field (not in a toast), keeps the step open, and is
/// counted by the engine: after two here, three more straight at the engine make five, and the fifth waits. A try
/// during the wait says the wait.
#[test]
fn a_wrong_password_changes_nothing_and_is_counted() {
    let server = server();
    let (engine, mut source) = without_hello(&server);
    source.set_app_lock(false);
    settle(&mut source);
    for _ in 0..2 {
        source.app_lock_with_password(password(WRONG));
        settle_computed(&mut source);
        let offer = offered(&source).expect("the step stays open");
        assert_eq!(offer.said.as_deref(), Some("That password isn't right."));
        assert!(!offer.checking);
        assert!(stored(&engine));
        assert_eq!(switch(&source), Some(true));
        assert_eq!(source.notice(), None, "said under the field");
    }
    let turn = engine.session_turn();
    let wrong = || Some(AppLockProof::Password(Zeroizing::new(WRONG.to_string())));
    for _ in 0..2 {
        assert_eq!(
            engine.set_app_lock_for(turn, false, wrong()),
            Err(AppLockRefused::Wrong {
                wait_ms: 0,
                local_check_off: false
            })
        );
    }
    let fifth = engine.set_app_lock_for(turn, false, wrong());
    assert!(
        matches!(fifth, Err(AppLockRefused::Wrong { wait_ms, .. }) if wait_ms > 0),
        "the step's two tries were counted: {fifth:?}"
    );
    source.app_lock_with_password(password(PASSWORD));
    settle_computed(&mut source);
    let offer = offered(&source).expect("a wait keeps the step open");
    assert!(
        offer
            .said
            .as_deref()
            .is_some_and(|said| said.starts_with("Too many wrong tries. Try again in ")),
        "{offer:?}"
    );
    assert!(stored(&engine), "nothing changed during the wait");
}

/// R2: an offer made before the screen locked and opened again (another turn of the session) sends nothing, even
/// before the window's next poll: no write, no counted try, and the offer is gone.
#[test]
fn a_password_offer_from_an_earlier_session_turn_does_nothing() {
    let server = server();
    let (engine, mut source) = without_hello(&server);
    source.set_app_lock(false);
    settle(&mut source);
    assert!(offered(&source).is_some());
    engine.lock_screen().unwrap();
    engine.unlock_screen_platform_verified(engine.session_turn()).unwrap();
    let seen = source.app_lock.answers_seen.len();
    source.app_lock_with_password(password(PASSWORD));
    settle_computed(&mut source);
    assert!(stored(&engine), "app lock is still on");
    assert_eq!(switch(&source), Some(true));
    assert!(
        source.app_lock.answers_seen[seen..]
            .iter()
            .all(|answer| !answer.starts_with("Set")),
        "nothing was written: {:?}",
        &source.app_lock.answers_seen[seen..]
    );
    assert_eq!(offered(&source), None);
}

/// R2, the #424 trap: the screen locks and opens again (another turn of the session) while a wrong password is
/// checked. Its answer comes back after the move (the engine answers it as overtaken) and is dropped before it is
/// said: no toast, no step, nothing written for the new turn, and no wrong-try words for a try of another turn.
#[test]
fn a_password_answer_after_the_session_moved_is_dropped_before_it_is_said() {
    let server = server();
    let (engine, mut source) = without_hello(&server);
    source.set_app_lock(false);
    settle(&mut source);
    server.proof_delay_ms.store(1_500, Ordering::SeqCst);
    let seen = source.app_lock.answers_seen.len();
    source.app_lock_with_password(password(WRONG));
    assert!(offered(&source).is_some_and(|offer| offer.checking));
    let moved = Instant::now();
    engine.lock_screen().unwrap();
    engine.unlock_screen_platform_verified(engine.session_turn()).unwrap();
    assert!(
        moved.elapsed() < Duration::from_millis(1_000),
        "the session moved while the password was checked"
    );
    until_computed(&mut source, |source| {
        source.pump();
        source.app_lock.answers_seen[seen..]
            .iter()
            .any(|answer| answer.starts_with("Set"))
    });
    settle(&mut source);
    assert_eq!(source.notice(), None, "said nothing");
    assert_eq!(offered(&source), None, "no step");
    assert!(stored(&engine), "nothing was written for the new turn");
    assert_eq!(switch(&source), Some(true));
}

/// R5 (lead's ruling 2026-11-15 14:34, in place of a log capture): the password's wrapper prints a fixed placeholder,
/// never the text; nothing this part keeps or answers holds it after a wrong and a right try; and the password path's
/// source has no log macro and formats nothing of the password.
#[test]
fn the_password_is_never_printed_kept_or_logged() {
    let typed = password(MARKER);
    assert_eq!(format!("{typed:?}"), "LockPassword(hidden)");
    assert_eq!(format!("{typed}"), "(hidden)");
    let server = server();
    let (engine, mut source) = without_hello(&server);
    source.set_app_lock(false);
    settle(&mut source);
    source.app_lock_with_password(password(&format!("{MARKER} wrong")));
    settle_computed(&mut source);
    let kept = format!("{:?} {:?}", source.app_lock.offer, offered(&source));
    assert!(!kept.contains(MARKER), "kept after a wrong try: {kept}");
    source.app_lock_with_password(password(PASSWORD));
    settle_computed(&mut source);
    assert!(!stored(&engine));
    let answers = source.app_lock.answers_seen.join("\n");
    assert!(answers.contains("Set"), "the answers were read: {answers}");
    assert!(
        !answers.contains(MARKER) && !answers.contains(PASSWORD),
        "an answer held the password"
    );
    assert!(source.app_lock.offer.is_none());
    // The source check: the step's file and this part's password path (`with_password`, `write`).
    let frame = include_str!("../frames/account/app_lock_password.rs");
    let part = include_str!("app_lock.rs");
    let path = [fn_body(part, "fn with_password"), fn_body(part, "fn write")].join("\n");
    let mut found = password_path_problems(frame.split("pub const SCENES").next().unwrap());
    found.extend(password_path_problems(&path));
    assert!(found.is_empty(), "{found:#?}");
}

/// The text of the function that starts at `head`, to its closing brace at the start of a line (4 spaces in).
fn fn_body<'a>(file: &'a str, head: &str) -> &'a str {
    let at = file.find(head).unwrap_or_else(|| panic!("no {head}"));
    let end = file[at..].find("\n    }\n").map_or(file.len(), |end| at + end);
    &file[at..end]
}

/// `token` stands in `line` at the start of a word (`Dialog::LockPassword` holds no `log::`).
fn starts_a_word(line: &str, token: &str) -> bool {
    line.match_indices(token).any(|(at, _)| {
        line[..at]
            .chars()
            .next_back()
            .is_none_or(|before| !(before.is_alphanumeric() || before == '_'))
    })
}

/// Lines of code (comments left out) with a log macro, or a format that mentions the password.
fn password_path_problems(code: &str) -> Vec<String> {
    const LOGS: [&str; 8] = [
        "eprintln!",
        "println!",
        "eprint!",
        "print!",
        "dbg!",
        "log::",
        "tracing::",
        "host::log",
    ];
    const FORMATS: [&str; 4] = ["format!", "write!", "panic!", "expect("];
    const SECRET: [&str; 4] = ["password", "typed", "proof", "self.0"];
    let lines: Vec<&str> = code
        .lines()
        .map(|line| line.split("//").next().unwrap_or_default())
        .collect();
    let mut found = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if LOGS.iter().any(|log| starts_a_word(line, log)) {
            found.push(format!("a log: {}", line.trim()));
        }
        if FORMATS.iter().any(|f| line.contains(f)) {
            let call = lines[i..lines.len().min(i + 4)].join(" ");
            if SECRET.iter().any(|secret| call.contains(secret)) {
                found.push(format!("formats the password: {}", line.trim()));
            }
        }
    }
    found
}

/// R1, R6: Windows Hello's yes writes the change and shows no step; turning app lock on asks nothing and shows none.
#[test]
fn verified_hello_shows_no_password_step() {
    let server = server();
    let (engine, mut source, prompts) = with_prompts(&server);
    source.set_app_lock(false);
    prompts.answer(HelloResult::Verified);
    settle(&mut source);
    assert!(!stored(&engine));
    assert_eq!(offered(&source), None);
    source.set_app_lock(true);
    settle(&mut source);
    assert_eq!(prompts.asked(), 1, "on asks nothing");
    assert!(stored(&engine));
    assert_eq!(offered(&source), None);
    // A shorter delay asks nothing and shows no step either.
    source.set_lock_after("Immediately");
    settle(&mut source);
    assert_eq!(prompts.asked(), 1, "shorter asks nothing");
    assert_eq!(stored_after(&engine), Some(LockAfter::Immediately));
    assert_eq!(offered(&source), None);
}

// ---- in the window ----

fn window_on(engine: TestEngine) -> Runtime<State> {
    let state = crate::shell::state::init_on_engine(crate::shell::host::Shell::fixture(), engine);
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, crate::view, (958.0, 638.0), env);
    rt.on_key = Some(host::on_key);
    rt.poll = Some(|s: &mut State, _: f64, tx: &mut Tx| crate::frames::account::poll(s, tx));
    rt.frame(0.0);
    rt.update(|s, _| {
        host::dispatch(s, ShellEvent::OpenSettings(SettingsPane::Security));
    });
    rt.frame(0.016);
    rt
}

fn has(rt: &Runtime<State>, key: &str) -> bool {
    rt.elements()
        .iter()
        .any(|e| !e.exiting && e.key.as_deref() == Some(key))
}

fn shows(rt: &Runtime<State>, words: &str) -> bool {
    rt.elements()
        .iter()
        .any(|e| !e.exiting && e.text.as_deref() == Some(words))
}

fn wait_for(rt: &mut Runtime<State>, what: &str, done: impl Fn(&Runtime<State>) -> bool) {
    let began = Instant::now();
    while !done(rt) {
        assert!(began.elapsed() < Duration::from_secs(10), "never: {what}");
        std::thread::sleep(Duration::from_millis(20));
        rt.frame(0.1);
    }
}

/// `wait_for` after a press that runs core's password check: `until_computed`'s rule in the window. The source is
/// the window's own, so the worker is seen through the engine: while more hold it than `before` the press, the
/// answer is still on its way; once no more do, it must show within 10 s, or it was lost.
fn wait_computed(
    rt: &mut Runtime<State>,
    engine: &TestEngine,
    before: usize,
    what: &str,
    done: impl Fn(&Runtime<State>) -> bool,
) {
    let (began, mut idle_since) = (Instant::now(), Instant::now());
    loop {
        let computing = Arc::strong_count(engine) > before;
        if done(rt) {
            return;
        }
        if computing {
            idle_since = Instant::now();
        }
        assert!(idle_since.elapsed() < Duration::from_secs(10), "never: {what}");
        assert!(
            began.elapsed() < Duration::from_secs(120),
            "a worker never ended: {what}"
        );
        std::thread::sleep(Duration::from_millis(20));
        rt.frame(0.1);
    }
}

/// A press on the last element keyed `key` (a dialog is drawn over the page, after it).
fn click(rt: &mut Runtime<State>, key: &str) {
    let rect = rt
        .elements()
        .into_iter()
        .rfind(|e| !e.exiting && e.key.as_deref() == Some(key))
        .unwrap_or_else(|| panic!("no element keyed {key}"))
        .rect;
    let (x, y) = (rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.016);
    rt.frame(0.4);
}

/// The running app's window, on a PC where Windows Hello cannot be asked: the switch, the link, the step, a wrong
/// password (said under the field, which is empty and focused again), the right one, Continue. App lock turns off,
/// the step closes, and the field is empty.
#[test]
fn in_the_window_the_password_step_turns_app_lock_off() {
    let server = server();
    let engine = signed_in(&server);
    let mut rt = window_on(engine.clone());
    wait_for(&mut rt, "the switch", |rt| has(rt, "item-1-0-switch"));
    assert!(!has(&rt, LINK_KEY), "not offered before Windows Hello was asked");
    click(&mut rt, "item-1-0-switch");
    wait_for(&mut rt, "Use your password", |rt| has(rt, LINK_KEY));
    click(&mut rt, LINK_KEY);
    wait_for(&mut rt, "the step", |rt| shows(rt, TITLE));
    assert!(rt.state.account.dialog.as_ref().is_some_and(|d| d.shows_secret()));
    // A wrong one: said under the field, the field empty and ready for the next try, app lock still on.
    rt.update(|s, _| s.account.form.current.text.push_str(WRONG));
    rt.frame(0.016);
    let before = Arc::strong_count(&engine);
    click(&mut rt, "btn-0");
    wait_computed(&mut rt, &engine, before, "the wrong password's words", |rt| {
        shows(rt, "That password isn't right.")
    });
    assert!(shows(&rt, TITLE), "the step stays open");
    let field = &rt.state.account.form.current;
    assert!(field.text.is_empty() && field.focused, "empty and focused again");
    assert!(stored(&engine));
    rt.update(|s, _| s.account.form.current.text.push_str(PASSWORD));
    rt.frame(0.016);
    let before = Arc::strong_count(&engine);
    click(&mut rt, "btn-0");
    wait_computed(&mut rt, &engine, before, "app lock off and the step closed", |rt| {
        !shows(rt, TITLE) && shows(rt, &beebeeb_engine::os_words::for_this_os("Off: anyone using this PC can open Beebeeb"))
    });
    assert!(!stored(&engine));
    assert!(rt.state.account.form.current.text.is_empty(), "the field is wiped");
    assert!(!has(&rt, LINK_KEY));
}
