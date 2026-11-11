//! Tests of the global elements: which banner shows and when it comes back, the keyboard
//! (scripted input), roles and names, and what each control asks of the rest of the app.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use rune::{Env, Key, KeyEvent, Mods, Runtime};

use super::source::{GlobalSource, ListedAt, Storage, StorageLevel, board, size_words};
use super::{BannerKind, FRAMES, GlobalFixture, GlobalStatus, Intent, Ui, sentence_of};
use crate::shell::app::{Screen, SettingsPane};
use crate::ui::banner::sentence;
use crate::{State, init, view};

const NONE: Mods = Mods {
    shift: false,
    cmd: false,
    alt: false,
    ctrl: false,
    logo: false,
};

fn settle(rt: &mut Runtime<State>, from: f64) {
    for i in 1..=3 {
        rt.frame(from + 0.4 * f64::from(i));
    }
}

fn runtime_of(state: State) -> Runtime<State> {
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
    // As the app runs: the shell's keys first, then the frame's through the registry.
    rt.on_key = Some(crate::shell::host::on_key);
    rt.frame(0.0);
    settle(&mut rt, 0.0);
    rt
}

/// The board's window with this state of the whole window behind it.
fn window(status: GlobalStatus) -> Runtime<State> {
    let mut state = init();
    state.global = Ui::new(Box::new(GlobalFixture::of(status)));
    runtime_of(state)
}

fn scene(frame: &str) -> Runtime<State> {
    let mut state = init();
    let (_, apply) = FRAMES
        .iter()
        .find(|(id, _)| *id == frame)
        .unwrap_or_else(|| panic!("no scene {frame}"));
    apply(&mut state);
    // A scene of a cropped board frame keeps the crop; the app has none.
    state.global.board_crop_bottom = false;
    runtime_of(state)
}

fn find(rt: &Runtime<State>, key: &str) -> Option<rune::ElementInfo> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

fn info(rt: &Runtime<State>, key: &str) -> rune::ElementInfo {
    find(rt, key).unwrap_or_else(|| panic!("no element keyed {key}"))
}

fn focused(rt: &Runtime<State>) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|e| e.focused && !e.exiting)
        .and_then(|e| e.key)
}

thread_local! {
    static CLOCK: std::cell::Cell<f64> = const { std::cell::Cell::new(10.0) };
}

fn press(rt: &mut Runtime<State>, key: Key) {
    rt.key(KeyEvent { key, mods: NONE });
    let now = CLOCK.with(|c| {
        c.set(c.get() + 2.0);
        c.get()
    });
    settle(rt, now);
}

/// Tab until `key` has the keyboard (at most 40 stops).
fn tab_to(rt: &mut Runtime<State>, key: &str) {
    for _ in 0..40 {
        if focused(rt).as_deref() == Some(key) {
            return;
        }
        press(rt, Key::Tab);
    }
    panic!("Tab never reached {key}; it stopped at {:?}", focused(rt));
}

fn everything() -> GlobalStatus {
    GlobalStatus {
        signed_out: true,
        storage: Some(Storage {
            used_bytes: 1_000,
            quota_bytes: 1_000,
        }),
        sync_problems: 3,
        backup_problems: 2,
        offline: Some(ListedAt {
            day: "today".into(),
            time: "09:12".into(),
        }),
        ..GlobalStatus::default()
    }
}

fn shown(status: GlobalStatus) -> Option<BannerKind> {
    Ui::new(Box::new(GlobalFixture::of(status)))
        .banner()
        .map(|(kind, _)| kind)
}

/// "One banner at a time, in this order: signed out, storage full, sync problem, backup problem,
/// storage almost full, offline."
#[test]
fn one_banner_at_a_time_in_the_boards_order() {
    let mut status = everything();
    assert_eq!(shown(status.clone()), Some(BannerKind::SignedOut));
    status.signed_out = false;
    assert_eq!(shown(status.clone()), Some(BannerKind::StorageFull));
    status.storage = Some(Storage {
        used_bytes: 950,
        quota_bytes: 1_000,
    });
    assert_eq!(shown(status.clone()), Some(BannerKind::SyncProblem));
    status.sync_problems = 0;
    assert_eq!(shown(status.clone()), Some(BannerKind::BackupProblem));
    status.backup_problems = 0;
    assert_eq!(shown(status.clone()), Some(BannerKind::StorageAlmostFull));
    status.storage = None;
    assert_eq!(shown(status.clone()), Some(BannerKind::Offline));
    status.offline = None;
    assert_eq!(shown(status), None);

    // In the window too: exactly one element keyed `banner`, and it is the first in the order.
    let rt = window(everything());
    let banners = rt
        .elements()
        .into_iter()
        .filter(|e| e.key.as_deref() == Some("banner") && !e.exiting)
        .count();
    assert_eq!(banners, 1);
    assert_eq!(
        info(&rt, "banner").label.as_deref(),
        Some("Backup and sync have stopped. Enter your password to continue. Nothing has been lost.")
    );
}

#[test]
fn a_quiet_window_and_a_locked_app_draw_no_banner() {
    assert!(find(&window(GlobalStatus::default()), "banner").is_none());
    let locked = GlobalStatus {
        locked: true,
        ..everything()
    };
    assert!(find(&window(locked), "banner").is_none());
}

/// The sentences are the board's, word for word, for each of its six samples and the strip.
#[test]
fn every_frame_says_the_boards_sentence() {
    let cases = [
        (
            "DW-0101",
            "You're offline. This list is from today at 09:12. Changes upload when you're back.",
        ),
        ("DW-0102", "3 items can't sync. Everything else is up to date."),
        ("DW-0103", "Your storage is almost full. 950 GB of 1 TB used."),
        (
            "DW-0104",
            "Your storage is full. New uploads and backups are paused. Your files are safe.",
        ),
        (
            "DW-0105",
            "Backup and sync have stopped. Enter your password to continue. Nothing has been lost.",
        ),
        ("DW-0106", "3 items couldn't back up. Everything else is backed up."),
        (
            "DW-0118",
            "Getting your files ready. 18,240 of 40,112 \u{00B7} about 1 min left. File list: In progress \u{00B7} Thumbnails: Waiting for the file list",
        ),
    ];
    for (frame, words) in cases {
        let rt = scene(frame);
        let banner = info(&rt, "banner");
        assert_eq!(banner.role, "status", "{frame}");
        assert_eq!(banner.label.as_deref(), Some(words), "{frame}");
    }
    // One item is "1 item".
    let one = GlobalStatus {
        sync_problems: 1,
        ..GlobalStatus::default()
    };
    assert_eq!(
        sentence(&sentence_of(BannerKind::SyncProblem, &one)),
        "1 item can't sync. Everything else is up to date."
    );
}

/// What the board draws on each banner: a button where there is a place to solve it, a cross
/// only where it may be dismissed.
#[test]
fn buttons_and_crosses_are_where_the_board_draws_them() {
    let cases = [
        ("DW-0101", None, false),
        ("DW-0102", Some("View"), true),
        ("DW-0103", Some("Storage"), true),
        ("DW-0104", Some("Storage"), false),
        ("DW-0105", Some("Sign in"), false),
        ("DW-0106", Some("View"), true),
    ];
    for (frame, action, cross) in cases {
        let rt = scene(frame);
        let button = find(&rt, "banner-action");
        assert_eq!(button.as_ref().and_then(|b| b.label.as_deref()), action, "{frame}");
        if let Some(button) = button {
            assert_eq!((button.role, button.focusable), ("button", true), "{frame}");
        }
        let dismiss = find(&rt, "banner-dismiss");
        assert_eq!(dismiss.is_some(), cross, "{frame}");
        if let Some(dismiss) = dismiss {
            assert_eq!(
                (dismiss.role, dismiss.label.as_deref(), dismiss.focusable),
                ("button", Some("Dismiss"), true),
                "{frame}"
            );
        }
    }
}

/// Keyboard: Tab reaches the banner's button, Enter presses it, and the window goes where it is solved.
#[test]
fn enter_on_the_button_opens_where_it_is_solved() {
    let mut rt = scene("DW-0103");
    tab_to(&mut rt, "banner-action");
    press(&mut rt, Key::Enter);
    assert_eq!(rt.state.global.intents, vec![Intent::Frame("DW-0712")]);
    assert_eq!(rt.state.shell.app.screen(), Screen::Settings(SettingsPane::Storage));

    let mut rt = scene("DW-0106");
    tab_to(&mut rt, "banner-action");
    press(&mut rt, Key::Text(" ".into()));
    assert_eq!(rt.state.global.intents, vec![Intent::Frame("DW-0741")]);
    assert_eq!(rt.state.shell.app.screen(), Screen::Settings(SettingsPane::Backups));

    // Sign in (DW-1109) and the items that didn't sync (DW-0428) are other lanes' frames: asked for.
    let mut rt = scene("DW-0105");
    tab_to(&mut rt, "banner-action");
    press(&mut rt, Key::Enter);
    assert_eq!(rt.state.global.intents, vec![Intent::Frame("DW-1109")]);
    let mut rt = scene("DW-0102");
    tab_to(&mut rt, "banner-action");
    press(&mut rt, Key::Enter);
    assert_eq!(rt.state.global.take_intents(), vec![Intent::Frame("DW-0428")]);
    assert!(rt.state.global.intents.is_empty());
    assert_eq!(super::source::Step::Done.words(), "Done");
}

/// Keyboard: the cross dismisses, the next banner in the order shows, and the keyboard is not lost.
#[test]
fn the_cross_dismisses_from_the_keyboard_and_the_next_banner_shows() {
    let status = GlobalStatus {
        sync_problems: 3,
        offline: board::offline().offline,
        ..GlobalStatus::default()
    };
    let mut rt = window(status);
    assert!(info(&rt, "banner").label.unwrap().starts_with("3 items can't sync"));
    tab_to(&mut rt, "banner-dismiss");
    press(&mut rt, Key::Enter);
    assert!(info(&rt, "banner").label.unwrap().starts_with("You're offline"));
    assert!(find(&rt, "banner-dismiss").is_none(), "offline cannot be dismissed");
    assert!(focused(&rt).is_some(), "the keyboard is somewhere: {:?}", focused(&rt));
}

struct Counting {
    status: std::sync::Arc<std::sync::Mutex<GlobalStatus>>,
}

impl GlobalSource for Counting {
    fn status(&self) -> GlobalStatus {
        self.status.lock().unwrap().clone()
    }
    fn check_connection(&mut self) -> bool {
        false
    }
}

/// "Stays until dismissed; comes back when another item can't sync", and storage "comes back as
/// storage gets fuller (90%, 95%, 99%)". What stops the app cannot be dismissed.
#[test]
fn a_dismissed_banner_comes_back_when_it_happens_again() {
    let status = Arc::new(std::sync::Mutex::new(board::sync_problem()));
    let mut ui = Ui::new(Box::new(Counting { status: status.clone() }));
    assert!(ui.dismiss());
    assert_eq!(ui.banner(), None);
    status.lock().unwrap().sync_problems = 4;
    assert_eq!(ui.banner().map(|(kind, _)| kind), Some(BannerKind::SyncProblem));
    // Solved, then one again: it shows, although 1 is fewer than what was dismissed.
    assert!(ui.dismiss());
    status.lock().unwrap().sync_problems = 0;
    assert_eq!(ui.banner(), None, "solved");
    status.lock().unwrap().sync_problems = 1;
    assert_eq!(ui.banner().map(|(kind, _)| kind), Some(BannerKind::SyncProblem));

    let storage = |used| Storage {
        used_bytes: used,
        quota_bytes: 1_000,
    };
    *status.lock().unwrap() = GlobalStatus {
        storage: Some(storage(905)),
        ..GlobalStatus::default()
    };
    assert_eq!(ui.banner().map(|(kind, _)| kind), Some(BannerKind::StorageAlmostFull));
    assert!(ui.dismiss());
    status.lock().unwrap().storage = Some(storage(940));
    assert_eq!(ui.banner(), None, "still the 90 % step");
    status.lock().unwrap().storage = Some(storage(950));
    assert!(ui.banner().is_some(), "95 %");
    assert!(ui.dismiss());
    status.lock().unwrap().storage = Some(storage(990));
    assert!(ui.banner().is_some(), "99 %");

    for fixed in [board::signed_out(), board::storage_full(), board::offline()] {
        let mut ui = Ui::new(Box::new(GlobalFixture::of(fixed)));
        assert!(!ui.dismiss());
        assert!(ui.banner().is_some());
    }
}

#[test]
fn storage_levels_and_sizes() {
    let level = |used| {
        Storage {
            used_bytes: used,
            quota_bytes: 1_000,
        }
        .level()
    };
    assert_eq!(level(899), StorageLevel::Roomy);
    assert_eq!(level(900), StorageLevel::AlmostFull(90));
    assert_eq!(level(950), StorageLevel::AlmostFull(95));
    assert_eq!(level(999), StorageLevel::AlmostFull(99));
    assert_eq!(level(1_000), StorageLevel::Full);
    assert_eq!(
        Storage {
            used_bytes: 5,
            quota_bytes: 0
        }
        .level(),
        StorageLevel::Roomy
    );
    assert_eq!(size_words(950_000_000_000), "950 GB");
    assert_eq!(size_words(1_000_000_000_000), "1 TB");
    assert_eq!(size_words(1_500_000_000_000), "1.5 TB");
    assert_eq!(size_words(0), "0 B");
}

/// The loading strip: a progress bar with its value, and the rows under it still work.
#[test]
fn the_loading_strip_is_a_progress_bar_over_a_list_that_works() {
    let mut rt = scene("DW-0118");
    let bar = info(&rt, "strip-bar");
    assert_eq!(bar.role, "progressbar");
    assert_eq!(bar.label.as_deref(), Some("Getting your files ready"));
    let (fill, track) = (info(&rt, "strip-bar-fill").rect[2], bar.rect[2]);
    assert!((fill / track - 0.45).abs() < 0.001, "45 %: {fill} of {track}");
    // "The rows already listed, usable": a click selects one.
    let r = info(&rt, "row-3").rect;
    rt.pointer_down(r[0] + 40.0, r[1] + r[3] / 2.0);
    rt.pointer_up(r[0] + 40.0, r[1] + r[3] / 2.0);
    settle(&mut rt, 100.0);
    assert_eq!(rt.state.selected, Some(3));
    // A banner goes before the strip.
    let both = GlobalStatus {
        sync_problems: 1,
        ..board::loading()
    };
    let rt = window(both);
    assert!(find(&rt, "strip-bar").is_none());
    assert!(find(&rt, "banner-action").is_some());
}

/// DW-0114: a new account. Nothing is counted, and Add files opens the New menu (DW-0414).
#[test]
fn a_new_account_has_one_way_forward() {
    let mut rt = scene("DW-0114");
    assert_eq!(info(&rt, "empty").label.as_deref(), Some("Nothing here yet"));
    assert_eq!(
        info(&rt, "status-details").text.as_deref(),
        Some("Nothing on this PC yet")
    );
    assert_eq!(info(&rt, "storage-value").text.as_deref(), Some("0 B of 1 TB"));
    assert!(find(&rt, "Shared-badge").is_none() && find(&rt, "Trash-badge").is_none());
    tab_to(&mut rt, "empty-add");
    press(&mut rt, Key::Text(" ".into()));
    assert!(rt.state.actions.menu.is_some(), "Add files opens the New menu");
}

fn blocked(back_online: bool) -> (Runtime<State>, Arc<AtomicU32>) {
    let mut state = init();
    let (_, apply) = FRAMES.iter().find(|(id, _)| *id == "DW-0121").unwrap();
    apply(&mut state);
    let fixture = GlobalFixture {
        back_online,
        ..GlobalFixture::of(board::offline())
    };
    let checks = fixture.checks.clone();
    state.global.source = Box::new(fixture);
    (runtime_of(state), checks)
}

/// DW-0121: the panel is a dialog named after the file, says what is needed, and the status line
/// and the row say the same.
#[test]
fn a_preview_that_needs_a_connection_says_so() {
    let (rt, _) = blocked(false);
    let card = info(&rt, "preview-card");
    assert_eq!(card.role, "dialog");
    assert_eq!(card.label.as_deref(), Some("Contract draft.pdf: Needs a connection"));
    assert_eq!(
        info(&rt, "blocked-text").text.as_deref(),
        Some("Contract draft.pdf isn't on this PC yet. Connect to open it.")
    );
    assert_eq!(info(&rt, "status-text").text.as_deref(), Some("Offline"));
    assert_eq!(info(&rt, "row-6-date").text.as_deref(), Some("Needs a connection"));
    assert_eq!(info(&rt, "row-5-date").text.as_deref(), Some("Today, 09:12"));
    let close = info(&rt, "preview-close");
    assert_eq!((close.role, close.label.as_deref()), ("button", Some("Close")));
    // The keyboard starts on the one way forward.
    assert_eq!(focused(&rt).as_deref(), Some("blocked-retry"));
}

/// Try again checks the connection: still none, the panel stays; back, it closes and the file is
/// asked for. Esc closes it.
#[test]
fn try_again_checks_the_connection_and_esc_closes() {
    let (mut rt, checks) = blocked(false);
    press(&mut rt, Key::Enter);
    assert_eq!(checks.load(Ordering::SeqCst), 1);
    assert!(find(&rt, "preview-card").is_some(), "no connection: it stays");
    assert!(rt.state.global.intents.is_empty());
    press(&mut rt, Key::Escape);
    assert!(find(&rt, "preview-card").is_none());
    assert!(find(&rt, "banner").is_some(), "the offline banner is still there");
    assert_ne!(info(&rt, "row-6-date").text.as_deref(), Some("Needs a connection"));

    let (mut rt, checks) = blocked(true);
    press(&mut rt, Key::Enter);
    assert_eq!(checks.load(Ordering::SeqCst), 1);
    assert!(find(&rt, "preview-card").is_none());
    assert_eq!(rt.state.global.intents, vec![Intent::OpenPreview("contract".into())]);
    assert!(find(&rt, "banner").is_none(), "the connection is back");
}

/// The window behind the panel is inert: Tab stays inside it.
#[test]
fn the_panel_keeps_the_keyboard() {
    let (mut rt, _) = blocked(false);
    for _ in 0..6 {
        press(&mut rt, Key::Tab);
        let at = focused(&rt).unwrap_or_default();
        assert!(
            at == "blocked-retry" || at == "preview-close",
            "Tab left the panel for {at}"
        );
    }
}

/// Every frame of the group that is built has a scene, and none panics in either theme.
#[test]
fn every_built_frame_draws_in_both_themes() {
    for (frame, apply) in FRAMES {
        for dark in [false, true] {
            let mut state = init();
            apply(&mut state);
            let env = Env {
                system_dark: dark,
                scale: 2.0,
            };
            let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
            rt.frame(0.0);
            assert!(find(&rt, "window").is_some(), "{frame}");
        }
    }
    assert_eq!(FRAMES.len(), 9);
}
