//! The refusal modal (Q3 of the sync-start contract, §8 S1): its words per refusal, that it opens once per refused
//! attempt and only while the gate is Ready, and that its "Try again" asks for one start. On the fixture engine, in a
//! Rune runtime: no window, no OS, no engine.

use std::sync::Arc;

use rune::{Env, Runtime};

use super::*;
use crate::shell::actions::{FixtureCall, FixtureEngine};
use crate::shell::app::{Dialog, GateState};
use crate::shell::host::{Shell, poll};
use crate::shell::platform::fake::FakePlatform;
use crate::shell::{registry, state};

const PRODUCT: &str = "Beebeeb Test";
const FOLDER: &str = "Beebeeb Test";

fn off(attempt: u64, kind: Refused) -> SyncOff {
    SyncOff {
        attempt,
        kind,
        folder: Some(FOLDER.into()),
    }
}

/// Every kind has its own title and its own sentence, names the folder by its name and never a path, and offers the
/// fix this build has: "Try again" where a retry can work, "Quit" where only a restart helps, and for the two kinds
/// with no fix only "Close" and a plain sentence that there is none.
#[test]
fn each_refusal_says_what_happened_and_offers_a_fix_only_where_one_exists() {
    let all: Vec<Words> = Refused::ALL.iter().map(|kind| words(&off(1, *kind), PRODUCT)).collect();
    let mut titles: Vec<&str> = all.iter().map(|w| w.title.as_str()).collect();
    let mut sentences: Vec<&str> = all.iter().map(|w| w.happened.as_str()).collect();
    titles.sort();
    titles.dedup();
    sentences.sort();
    sentences.dedup();
    assert_eq!(
        (titles.len(), sentences.len()),
        (9, 9),
        "one title and one sentence per kind"
    );
    for (kind, words) in Refused::ALL.iter().zip(&all) {
        for text in [&words.title, &words.happened, &words.fix] {
            assert!(
                !text.contains('/') && !text.contains('\\'),
                "{kind:?} names a path: {text}"
            );
            assert!(!text.is_empty());
        }
        assert_eq!(
            words.actions.last(),
            Some(&Action::Close),
            "{kind:?}: Close is the way out"
        );
        let expected: &[Action] = match kind {
            Refused::OtherAccountsData | Refused::IdentityUnknown => &[Action::Close],
            Refused::StopUnconfirmed => &[Action::Quit, Action::Close],
            _ => &[Action::TryAgain, Action::Close],
        };
        assert_eq!(words.actions, expected, "{kind:?}");
        let no_fix = words.fix.contains("no fix");
        assert_eq!(no_fix, words.actions == [Action::Close], "{kind:?}: {}", words.fix);
    }
    // The folder by its name where the kind is about the folder; the code Windows gave.
    let another = words(&off(1, Refused::FolderOfAnotherProvider), PRODUCT);
    assert!(
        another.happened.contains("\u{201c}Beebeeb Test\u{201d}"),
        "{}",
        another.happened
    );
    let failed = words(
        &off(
            1,
            Refused::RegistrationFailed {
                code: Some(0x8007_017C_u32 as i32),
            },
        ),
        PRODUCT,
    );
    assert!(failed.happened.contains("(error 0x8007017C)"), "{}", failed.happened);
    let no_code = words(&off(1, Refused::RegistrationFailed { code: None }), PRODUCT);
    assert!(!no_code.happened.contains("error"), "{}", no_code.happened);
    let unnamed = SyncOff {
        folder: None,
        ..off(1, Refused::FolderMissing)
    };
    assert!(words(&unnamed, PRODUCT).happened.contains("the sync folder"));
}

const ENV: Env = Env {
    system_dark: false,
    scale: 2.0,
};

fn runtime(engine: Arc<FixtureEngine>) -> Runtime<State> {
    let shell = Shell::fixture_on(engine, Arc::new(FakePlatform::default()));
    let mut rt = Runtime::new(state::init_with(shell), registry::view, (1280.0, 800.0), ENV);
    rt.poll = Some(poll);
    rt.frame(0.0);
    rt
}

fn frames(rt: &mut Runtime<State>, count: usize) {
    for _ in 0..count {
        rt.frame(0.016);
    }
    rt.settle_all();
    rt.frame(0.016);
}

fn open(rt: &Runtime<State>) -> bool {
    rt.state.shell.app.dialog() == Some(&Dialog::SyncOff)
}

fn press(rt: &mut Runtime<State>, label: &str) {
    let button = rt
        .elements()
        .into_iter()
        .find(|e| e.role == "button" && !e.inert && e.label.as_deref() == Some(label))
        .unwrap_or_else(|| panic!("no button {label} on the card"));
    let r = button.rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    frames(rt, 2);
}

/// One refused attempt, seen on 5 polls, opens the modal once; closed, it stays closed; the next refused attempt
/// opens it once more; an attempt refused while the gate is closed never opens it, also once the gate opens again.
#[test]
fn the_refusal_modal_opens_once_per_refused_attempt() {
    let engine = Arc::new(FixtureEngine::ready());
    let mut rt = runtime(engine.clone());
    engine.set_sync_refusal(Some(off(1, Refused::FolderOfAnotherProvider)));
    frames(&mut rt, 5);
    assert!(open(&rt));
    assert_eq!(rt.state.shell.sync_off_opened(), 1);
    assert!(rt.find("sync-off").is_some(), "the card is drawn");
    press(&mut rt, "Close");
    frames(&mut rt, 5);
    assert!(!open(&rt), "closed, it stays closed");
    assert_eq!(rt.state.shell.sync_off_opened(), 1);

    engine.set_sync_refusal(Some(off(2, Refused::FolderOfAnotherProvider)));
    frames(&mut rt, 5);
    assert!(open(&rt), "the next refused attempt");
    assert_eq!(rt.state.shell.sync_off_opened(), 2);
    press(&mut rt, "Close");

    engine.set_gate(GateState::Locked);
    frames(&mut rt, 2);
    engine.set_sync_refusal(Some(off(3, Refused::Other)));
    frames(&mut rt, 5);
    engine.set_gate(GateState::Ready);
    frames(&mut rt, 5);
    assert!(!open(&rt), "a refusal while the gate was closed is never shown");
    assert_eq!(rt.state.shell.sync_off_opened(), 2);
}

/// "Try again" closes the modal and asks for exactly one start; "Close" asks for none; a kind with no fix has no
/// "Try again" on its card.
#[test]
fn the_modals_try_again_asks_for_one_start() {
    let engine = Arc::new(FixtureEngine::ready());
    let mut rt = runtime(engine.clone());
    engine.set_sync_refusal(Some(off(1, Refused::RegistrationFailed { code: Some(5) })));
    frames(&mut rt, 2);
    assert!(open(&rt));
    let starts = |engine: &FixtureEngine| engine.calls().iter().filter(|c| **c == FixtureCall::StartSync).count();
    assert_eq!(starts(&engine), 0);
    press(&mut rt, "Try again");
    assert!(!open(&rt), "the modal closed");
    assert_eq!(starts(&engine), 1, "one start was asked for");

    engine.set_sync_refusal(Some(off(2, Refused::OtherAccountsData)));
    frames(&mut rt, 2);
    assert!(open(&rt));
    let labels: Vec<String> = rt
        .elements()
        .into_iter()
        .filter(|e| e.role == "button" && !e.inert)
        .filter_map(|e| e.label)
        .collect();
    assert!(
        labels.iter().any(|l| l == "Close") && !labels.iter().any(|l| l == "Try again"),
        "{labels:?}"
    );
    press(&mut rt, "Close");
    assert_eq!(starts(&engine), 1, "Close asks for nothing");
}
