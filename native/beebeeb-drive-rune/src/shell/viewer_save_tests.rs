//! "Save a copy…" on the fake platform: the panel's answer is bound to the session of the press.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};

use rune::{Env, Runtime};

use super::super::actions::FixtureEngine;
use super::super::host::{Shell, poll as shell_poll};
use super::super::platform::fake::FakePlatform;
use super::super::{registry, state};
use super::*;
use crate::frames::viewers::source::{FixtureViewerSource, board};

const ENV: Env = Env {
    system_dark: false,
    scale: 2.0,
};

/// A saver of its own: the session it names moves when a test says (a sign-out), and a copy writes `bytes`.
struct FakeSaver {
    turn: AtomicU64,
    fails: AtomicBool,
    saved: Mutex<Vec<PathBuf>>,
}

impl FakeSaver {
    fn new() -> Arc<FakeSaver> {
        Arc::new(FakeSaver {
            turn: AtomicU64::new(1),
            fails: AtomicBool::new(false),
            saved: Mutex::default(),
        })
    }
}

impl CopySaver for FakeSaver {
    fn turn(&self) -> SessionGeneration {
        SessionGeneration::for_test(self.turn.load(Ordering::SeqCst))
    }
    fn save(
        &self,
        _session: SessionGeneration,
        _file_id: &str,
        dest: &Path,
        _progress: &crate::engine_host::save_copy::Progress,
        _cancel: &AtomicBool,
    ) -> Result<(), SaveRefused> {
        self.saved.lock().unwrap().push(dest.to_path_buf());
        if self.fails.load(Ordering::SeqCst) {
            return Err(SaveRefused::Failed);
        }
        std::fs::write(dest, b"copy").map_err(|_| SaveRefused::Failed)
    }
}

fn runtime() -> Runtime<State> {
    let state = state::init_with(Shell::fixture_on(
        Arc::new(FixtureEngine::ready()),
        Arc::new(FakePlatform::default()),
    ));
    let mut rt = Runtime::new(state, registry::view, (1280.0, 800.0), ENV);
    rt.poll = Some(shell_poll);
    rt.frame(0.0);
    rt
}

/// Presses Save a copy on the open fixture file with `platform` and `saver`, then polls until the save ended.
fn press(rt: &mut Runtime<State>, platform: Arc<FakePlatform>, saver: Arc<FakeSaver>) -> Option<String> {
    rt.update(|s, tx| {
        assert!(viewers::open_from(
            s,
            Box::new(FixtureViewerSource::new()),
            board::AUDIO
        ));
        assert!(save_with(
            s,
            tx,
            platform,
            saver,
            "id".into(),
            "take-3.m4a".into(),
            PathBuf::new()
        ));
    });
    for _ in 0..400 {
        let mut ended = false;
        rt.update(|s, tx| {
            let was = running();
            poll(s, tx);
            ended = was && !running();
        });
        if ended {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let mut toast = None;
    rt.update(|s, _| toast = viewers::ui(s).and_then(|ui| ui.toast.clone()));
    toast
}

#[test]
fn viewer_save_asks_the_panel_with_the_files_name_and_writes_the_copy_there() {
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("take-3.m4a");
    let platform = Arc::new(FakePlatform::default());
    platform
        .saves
        .lock()
        .unwrap()
        .push_back(SaveAnswer::Chosen(dest.clone()));
    let saver = FakeSaver::new();
    let toast = press(&mut runtime(), platform.clone(), saver.clone());
    assert_eq!(platform.save_asks.lock().unwrap()[0].0, "take-3.m4a");
    assert_eq!(std::fs::read(&dest).unwrap(), b"copy");
    assert_eq!(toast.as_deref(), Some(SAVED));
}

#[test]
fn viewer_save_a_place_chosen_after_sign_out_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("late.m4a");
    let saver = FakeSaver::new();
    let pressed = saver.turn();
    // The session moved on while the panel was open.
    saver.turn.store(2, Ordering::SeqCst);
    let platform = FakePlatform::default();
    platform
        .saves
        .lock()
        .unwrap()
        .push_back(SaveAnswer::Chosen(dest.clone()));
    let end = ask_and_save(
        &platform,
        &*saver,
        pressed,
        &AtomicBool::new(false),
        &|_, _| {},
        "id",
        "late.m4a",
        PathBuf::new(),
    );
    assert_eq!(end, Some(Err(SaveRefused::NotSignedIn.words())));
    assert!(saver.saved.lock().unwrap().is_empty(), "no copy was started");
    assert!(!dest.exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn viewer_save_a_panel_closed_without_a_place_saves_nothing_and_says_nothing() {
    let saver = FakeSaver::new();
    let toast = press(&mut runtime(), Arc::new(FakePlatform::default()), saver.clone());
    assert!(saver.saved.lock().unwrap().is_empty());
    assert_eq!(toast, None);
}

#[test]
fn viewer_save_a_refused_copy_says_so_without_a_name_and_leaves_no_file() {
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("x.m4a");
    let platform = Arc::new(FakePlatform::default());
    platform.saves.lock().unwrap().push_back(SaveAnswer::Chosen(dest));
    let saver = FakeSaver::new();
    saver.fails.store(true, Ordering::SeqCst);
    let toast = press(&mut runtime(), platform, saver);
    assert_eq!(toast, Some(SaveRefused::Failed.words()));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn viewer_save_takes_only_its_two_intents_and_without_an_engine_says_not_available() {
    let mut rt = runtime();
    rt.update(|s, tx| {
        assert!(viewers::open_from(
            s,
            Box::new(FixtureViewerSource::new()),
            board::AUDIO
        ));
        assert!(!take(s, tx, Intent::Share));
        assert!(take(s, tx, Intent::SaveCopy));
        assert_eq!(viewers::ui(s).unwrap().toast.as_deref(), Some(NOT_AVAILABLE));
    });
}

#[test]
fn viewer_save_the_shells_poll_hands_the_result_to_the_toast() {
    let dir = tempfile::tempdir().unwrap();
    let platform = Arc::new(FakePlatform::default());
    platform
        .saves
        .lock()
        .unwrap()
        .push_back(SaveAnswer::Chosen(dir.path().join("p.m4a")));
    let saver = FakeSaver::new();
    let mut rt = runtime();
    super::super::intents::poll_drains_for_this_test();
    rt.update(|s, tx| {
        assert!(viewers::open_from(
            s,
            Box::new(FixtureViewerSource::new()),
            board::AUDIO
        ));
        assert!(save_with(
            s,
            tx,
            platform,
            saver,
            "id".into(),
            "p.m4a".into(),
            PathBuf::new()
        ));
    });
    let mut toast = None;
    for _ in 0..400 {
        rt.update(|s, tx| {
            super::super::intents::drain_in_poll(s, tx);
            toast = viewers::ui(s).and_then(|ui| ui.toast.clone());
        });
        if toast.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(toast.as_deref(), Some(SAVED));
}

/// A saver that reports 30 of 100 and then waits until the test lets the copy end.
struct Slow {
    go: Mutex<mpsc::Receiver<()>>,
}

impl CopySaver for Slow {
    fn turn(&self) -> SessionGeneration {
        SessionGeneration::for_test(1)
    }
    fn save(
        &self,
        _session: SessionGeneration,
        _file_id: &str,
        dest: &Path,
        progress: &crate::engine_host::save_copy::Progress,
        _cancel: &AtomicBool,
    ) -> Result<(), SaveRefused> {
        progress(30, 100);
        let _ = self.go.lock().unwrap().recv();
        std::fs::write(dest, b"copy").map_err(|_| SaveRefused::Failed)
    }
}

#[test]
fn viewer_save_says_how_far_the_copy_is_while_it_is_written_then_that_it_is_saved() {
    let dir = tempfile::tempdir().unwrap();
    let platform = Arc::new(FakePlatform::default());
    platform
        .saves
        .lock()
        .unwrap()
        .push_back(SaveAnswer::Chosen(dir.path().join("take-3.m4a")));
    let (let_go, go) = mpsc::channel();
    let saver = Arc::new(Slow { go: Mutex::new(go) });
    let mut rt = runtime();
    rt.update(|s, tx| {
        assert!(viewers::open_from(
            s,
            Box::new(FixtureViewerSource::new()),
            board::AUDIO
        ));
        assert!(save_with(
            s,
            tx,
            platform,
            saver,
            "id".into(),
            "take-3.m4a".into(),
            PathBuf::new()
        ));
    });
    let mut said = None;
    for _ in 0..400 {
        rt.update(|s, tx| {
            poll(s, tx);
            said = viewers::ui(s).and_then(|ui| ui.toast.clone());
        });
        if said.as_deref() == Some("Saving a copy\u{2026} 30%") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        said.as_deref(),
        Some("Saving a copy\u{2026} 30%"),
        "the viewer says how far it is"
    );
    assert!(running());
    let_go.send(()).unwrap();
    for _ in 0..400 {
        let mut ended = false;
        rt.update(|s, tx| {
            let was = running();
            poll(s, tx);
            ended = was && !running();
        });
        if ended {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let mut toast = None;
    rt.update(|s, _| toast = viewers::ui(s).and_then(|ui| ui.toast.clone()));
    assert_eq!(toast.as_deref(), Some(SAVED));
}

#[test]
fn viewer_save_progress_words_never_say_whole_before_the_end() {
    assert_eq!(progress_words(0, 0), "Saving a copy\u{2026}");
    assert_eq!(progress_words(50, 100), "Saving a copy\u{2026} 50%");
    assert_eq!(progress_words(100, 100), "Saving a copy\u{2026} 99%");
}
