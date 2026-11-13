//! The shell's tests of what happens around a session: the first sync, an approval under a lock or a sign-out,
//! sign-up in the sign-in window, capture exclusion on the way out of a secret screen. Moved out of `host_tests.rs`
//! unchanged when that file passed the size limit.

use super::*;

/// A source with nothing synced on this PC yet (`state.db` does not exist: every read is `Unreadable`) until the
/// test says the first sync wrote it; it counts the reads.
#[derive(Clone, Default)]
struct FirstSync {
    synced: Arc<std::sync::atomic::AtomicBool>,
    reads: Arc<std::sync::atomic::AtomicU32>,
}

impl FirstSync {
    fn reads(&self) -> u32 {
        self.reads.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl crate::source::DriveSource for FirstSync {
    fn list(&self, folder_id: Option<&str>) -> Result<crate::source::Listing, crate::source::SourceError> {
        self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if !self.synced.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(crate::source::SourceError::Unreadable);
        }
        crate::source::DriveSource::list(&crate::source::BoardFixtureSource::new(), folder_id)
    }
    fn storage(&self) -> Option<crate::source::StorageSummary> {
        None
    }
}

/// The first sign-in on a PC (R1.1 of the sync-start contract): the gate opens before anything has synced, so the
/// root cannot be read and the window says so. The running engine's first finished check reads the root again, with
/// no click. Nothing is read on the frames in between, and a root that reads is not read again from here.
#[test]
fn the_first_finished_check_shows_the_files_without_a_click() {
    use super::super::super::tray::EngineState;
    let engine = Arc::new(FixtureEngine::with(GateState::SignedOut, TrayInputs::default()));
    let source = FirstSync::default();
    let shell = Shell::fixture_on(engine.clone(), Arc::new(FakePlatform::default()));
    let mut rt = runtime(state::init_from(shell, Box::new(source.clone()), None));
    let state = |engine_state| TrayInputs {
        signed_in: true,
        engine: engine_state,
        ..TrayInputs::default()
    };
    engine.set_inputs(state(EngineState::NotRunning));
    engine.set_gate(GateState::Ready);
    settle(&mut rt);
    assert_eq!(rt.state.list_error, Some(crate::source::SourceError::Unreadable));
    let after_the_gate = source.reads();

    engine.set_inputs(state(EngineState::Checking));
    settle(&mut rt);
    source.synced.store(true, std::sync::atomic::Ordering::SeqCst);
    engine.set_inputs(state(EngineState::Syncing));
    settle(&mut rt);
    assert_eq!(
        source.reads(),
        after_the_gate,
        "nothing is read before a check finishes"
    );
    assert_eq!(rt.state.list_error, Some(crate::source::SourceError::Unreadable));

    engine.set_inputs(state(EngineState::Idle));
    settle(&mut rt);
    assert_eq!(rt.state.list_error, None, "the finished check showed the files");
    let board = crate::source::DriveSource::list(&crate::source::BoardFixtureSource::new(), None).unwrap();
    assert_eq!(rt.state.listing.items.len(), board.items.len());
    assert!(rt.find("file-list-error").is_none());

    let shown = source.reads();
    engine.set_inputs(state(EngineState::Syncing));
    settle(&mut rt);
    engine.set_inputs(state(EngineState::Idle));
    settle(&mut rt);
    assert_eq!(source.reads(), shown, "a root that reads is navigation's");
}

/// Approving a sign-in: the screens that hold the approval code are secret (DW-A12, DW-A13,
/// DW-A14); the results, which show after the code is wiped, are not.
#[test]
fn the_approval_code_screens_are_secret_and_the_window_is_excluded_while_they_show() {
    use crate::frames::account::{Dialog as AccountDialog, Pane};
    assert!(Pane::ApproveCode.shows_secret(), "DW-A12");
    assert!(AccountDialog::ApproveReview.shows_secret(), "DW-A13");
    assert!(AccountDialog::ApproveHello.shows_secret(), "DW-A14");
    assert!(!AccountDialog::ApproveApproved.shows_secret(), "DW-A15");
    assert!(!AccountDialog::ApproveDenied.shows_secret(), "DW-A16");
    assert!(
        !AccountDialog::ApproveExpired { handled: false }.shows_secret(),
        "DW-A17"
    );
    // The ends with no board frame show after the code is wiped too.
    assert!(!AccountDialog::ApproveUnsure { denying: false }.shows_secret());
    assert!(!AccountDialog::ApproveUnsure { denying: true }.shows_secret());
    assert!(!AccountDialog::ApproveTooLate.shows_secret());
    assert!(
        !Pane::ApproveReview.shows_secret(),
        "the pane under the dialogs has no secret of its own"
    );

    // End to end: the platform is told when the code page opens, and told again when it closes.
    let engine = Arc::new(FixtureEngine::ready());
    let platform = Arc::new(FakePlatform::default());
    let mut rt = runtime(state::init_with(Shell::fixture_on(engine.clone(), platform.clone())));
    assert!(platform.capture_exclusions.lock().unwrap().is_empty());
    rt.update(|s, _| crate::frames::account::go(s, "DW-A12"));
    rt.frame(1.0);
    assert_eq!(
        platform.capture_exclusions.lock().unwrap().clone(),
        vec![true],
        "DW-A12 excludes the window"
    );
    rt.update(|s, _| crate::frames::account::go(s, "DW-A52"));
    rt.frame(2.0);
    assert_eq!(platform.capture_exclusions.lock().unwrap().clone(), vec![true]);
    // Exclusion drops one poll late: the poll that first sees no secret keeps it and lands the
    // leaving elements; the next one tells the platform.
    rt.frame(2.5);
    assert_eq!(platform.capture_exclusions.lock().unwrap().clone(), vec![true, false]);
}

/// A window on the review of another device's sign-in (DW-A13), its code typed on DW-A12, with the
/// shell's engine in hand. `late`: a cancel cannot stop an answer of Windows Hello that is on its way.
fn on_the_review(late: bool) -> (Arc<FixtureEngine>, Runtime<State>) {
    use crate::frames::account::source::BoardAccount;
    use crate::frames::account::{self, Dialog as AccountDialog, approve};
    let (engine, _, mut rt) = fixture();
    rt.update(move |s, _| {
        let board = BoardAccount::new();
        s.account.source = Some(Box::new(if late { board.late_answers() } else { board }));
        account::go(s, "DW-A12");
    });
    settle(&mut rt);
    press(
        &mut rt,
        Key::Text(approve::BOARD_REQUEST_CODE.replace('-', "")),
        Mods::default(),
    );
    press(&mut rt, Key::Enter, Mods::default());
    assert_eq!(rt.state.account.dialog, Some(AccountDialog::ApproveReview));
    assert!(rt.state.account.approve.matches, "the review holds the typed code");
    (engine, rt)
}

/// What the account lane holds once a session ended under it: no request, no code, no wait, no
/// dialog, and the section back on its list.
fn assert_the_approval_is_gone(rt: &Runtime<State>, why: &str) {
    use crate::frames::account::Pane;
    let a = &rt.state.account.approve;
    assert!(a.request.is_none(), "{why}: the request is forgotten");
    assert!(a.code.text.is_empty(), "{why}: the code is wiped");
    assert!(!a.matches && !a.waiting, "{why}: nothing is live");
    assert_eq!(rt.state.account.dialog, None, "{why}: the review is closed");
    assert_eq!(rt.state.account.pane, Pane::Security, "{why}: back on the list");
}

/// Review P2-1: a lock ends the approval. After the unlock the person starts again from the
/// list: no code on screen, no Approve to press.
#[test]
fn a_lock_in_the_middle_of_a_review_ends_it_and_the_unlock_shows_no_code_and_no_approve() {
    let (engine, mut rt) = on_the_review(false);
    engine.set_gate(GateState::Locked);
    settle(&mut rt);
    assert_the_approval_is_gone(&rt, "locked");
    engine.set_gate(GateState::Ready);
    settle(&mut rt);
    rt.update(|s, _| crate::frames::account::go(s, "DW-A52"));
    settle(&mut rt);
    assert_the_approval_is_gone(&rt, "unlocked");
    let elements = rt.elements();
    assert!(
        !elements
            .iter()
            .any(|e| e.key.as_deref() == Some(crate::frames::account::approve::CODE_KEY)),
        "no code field shows"
    );
    assert!(
        !texts(&rt).iter().any(|t| t == "Approve" || t.contains("Anna's iPhone")),
        "no Approve and nothing of the request shows"
    );
}

/// Review P2-1: Windows Hello is up when the app locks. The prompt is cancelled, and its answer,
/// arriving while the lock screen shows (and after), changes nothing.
#[test]
fn a_windows_hello_answer_that_arrives_while_locked_changes_nothing() {
    use crate::frames::account::Dialog as AccountDialog;
    let (engine, mut rt) = on_the_review(true);
    // Approve, with the answer not read yet: the wait is open when the lock comes.
    rt.update(|s, _| {
        crate::frames::account::approve::press_approve(s);
        assert!(s.account.approve.waiting);
        assert_eq!(s.account.dialog, Some(AccountDialog::ApproveHello));
    });
    engine.set_gate(GateState::Locked);
    // The engine's status event reads the session before the next poll does.
    rt.update(|s, _| assert!(s.shell.refresh(), "the lock shows"));
    settle(&mut rt);
    assert_the_approval_is_gone(&rt, "locked with Windows Hello up");
    let sent = rt.state.account.source.as_deref().expect("the fixture").sent();
    assert_eq!(
        sent.iter().filter(|line| line.starts_with("cancel approval")).count(),
        1,
        "the prompt was cancelled once: {sent:?}"
    );
    // The answer ("approved") was on its way and the cancel could not stop it: it is taken
    // and dropped while the lock screen shows.
    let waiting = |rt: &mut Runtime<State>| {
        let mut left = None;
        rt.update(|s, _| left = s.account.source.as_mut().expect("the fixture").approval_answer());
        left
    };
    assert_eq!(waiting(&mut rt), None, "the late answer was taken while locked");
    engine.set_gate(GateState::Ready);
    settle(&mut rt);
    settle(&mut rt);
    assert_the_approval_is_gone(&rt, "unlocked");
}

/// Review P2-1: account A reviews a request, signs out, account B signs in and opens Sign-in and
/// security. Nothing of A's request is there and Approve cannot be reached.
#[test]
fn a_sign_out_ends_the_review_and_the_next_account_sees_none_of_it() {
    let (engine, mut rt) = on_the_review(false);
    engine.set_gate(GateState::SignedOut);
    settle(&mut rt);
    assert_the_approval_is_gone(&rt, "signed out");
    engine.set_account("user-b");
    engine.set_gate(GateState::Ready);
    settle(&mut rt);
    rt.update(|s, _| crate::frames::account::go(s, "DW-A52"));
    settle(&mut rt);
    assert_eq!(rt.state.shell.app.account(), Some("user-b"));
    assert_the_approval_is_gone(&rt, "the next account");
    assert!(
        !texts(&rt)
            .iter()
            .any(|t| t == "Approve" || t.contains("Anna's iPhone") || t.contains("203.0.113.47")),
        "0 of the first account's request"
    );
    let before = rt.state.account.source.as_deref().expect("the fixture").sent().len();
    rt.update(crate::frames::account::approve::press_approve_tx);
    assert_eq!(
        rt.state.account.source.as_deref().expect("the fixture").sent().len(),
        before,
        "Approve reaches nothing"
    );
}

/// Review P2-1: the engine reports another account with no sign-out in between (the gate stays
/// Ready): the review ends all the same.
#[test]
fn another_account_under_an_open_review_ends_it() {
    let (engine, mut rt) = on_the_review(false);
    engine.set_account("user-b");
    settle(&mut rt);
    assert_the_approval_is_gone(&rt, "another account");
}

/// Review P2-1, the same gap in the account forms (it was there before the approval flow): a
/// password typed on Change password does not survive a lock.
#[test]
fn a_lock_wipes_what_was_typed_in_an_account_form() {
    use crate::frames::account::Pane;
    let (engine, _, mut rt) = fixture();
    rt.update(|s, _| crate::frames::account::go(s, "DW-A67"));
    settle(&mut rt);
    press(&mut rt, Key::Text("typed-here".into()), Mods::default());
    assert!(
        !rt.state.account.form.current.text.is_empty(),
        "the form holds what was typed"
    );
    engine.set_gate(GateState::Locked);
    settle(&mut rt);
    assert!(rt.state.account.form.current.text.is_empty(), "the lock wipes the form");
    assert_eq!(
        rt.state.account.pane,
        Pane::Security,
        "and the section is back on its list"
    );
}

/// Sign-up (DW-A31 to DW-A44) is a set of screens of the sign-in window: "Create a Beebeeb account" opens
/// it on the shell's next poll, and the window stays excluded from capture on every one of its screens:
/// the password (DW-A36 to DW-A38), the recovery phrase shown (DW-A39) and typed back (DW-A40, DW-A41).
#[test]
fn sign_up_opens_in_the_sign_in_window_and_every_screen_of_it_stays_excluded_from_capture() {
    use crate::frames::signin::create::{FRAME_IDS, Step};
    use crate::frames::signin::{Intent as SignIn, Screen};
    let engine = Arc::new(FixtureEngine::signed_out());
    let platform = Arc::new(FakePlatform::default());
    let mut rt = runtime(state::init_with(Shell::fixture_on(engine, platform.clone())));
    rt.update(|s, _| {
        s.signin = crate::frames::signin::scene("DW-A01");
        s.signin.as_mut().unwrap().leave(SignIn::CreateAccount);
    });
    rt.frame(1.0);
    rt.update(|s, _| assert_eq!(s.signin.as_ref().unwrap().screen, Screen::Create(Step::Email)));

    let mut secret_screens = 0;
    for (i, frame) in FRAME_IDS.iter().enumerate() {
        rt.update(|s, _| s.signin = crate::frames::signin::scene(frame));
        rt.frame(2.0 + i as f64);
        assert!(registry::route_shows_secret(&rt.state.shell.app), "{frame}");
        assert!(rt.state.shell.capture_excluded, "{frame} can be captured");
        if let Screen::Create(step) = rt.state.signin.as_ref().unwrap().screen {
            secret_screens += usize::from(step.shows_secret());
        }
    }
    assert_eq!(secret_screens, 6, "DW-A36 to DW-A41");
    assert_eq!(
        platform.capture_exclusions.lock().unwrap().clone(),
        vec![true],
        "excluded once, at the start, and never lifted between the screens"
    );
}
/// The reviewer's open point: capture exclusion must not drop while the code can still be
/// painted. Rune keeps a leaving element in the tree while it animates out, and the poll that
/// drops exclusion runs before the frame without the code is shown. So on every way out of a
/// screen that holds the code (a press, Windows Hello's answer inside the poll, a lock), at every
/// frame where the platform was last told "not excluded", the code field is not in the tree, not
/// even as a leaving element.
#[test]
fn capture_exclusion_drops_only_after_the_code_is_out_of_the_tree() {
    use crate::frames::account::Dialog as AccountDialog;
    use crate::frames::account::approve::{self, CODE_KEY};
    use crate::frames::account::source::BoardAccount;
    type Way = fn(&Arc<FixtureEngine>, &mut Runtime<State>);
    let ways: [(&str, Way); 5] = [
        // A key inside a frame: its handler runs after that frame's poll, and its commit animates.
        ("Esc (Deny), pressed inside a frame", |_, rt| {
            rt.key(KeyEvent {
                key: Key::Escape,
                mods: Mods::default(),
            })
        }),
        ("Deny between frames", |_, rt| {
            rt.update(|s, _| assert!(approve::on_escape(s)))
        }),
        ("the answer of Windows Hello, inside the poll", |_, rt| {
            rt.update(|s, _| approve::press_approve(s))
        }),
        ("leaving the section", |_, rt| {
            rt.update(|s, _| crate::frames::account::go(s, "DW-A51"))
        }),
        ("a sign-out to another account", |engine, _| {
            engine.set_account("user-b")
        }),
    ];
    for (way, leave) in ways {
        let engine = Arc::new(FixtureEngine::ready());
        let platform = Arc::new(FakePlatform::default());
        let mut rt = runtime(state::init_with(Shell::fixture_on(engine.clone(), platform.clone())));
        rt.update(|s, _| {
            s.account.source = Some(Box::new(BoardAccount::new()));
            crate::frames::account::go(s, "DW-A12");
        });
        settle(&mut rt);
        press(
            &mut rt,
            Key::Text(approve::BOARD_REQUEST_CODE.replace('-', "")),
            Mods::default(),
        );
        press(&mut rt, Key::Enter, Mods::default());
        assert_eq!(rt.state.account.dialog, Some(AccountDialog::ApproveReview), "{way}");
        let excluded = |platform: &FakePlatform| platform.capture_exclusions.lock().unwrap().last().copied();
        assert_eq!(excluded(&platform), Some(true), "{way}: the review is excluded");
        leave(&engine, &mut rt);
        let mut frames_with_the_code = 0;
        for frame in 0..60 {
            rt.frame(0.016);
            let code_in_tree = rt.elements().iter().any(|e| e.key.as_deref() == Some(CODE_KEY));
            frames_with_the_code += usize::from(code_in_tree);
            assert!(
                !(code_in_tree && excluded(&platform) == Some(false)),
                "{way}: frame {frame} still has the code field in the tree and capture is not excluded"
            );
        }
        assert_eq!(excluded(&platform), Some(false), "{way}: exclusion drops in the end");
        assert!(!rt.state.account.shows_secret(), "{way}");
        // At most the one frame of a press inside a frame (still excluded) holds the leaving field.
        assert!(frames_with_the_code <= 1, "{way}: {frames_with_the_code} frames");
    }
}

/// The Share dialog's password field (DW-0407) is a secret on a route that is not one: the
/// window is excluded from capture exactly while the field shows. The link itself is never
/// drawn, so the dialog without the field leaves the window capturable.
#[test]
fn a_links_password_field_excludes_the_window_from_capture_while_it_shows() {
    let (_engine, platform, mut rt) = fixture();
    let exclusions = || platform.capture_exclusions.lock().unwrap().clone();
    let centre_of = |rt: &Runtime<State>, key: &str| {
        let e = rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
            .unwrap_or_else(|| panic!("no element keyed {key}"));
        (e.rect[0] + e.rect[2] / 2.0, e.rect[1] + e.rect[3] / 2.0)
    };
    let click = |rt: &mut Runtime<State>, key: &str| {
        let (x, y) = centre_of(rt, key);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        for i in 0..3 {
            rt.frame(10.0 + f64::from(i));
        }
    };
    // Share…, from the details panel's button: the dialog alone shows no secret.
    click(&mut rt, "share-box");
    assert!(matches!(
        rt.state.actions.dialog,
        Some(crate::frames::files::dialogs::Dialog::Share(_))
    ));
    assert!(
        exclusions().is_empty(),
        "the dialog without a password field is capturable"
    );
    // Password on: the field shows, and the platform is told to exclude the window.
    click(&mut rt, "item-0-2-switch");
    assert!(crate::frames::files::share::shows_secret(&rt.state));
    assert_eq!(exclusions(), vec![true]);
    assert!(rt.state.shell.capture_excluded);
    // Password off again: capturable again.
    click(&mut rt, "item-0-2-switch");
    assert_eq!(exclusions(), vec![true, false]);
    // On, then Cancel: the dialog and the field are gone, and so is the exclusion.
    click(&mut rt, "item-0-2-switch");
    assert_eq!(exclusions(), vec![true, false, true]);
    click(&mut rt, "btn-2");
    assert!(rt.state.actions.dialog.is_none());
    assert_eq!(exclusions(), vec![true, false, true, false]);
    assert!(!rt.state.shell.capture_excluded);
}
