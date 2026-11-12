//! Tests of the Bandwidth pane (DW-0785 and its states) and the speed test (DW-0786 to d):
//! that each state is reached through the pane's own controls, what the controls do, the
//! names for assistive technology, and that the pane is the whole pane outside a scene.
//!
//! Input is scripted: keys and presses go through Rune's `Runtime`, as a person's would.

use super::bandwidth::Crop;
use super::bandwidth_data::{Bandwidth, BoardBandwidth, Direction, History, STEP_SECONDS, SpeedTest};
use super::drive::{BoardDrive, DriveSettings, SyncStatus, UnwiredDrive};
use super::{MenuKind, Pane};
use crate::shell::app::{Screen, SettingsPane};
use crate::shell::host;
use crate::{State, init, view};
use rune::{Env, Key, KeyEvent, Mods, Runtime};

/// The frame `frame`, in a window tall enough to show the whole pane.
fn runtime(frame: &str) -> Runtime<State> {
    let mut state = init();
    assert!(super::scene(frame, &mut state), "{frame} is a built frame");
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (958.0, 1100.0), env);
    rt.on_key = Some(host::on_key);
    rt.poll = Some(host::poll);
    rt.frame(0.0);
    settle(&mut rt);
    rt
}

/// The app's own Bandwidth pane, opened from the settings sidebar: never a scene's crop.
fn opened() -> Runtime<State> {
    let mut rt = runtime("DW-0701");
    click(&mut rt, "Bandwidth-row");
    rt
}

fn settle(rt: &mut Runtime<State>) {
    rt.frame(0.016);
    rt.settle_all();
    rt.frame(0.016);
}

fn find(rt: &Runtime<State>, key: &str) -> Option<rune::ElementInfo> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

fn info(rt: &Runtime<State>, key: &str) -> rune::ElementInfo {
    find(rt, key).unwrap_or_else(|| panic!("no element keyed {key}"))
}

fn text(rt: &Runtime<State>, key: &str) -> String {
    info(rt, key).text.unwrap_or_default()
}

fn key(rt: &mut Runtime<State>, key: Key) {
    rt.key(KeyEvent {
        key,
        mods: Mods::default(),
    });
    settle(rt);
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let r = info(rt, key).rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    settle(rt);
}

/// Tab to the control keyed `target`, and press it with Enter.
fn press(rt: &mut Runtime<State>, target: &str) {
    for _ in 0..80 {
        if find(rt, target).is_some_and(|e| e.focused) {
            key(rt, Key::Enter);
            return;
        }
        key(rt, Key::Tab);
    }
    panic!("Tab never reaches {target}");
}

/// Let `seconds` pass on the window's clock (the shell's poll runs at the start of each frame).
fn wait(rt: &mut Runtime<State>, seconds: f64) {
    rt.frame(seconds);
    settle(rt);
}

fn bandwidth(rt: &Runtime<State>) -> Bandwidth {
    rt.state.settings.drive.bandwidth()
}

/// What the scene of `frame` leaves the source saying.
fn scene_says(frame: &str) -> Bandwidth {
    let mut state = init();
    assert!(super::scene(frame, &mut state), "{frame} is a built frame");
    state.settings.drive.bandwidth()
}

#[test]
fn the_sidebar_opens_the_whole_pane_on_the_shells_bandwidth_section() {
    let rt = opened();
    assert_eq!(rt.state.shell.app.screen(), Screen::Settings(SettingsPane::Bandwidth));
    assert_eq!(rt.state.settings.pane, Pane::Bandwidth);
    assert_eq!(text(&rt, "set-title"), "Bandwidth");
    assert!(
        info(&rt, "Bandwidth-row").fill.a > 0.0,
        "the Bandwidth item is the selected one"
    );
    assert!(rt.state.settings.intents.is_empty(), "nothing is asked of the shell");
    // The app draws every part, in the board's order: no crop outside a scene.
    assert_eq!(rt.state.settings.bandwidth.crop, Crop::Whole);
    for part in [
        "card-0",
        "card-1",
        "chart-plot",
        "device-2",
        "item-1-2-switch",
        "btn-0",
        "list-2-foot",
    ] {
        assert!(find(&rt, part).is_some(), "{part} is drawn");
    }
    assert_eq!(text(&rt, "list-0-title"), "Your devices right now");
    assert_eq!(text(&rt, "list-1-title"), "Limits");
    assert_eq!(text(&rt, "list-2-title"), "Speed test");
    assert_eq!(
        bandwidth(&rt),
        scene_says("DW-0785"),
        "the pane opens on DW-0785's state"
    );
    // The slug round-trips, so a link can name the section.
    assert_eq!(SettingsPane::from_slug("bandwidth"), Some(SettingsPane::Bandwidth));
}

#[test]
fn figures_carry_their_unit_and_say_it_in_full() {
    let rt = opened();
    assert_eq!(text(&rt, "fig-0"), "3.2");
    assert_eq!(text(&rt, "fig-0-unit"), "MB/s");
    assert_eq!(text(&rt, "fig-1"), "410");
    assert_eq!(text(&rt, "fig-1-unit"), "KB/s");
    assert_eq!(text(&rt, "card-0-now"), "2 of 3 devices syncing");
    assert_eq!(
        info(&rt, "fig-0-tile").label.as_deref(),
        Some("Upload, 3.2 megabytes per second. Encrypting and uploading 12 files \u{b7} 48\u{a0}MB left")
    );
    assert_eq!(
        info(&rt, "fig-1-tile").label.as_deref(),
        Some("Download, 410 kilobytes per second. 2 files from Anna's iPhone")
    );
    // The chart has a text alternative: the totals.
    let chart = info(&rt, "chart");
    assert_eq!(chart.role, "img");
    assert_eq!(
        chart.label.as_deref(),
        Some("Upload and download over the last 20 hours: upload 5.2 GB, download 1.3 GB")
    );
    assert_eq!(text(&rt, "legend-up-name"), "Upload");
    assert_eq!(text(&rt, "legend-up-total"), "5.2 GB");
    assert_eq!(text(&rt, "legend-down-name"), "Download");
    assert_eq!(text(&rt, "legend-down-total"), "1.3 GB");
}

#[test]
fn an_upload_limit_is_chosen_from_its_menu_by_keyboard_and_reaches_dw_0785c() {
    let mut rt = opened();
    let row = info(&rt, "item-1-0");
    assert_eq!(row.role, "combobox");
    assert_eq!(row.label.as_deref(), Some("Upload, No limit"));
    press(&mut rt, "item-1-0");
    // DW-0785d: the menu, on the current choice.
    let upload = MenuKind::SpeedLimit(Direction::Upload);
    assert_eq!(rt.state.settings.menu.map(|m| (m.kind, m.hot)), Some((upload, 0)));
    assert_eq!(info(&rt, "menu-0").label.as_deref(), Some("Upload limit"));
    for (i, label) in [
        "No limit", "1 Mbps", "2 Mbps", "5 Mbps", "10 Mbps", "20 Mbps", "50 Mbps", "100 Mbps",
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(text(&rt, &format!("menu-0-item-{i}-label")), label);
    }
    assert!(find(&rt, "menu-0-item-8").is_none(), "eight choices");
    assert!(find(&rt, "menu-0-item-0-check").is_some(), "No limit is checked");
    assert!(find(&rt, "menu-0-item-1-check").is_none());
    // Esc closes it and nothing changes.
    key(&mut rt, Key::Escape);
    assert_eq!(rt.state.settings.menu, None);
    assert_eq!(bandwidth(&rt).upload_limit, None);
    assert_eq!(
        rt.state.settings.pane,
        Pane::Bandwidth,
        "Esc closed the menu, not settings"
    );
    // Down, Enter: 1 Mbps applies straight away.
    press(&mut rt, "item-1-0");
    key(&mut rt, Key::Down);
    key(&mut rt, Key::Enter);
    assert_eq!(rt.state.settings.menu, None);
    assert_eq!(
        bandwidth(&rt),
        scene_says("DW-0785c"),
        "the controls reach DW-0785c's state"
    );
    assert_eq!(text(&rt, "item-1-0-pop-label"), "1 Mbps");
    assert_eq!(
        text(&rt, "item-1-0-sub"),
        "Sync takes longer while this is lower than your connection"
    );
    assert_eq!(text(&rt, "fig-0"), "125");
    assert_eq!(text(&rt, "fig-0-unit"), "KB/s");
    assert_eq!(text(&rt, "fig-0-says"), "Limited to 1 Mbps \u{b7} 12 files, 48 MB left");
    assert!(
        text(&rt, "device-0-says").contains("125 KB/s"),
        "this PC's row is at the limit"
    );
    assert!(find(&rt, "item-1-1-sub").is_none(), "Download has no limit");
    // The chart flattens at the limit: 150 MB per 20 minutes since 07:00.
    let h = bandwidth(&rt).history;
    assert!(h.up_mb[52..].iter().all(|mb| *mb <= 150.0));
    assert!(
        (h.up_mb[51] - 340.0).abs() < 0.01,
        "what was sent before the limit stays"
    );
    // The menu now checks 1 Mbps.
    press(&mut rt, "item-1-0");
    assert!(find(&rt, "menu-0-item-1-check").is_some());
    assert!(find(&rt, "menu-0-item-0-check").is_none());
}

#[test]
fn the_download_limit_has_the_same_choices_and_is_chosen_by_pointer() {
    let mut rt = opened();
    click(&mut rt, "item-1-1");
    let download = MenuKind::SpeedLimit(Direction::Download);
    assert_eq!(rt.state.settings.menu.map(|m| m.kind), Some(download));
    assert_eq!(info(&rt, "menu-0").label.as_deref(), Some("Download limit"));
    click(&mut rt, "menu-0-item-3");
    assert_eq!(bandwidth(&rt).download_limit, Some(5));
    assert_eq!(bandwidth(&rt).upload_limit, None, "the other direction is untouched");
    assert_eq!(text(&rt, "item-1-1-pop-label"), "5 Mbps");
    assert_eq!(rt.state.settings.menu, None);
    // Back to No limit.
    click(&mut rt, "item-1-1");
    click(&mut rt, "menu-0-item-0");
    assert_eq!(bandwidth(&rt).download_limit, None);
}

#[test]
fn a_saved_limit_that_is_no_preset_is_its_own_choice_and_enter_keeps_it() {
    let mut rt = opened();
    rt.state.settings.drive.set_speed_limit(Direction::Download, Some(3));
    settle(&mut rt);
    assert_eq!(text(&rt, "item-1-1-pop-label"), "3 Mbps");
    // The menu opens ON the saved value: a ninth item of its own, the checked one.
    press(&mut rt, "item-1-1");
    let download = MenuKind::SpeedLimit(Direction::Download);
    assert_eq!(rt.state.settings.menu.map(|m| (m.kind, m.hot)), Some((download, 8)));
    assert_eq!(text(&rt, "menu-0-item-8-label"), "3 Mbps");
    assert!(find(&rt, "menu-0-item-8-check").is_some());
    for i in 0..8 {
        assert!(
            find(&rt, &format!("menu-0-item-{i}-check")).is_none(),
            "preset {i} is not checked"
        );
    }
    // Enter without a choice changes nothing: the saved limit is not lost.
    key(&mut rt, Key::Enter);
    assert_eq!(rt.state.settings.menu, None);
    assert_eq!(bandwidth(&rt).download_limit, Some(3));
    // By pointer on its own item: the same.
    click(&mut rt, "item-1-1");
    click(&mut rt, "menu-0-item-8");
    assert_eq!(bandwidth(&rt).download_limit, Some(3));
    // Up from it is the last preset; once a preset is chosen the ninth item is gone.
    press(&mut rt, "item-1-1");
    key(&mut rt, Key::Up);
    key(&mut rt, Key::Enter);
    assert_eq!(bandwidth(&rt).download_limit, Some(100));
    click(&mut rt, "item-1-1");
    assert!(find(&rt, "menu-0-item-8").is_none());
}

#[test]
fn nothing_moving_is_a_dash_never_a_zero() {
    for frame in ["DW-0785b", "DW-0785e", "DW-0785f"] {
        let rt = runtime(frame);
        for fig in ["fig-0", "fig-1"] {
            assert_eq!(text(&rt, fig), "\u{2014}", "{frame} {fig}");
            assert!(
                find(&rt, &format!("{fig}-unit")).is_none(),
                "{frame}: a dash has no unit"
            );
        }
    }
    let rt = runtime("DW-0785b");
    assert_eq!(text(&rt, "card-0-now"), "Nothing is moving");
    assert_eq!(text(&rt, "fig-0-says"), "Up to date \u{b7} last change 2 min ago");
    assert_eq!(text(&rt, "fig-1-says"), "Files download when you open them");
    assert!(find(&rt, "chart-plot").is_some(), "the history is still drawn");
    assert!(
        info(&rt, "fig-0-tile")
            .label
            .unwrap_or_default()
            .starts_with("Upload, nothing is moving.")
    );
}

#[test]
fn a_new_pc_says_when_recording_starts_instead_of_an_empty_chart() {
    let rt = runtime("DW-0785e");
    assert_eq!(text(&rt, "card-1-sub"), "Nothing recorded yet.");
    assert_eq!(
        text(&rt, "card-1-empty"),
        "Beebeeb records traffic from when it starts syncing on this PC."
    );
    for gone in ["chart-plot", "legend", "chart-x"] {
        assert!(find(&rt, gone).is_none(), "{gone} is not drawn without a history");
    }
    assert!(bandwidth(&rt).history.is_empty());
}

#[test]
fn the_metered_wait_is_reached_from_sync_and_the_switch_resumes_sync_at_once() {
    // DW-0746e: sync waits on a metered connection. Its sidebar leads here.
    let mut rt = runtime("DW-0746e");
    click(&mut rt, "Bandwidth-row");
    assert_eq!(
        bandwidth(&rt),
        scene_says("DW-0785f"),
        "the app reaches DW-0785f's state"
    );
    assert_eq!(rt.state.settings.bandwidth.crop, Crop::Whole);
    assert_eq!(text(&rt, "card-0-now"), "Waiting for another network");
    assert_eq!(
        text(&rt, "fig-0-says"),
        "Waiting: this PC is on a metered connection \u{b7} 7 changes wait"
    );
    assert_eq!(text(&rt, "fig-1-says"), "Files you open still download");
    assert_eq!(
        text(&rt, "item-1-2-sub"),
        "Off: sync waits on networks Windows marks as metered"
    );
    let switch = info(&rt, "item-1-2-switch");
    assert_eq!(switch.role, "switch");
    assert_eq!(switch.label.as_deref(), Some("Sync on metered connections"));
    // The last hour sent nothing; downloads are still drawn.
    let h = bandwidth(&rt).history;
    assert!(h.up_mb[57..].iter().all(|mb| *mb == 0.0));
    assert!(h.down_mb[57..].iter().all(|mb| *mb > 0.0));
    // On: sync resumes at once, here and on Sync and File Explorer.
    press(&mut rt, "item-1-2-switch");
    let now = bandwidth(&rt);
    assert!(now.sync_on_metered && !now.waiting_metered);
    assert_eq!(now.upload.speed, Some(("3.2".into(), "MB/s".into())));
    assert_eq!(text(&rt, "card-0-now"), "2 of 3 devices syncing");
    assert_eq!(
        text(&rt, "item-1-2-sub"),
        "A network Windows marks as metered, such as a phone's hotspot"
    );
    assert!(matches!(rt.state.settings.drive.sync().status, SyncStatus::Running(_)));
    // Off again: it waits again.
    click(&mut rt, "item-1-2-switch");
    assert!(bandwidth(&rt).waiting_metered);
    assert!(matches!(
        rt.state.settings.drive.sync().status,
        SyncStatus::WaitingMetered(_)
    ));
}

#[test]
fn sync_anyway_ends_the_wait_and_leaves_the_setting_off() {
    let mut drive = BoardDrive::new().with_metered_wait();
    drive.sync_anyway();
    let now = drive.bandwidth();
    assert!(!now.waiting_metered, "sync runs on this connection");
    assert!(!now.sync_on_metered, "the setting stays off");
}

#[test]
fn the_metered_frame_draws_the_head_and_the_limits_only() {
    let rt = runtime("DW-0785f");
    assert_eq!(rt.state.settings.bandwidth.crop, Crop::HeadAndLimits);
    assert_eq!(text(&rt, "list-0-title"), "Limits");
    assert!(find(&rt, "item-0-2-switch").is_some());
    assert!(find(&rt, "device-0").is_none() && find(&rt, "btn-0").is_none());
}

#[test]
fn run_test_shows_each_step_and_one_test_runs_at_a_time() {
    let mut rt = opened();
    assert_eq!(bandwidth(&rt).test, SpeedTest::Idle);
    assert_eq!(text(&rt, "btn-0-label"), "Run test");
    assert!(find(&rt, "steps").is_none());
    press(&mut rt, "btn-0");
    // DW-0786: latency done, the second of three uploads, the downloads wait.
    assert_eq!(
        bandwidth(&rt),
        scene_says("DW-0786"),
        "Run test reaches DW-0786's state"
    );
    assert_eq!(text(&rt, "btn-0-label"), "Running\u{2026}");
    assert_eq!(
        text(&rt, "item-2-0-sub"),
        "Testing\u{2026} this takes 30 to 90 seconds."
    );
    assert_eq!(text(&rt, "step-0-value"), "24");
    assert_eq!(text(&rt, "step-0-value-unit"), "ms");
    assert_eq!(text(&rt, "step-0-says"), "18 to 41 ms");
    assert_eq!(text(&rt, "step-1-value"), "2 of 3");
    assert_eq!(text(&rt, "step-1-measuring"), "measuring");
    assert_eq!(text(&rt, "step-2-value"), "\u{2014}");
    assert_eq!(text(&rt, "step-2-says"), "Waits for the uploads");
    assert_eq!(info(&rt, "step-0").label.as_deref(), Some("Latency, 24 milliseconds"));
    assert_eq!(info(&rt, "step-1").label.as_deref(), Some("Upload, measuring, 2 of 3"));
    // The upload bar is Amber (upload is the encryption step).
    let amber = crate::theme::tokens::theme(false).accent;
    assert_eq!(info(&rt, "step-1-bar-fill").fill, amber);
    assert!(find(&rt, "step-2-bar").is_none());
    // Before the step is over nothing moves.
    wait(&mut rt, STEP_SECONDS - 0.5);
    assert_eq!(bandwidth(&rt), scene_says("DW-0786"));
    // DW-0786b: the downloads, by themselves, with no press.
    wait(&mut rt, 0.6);
    assert_eq!(
        bandwidth(&rt),
        scene_says("DW-0786b"),
        "the test moves on to DW-0786b's state"
    );
    // One test at a time: a press on the button while it runs starts nothing.
    click(&mut rt, "btn-0");
    assert_eq!(
        bandwidth(&rt),
        scene_says("DW-0786b"),
        "a press while it runs starts nothing"
    );
    assert_eq!(text(&rt, "step-1-value"), "35.2");
    assert_eq!(text(&rt, "step-1-value-unit"), "Mbps");
    assert_eq!(text(&rt, "step-2-value"), "1 of 3");
    assert!(find(&rt, "step-1-check").is_some(), "the uploads are done");
    assert_ne!(info(&rt, "step-2-bar-fill").fill, amber, "the download bar is grey");
}

#[test]
fn leaving_settings_does_not_stop_the_test_and_the_result_stays() {
    let mut rt = opened();
    press(&mut rt, "btn-0");
    // Leave for General, then for Drive: the test goes on to its next step out there.
    click(&mut rt, "General-row");
    assert_eq!(rt.state.settings.pane, Pane::General);
    key(&mut rt, Key::Escape);
    assert!(!matches!(rt.state.shell.app.screen(), Screen::Settings(_)));
    wait(&mut rt, STEP_SECONDS + 0.1);
    assert_eq!(
        bandwidth(&rt),
        scene_says("DW-0786b"),
        "the test keeps running outside settings"
    );
    host::dispatch(
        &mut rt.state,
        crate::shell::app::Event::OpenSettings(SettingsPane::Bandwidth),
    );
    settle(&mut rt);
    assert_eq!(text(&rt, "btn-0-label"), "Running\u{2026}");
    // DW-0786c: the result, with Run test again.
    wait(&mut rt, STEP_SECONDS);
    assert_eq!(
        bandwidth(&rt),
        scene_says("DW-0786c"),
        "the test ends on DW-0786c's state"
    );
    assert_eq!(text(&rt, "btn-0-label"), "Run test");
    assert_eq!(text(&rt, "step-2-value"), "94.0");
    assert_eq!(text(&rt, "step-2-value-unit"), "Mbps");
    assert_eq!(
        info(&rt, "step-2").label.as_deref(),
        Some("Download, 94.0 megabits per second")
    );
    assert_eq!(
        text(&rt, "list-2-foot"),
        "Measured just now, from this PC to Beebeeb's servers in Europe. The test moved 60 MB."
    );
    for i in 0..3 {
        assert!(
            find(&rt, &format!("step-{i}-check")).is_some(),
            "step {i} has its check"
        );
    }
    click(&mut rt, "General-row");
    click(&mut rt, "Bandwidth-row");
    wait(&mut rt, 10.0);
    assert_eq!(
        text(&rt, "step-2-value"),
        "94.0",
        "the figures stay until the next test"
    );
    // Run test runs it again.
    press(&mut rt, "btn-0");
    assert_eq!(bandwidth(&rt), scene_says("DW-0786"));
}

#[test]
fn a_failed_test_names_the_step_and_run_test_tries_again() {
    // A PC that cannot reach Beebeeb (a state of the source, as a dead connection is).
    let mut rt = opened();
    rt.state.settings.drive = Box::new(BoardDrive::new().with_beebeeb_unreachable());
    settle(&mut rt);
    let before = bandwidth(&rt);
    press(&mut rt, "btn-0");
    assert_eq!(text(&rt, "btn-0-label"), "Running\u{2026}");
    assert_eq!(text(&rt, "step-0-value"), "1 of 5");
    assert_eq!(text(&rt, "step-1-says"), "Waits for the pings");
    // DW-0786d: no ping answered.
    wait(&mut rt, STEP_SECONDS + 0.1);
    assert_eq!(
        bandwidth(&rt),
        scene_says("DW-0786d"),
        "Run test ends on DW-0786d's state"
    );
    assert_eq!(
        text(&rt, "note-0-text"),
        "Couldn't run the speed test. Beebeeb didn't answer any of the 5 pings. Check your internet connection and try again."
    );
    assert_eq!(info(&rt, "note-0").role, "status");
    assert!(
        find(&rt, "steps").is_none(),
        "no figure is shown for a test that failed"
    );
    assert_eq!(text(&rt, "btn-0-label"), "Run test");
    assert_eq!(
        text(&rt, "item-2-0-sub"),
        "5 pings, then 3 uploads and 3 downloads of 10 MB \u{b7} 30 to 90 s"
    );
    // A failed test changes nothing else: sync goes on.
    let after = bandwidth(&rt);
    assert_eq!(
        (after.upload, after.download, after.devices),
        (before.upload, before.download, before.devices)
    );
    press(&mut rt, "btn-0");
    assert!(bandwidth(&rt).test.running());
    assert!(find(&rt, "note-0").is_none());
}

#[test]
fn the_chart_says_the_hovered_bucket_and_forgets_it_on_leaving() {
    // DW-0785: the pointer at 06:20.
    let mut rt = runtime("DW-0785");
    assert_eq!(rt.state.settings.bandwidth.tip, Some(50));
    assert_eq!(text(&rt, "chart-tip-when"), "Today 06:20 \u{2013} 06:40");
    assert_eq!(text(&rt, "chart-tip-moved"), "Upload 240 MB \u{b7} Download 30 MB");
    // Another bucket, by pointer: yesterday evening's upload.
    let r = info(&rt, "chart-at-5").rect;
    rt.pointer_move(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    settle(&mut rt);
    assert_eq!(rt.state.settings.bandwidth.tip, Some(5));
    assert_eq!(text(&rt, "chart-tip-when"), "Yesterday 15:20 \u{2013} 15:40");
    assert_eq!(text(&rt, "chart-tip-moved"), "Upload 310 MB \u{b7} Download 20 MB");
    click(&mut rt, "General-row");
    assert_eq!(rt.state.settings.bandwidth.tip, None);
    // Without a pointer on it, the chart draws no note.
    let rt = opened();
    assert!(find(&rt, "chart-tip").is_none() && find(&rt, "chart-dot-up").is_none());
}

#[test]
fn a_device_row_says_its_state_and_leads_to_devices_and_sessions() {
    let mut rt = opened();
    let row = info(&rt, "device-0");
    assert_eq!(row.role, "link");
    assert_eq!(
        row.label.as_deref(),
        Some("Anna-Laptop, This PC, Syncing. 38 of 50 files \u{b7} 120 of 168 MB \u{b7} 3.2 MB/s \u{b7} 4 s ago")
    );
    assert_eq!(text(&rt, "pill-0-label"), "Syncing");
    assert_eq!(text(&rt, "pill-2-label"), "Paused");
    assert!(
        find(&rt, "device-0-bar").is_some(),
        "a bar while it syncs and says how far"
    );
    assert!(find(&rt, "device-1-bar").is_none() && find(&rt, "device-2-bar").is_none());
    press(&mut rt, "device-1");
    assert_eq!(
        rt.state.shell.app.screen(),
        Screen::Settings(SettingsPane::DevicesAndSessions)
    );
}

#[test]
fn every_control_of_the_pane_takes_the_keyboard_once() {
    let mut rt = opened();
    let mut stops = Vec::new();
    for _ in 0..60 {
        key(&mut rt, Key::Tab);
        let Some(at) = rt
            .elements()
            .into_iter()
            .find(|e| e.focused && !e.exiting)
            .and_then(|e| e.key)
        else {
            continue;
        };
        if stops.contains(&at) {
            break;
        }
        stops.push(at);
    }
    let pane: Vec<&str> = stops
        .iter()
        .map(String::as_str)
        .filter(|k| !k.ends_with("-row") && !k.starts_with("cap-"))
        .collect();
    assert_eq!(
        pane,
        [
            "device-0",
            "device-1",
            "device-2",
            "item-1-0",
            "item-1-1",
            "item-1-2-switch",
            "btn-0"
        ],
        "the Tab order of Bandwidth: top to bottom, nothing twice"
    );
}

#[test]
fn a_source_that_does_not_know_bandwidth_answers_with_the_empty_pane() {
    // An engine-backed source written before this pane keeps compiling and shows no sample.
    let mut unwired = UnwiredDrive::new();
    assert_eq!(unwired.bandwidth(), Bandwidth::default());
    unwired.set_speed_limit(Direction::Upload, Some(1));
    unwired.set_sync_on_metered(true);
    unwired.run_speed_test();
    unwired.speed_test_reported(BoardBandwidth::test_done());
    assert_eq!(unwired.bandwidth(), Bandwidth::default());
    let nothing = Bandwidth::default();
    assert!(nothing.history.is_empty() && nothing.devices.is_empty());
    assert_eq!(nothing.upload.speed, None);
    assert_eq!(nothing.test, SpeedTest::Idle);
}

#[test]
fn the_history_tells_its_times_and_a_limit_above_the_speed_changes_nothing() {
    let h: History = BoardBandwidth::default().bandwidth().history;
    assert_eq!(h.up_mb.len(), 60);
    assert_eq!(h.marks(), ["13:40", "17:40", "21:40", "01:40", "05:40", "Now"]);
    assert_eq!(h.tip(60), None);
    assert_eq!(
        h.tip(59).map(|(when, _)| when).as_deref(),
        Some("Today 09:20 \u{2013} 09:40")
    );
    assert_eq!(
        h.tip(30).map(|(when, _)| when).as_deref(),
        Some("Yesterday 23:40 \u{2013} 00:00")
    );
    // 50 Mbps is above what this PC sends (3.2 MB/s is 25.6 Mbps): nothing is held back.
    let mut board = BoardBandwidth::default();
    board.set_limit(Direction::Upload, Some(50));
    let now = board.bandwidth();
    assert_eq!(now.upload.speed, Some(("3.2".into(), "MB/s".into())));
    assert_eq!(
        now.upload.says,
        "Encrypting and uploading 12 files \u{b7} 48\u{a0}MB left"
    );
    // 2 Mbps is 250 KB/s.
    board.set_limit(Direction::Upload, Some(2));
    assert_eq!(board.bandwidth().upload.speed, Some(("250".into(), "KB/s".into())));
}

#[test]
fn a_second_run_while_the_test_runs_starts_nothing() {
    // The button is off while a test runs; the source holds the same rule for any other caller.
    let mut drive = BoardDrive::new();
    drive.run_speed_test();
    drive.speed_test_reported(BoardBandwidth::test_downloading());
    drive.run_speed_test();
    assert_eq!(drive.bandwidth().test, BoardBandwidth::test_downloading());
    // After the result it runs again, from its first step.
    drive.speed_test_reported(BoardBandwidth::test_done());
    drive.run_speed_test();
    assert_eq!(drive.bandwidth().test, BoardBandwidth::test_uploading());
}

#[test]
fn a_bucket_is_said_by_the_day_it_is_on() {
    // The board's window starts yesterday at 13:40: midnight is between buckets 30 and 31.
    let h: History = BoardBandwidth::default().bandwidth().history;
    assert_eq!(h.starts_days_ago, 1);
    assert_eq!(
        h.tip(30).map(|(when, _)| when).as_deref(),
        Some("Yesterday 23:40 \u{2013} 00:00")
    );
    assert_eq!(
        h.tip(31).map(|(when, _)| when).as_deref(),
        Some("Today 00:00 \u{2013} 00:20")
    );
    // At 23:00 a 20-hour window starts at 03:00 the same day: every bucket is today's.
    let today = History {
        starts_at: 3 * 60,
        starts_days_ago: 0,
        ..h.clone()
    };
    for i in [0, 30, 59] {
        let (when, _) = today.tip(i).expect("a bucket of the window");
        assert!(when.starts_with("Today "), "bucket {i}: {when}");
    }
    assert_eq!(
        today.tip(0).map(|(when, _)| when).as_deref(),
        Some("Today 03:00 \u{2013} 03:20")
    );
    // A window that starts two days back names the day by its distance.
    let older = History {
        starts_days_ago: 2,
        ..h
    };
    assert!(older.tip(0).is_some_and(|(when, _)| when.starts_with("2 days ago ")));
    assert!(older.tip(31).is_some_and(|(when, _)| when.starts_with("Yesterday ")));
}

#[test]
fn a_drive_nothing_answers_for_draws_no_bandwidth_value() {
    let mut rt = runtime("DW-0701");
    rt.state.settings.drive = Box::new(UnwiredDrive::new());
    settle(&mut rt);
    click(&mut rt, "Bandwidth-row");
    assert_eq!(rt.state.shell.app.screen(), Screen::Settings(SettingsPane::Bandwidth));
    for part in ["card-0", "chart-plot", "device-0", "item-1-0", "btn-0", "fig-0"] {
        assert!(find(&rt, part).is_none(), "{part} is not drawn while nothing answers");
    }
}

#[test]
fn the_fixtures_test_stands_still_in_a_scene_and_reports_reset_its_clock() {
    // A board frame's moment does not move on with the clock.
    let mut rt = runtime("DW-0786b");
    wait(&mut rt, 60.0);
    assert_eq!(bandwidth(&rt), scene_says("DW-0786b"));
    // The host's own report wins over the fixture's clock.
    let mut drive = BoardDrive::new();
    drive.run_speed_test();
    assert_eq!(drive.pump(0.0), (false, true), "the first frame starts the clock");
    assert_eq!(drive.pump(STEP_SECONDS), (true, true), "the downloads start");
    assert_eq!(
        drive.pump(STEP_SECONDS + 0.1),
        (false, true),
        "nothing new within a step"
    );
    assert_eq!(
        drive.pump(2.0 * STEP_SECONDS),
        (true, false),
        "the result; nothing more is coming"
    );
    assert_eq!(drive.pump(9.0), (false, false));
    assert_eq!(drive.bandwidth().test, BoardBandwidth::test_done());
}
