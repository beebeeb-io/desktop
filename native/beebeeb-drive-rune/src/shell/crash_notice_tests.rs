//! drive-complete-138b: the crash notice on the next start. It shows once the vault is open; Send a report opens
//! Report a problem with the record shown; Dismiss deletes the record.

use std::sync::Arc;

use rune::{Env, Runtime};

use super::actions::FixtureEngine;
use super::host::{Shell, poll};
use super::platform::fake::FakePlatform;
use super::{registry, state};
use crate::State;
use crate::frames::settings::Dialog;
use crate::log_file::crash;

const ENV: Env = Env {
    system_dark: false,
    scale: 2.0,
};
const RECORD: &str = "2026-11-22T10:00:00Z CRASH version=0.9.0 thread=other location=main.rs:80\n";

fn runtime_after_crash(tag: &str) -> (Runtime<State>, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("bb-crash-notice-{tag}-{}", std::process::id()));
    crash::write(&dir, RECORD);
    let record = crash::take_pending(&dir).expect("record");
    let mut state = state::init_with(Shell::fixture_on(
        Arc::new(FixtureEngine::ready()),
        Arc::new(FakePlatform::default()),
    ));
    state.crash = Some(record);
    let mut rt = Runtime::new(state, registry::view, (1280.0, 800.0), ENV);
    rt.poll = Some(poll);
    super::intents::poll_drains_for_this_test();
    rt.frame(0.0);
    rt.frame(0.016);
    (rt, dir)
}

fn texts(rt: &Runtime<State>) -> String {
    rt.elements()
        .into_iter()
        .filter(|e| !e.exiting)
        .flat_map(|e| [e.text, e.label].into_iter().flatten())
        .collect::<Vec<_>>()
        .join("\n")
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let e = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .unwrap_or_else(|| panic!("no {key}"));
    let r = e.rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    for _ in 0..30 {
        rt.frame(0.05);
    }
}

#[test]
fn the_crash_notice_shows_and_send_a_report_opens_report_a_problem_with_the_record() {
    let (mut rt, dir) = runtime_after_crash("send");
    assert!(texts(&rt).contains(super::crash_notice::WORDS), "the notice shows");
    click(&mut rt, "crash-notice-send");
    assert_eq!(rt.state.settings.dialog, Some(Dialog::ReportProblem));
    let drawn = texts(&rt);
    assert!(!drawn.contains(super::crash_notice::WORDS), "the notice went");
    assert!(
        drawn.contains(RECORD.trim_end()),
        "the record is shown before sending: {drawn}"
    );
    assert!(rt.state.settings.crash_record.is_some(), "attached");
    assert!(crash::take_pending(&dir).is_some(), "kept until a bundle holds it");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dismissing_the_crash_notice_deletes_the_record() {
    let (mut rt, dir) = runtime_after_crash("dismiss");
    click(&mut rt, "crash-notice-dismiss");
    assert!(!texts(&rt).contains(super::crash_notice::WORDS));
    assert!(rt.state.crash.is_none());
    assert!(crash::take_pending(&dir).is_none(), "deleted");
    let _ = std::fs::remove_dir_all(&dir);
}
