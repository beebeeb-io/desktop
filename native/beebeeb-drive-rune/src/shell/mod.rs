//! The app shell: lifetime, windows, the b in the notification area, single instance, start at
//! login, deep links and routing. `README.md` in this directory is the guide for other lanes.
//!
//! | file | what |
//! |---|---|
//! | `app.rs` | the route machine: sections, settings, dialogs, history, the session gate |
//! | `registry.rs` | which frame group draws which state: ONE line per group |
//! | `placeholder.rs` | the "not built yet" frame for a group with no line |
//! | `host.rs` | the running shell in the root state, and the hooks Rune calls |
//! | `actions.rs` | `ShellActions` / `EngineActions` and the fixture engine |
//! | `surfaces.rs`, `window_state.rs` | the two windows, and where the main one was |
//! | `tray.rs`, `tray_icon.rs` | the b: its model and its pixels |
//! | `flyout.rs`, `flyout_window.rs` | the flyout: where it opens, and its window |
//! | `instance.rs`, `deeplink.rs`, `autostart.rs` | starting up, links, start at login |
//! | `platform/` | the OS seam: `windows` real, `macos` and `linux` stubs |

pub mod actions;
pub mod app;
// The pure half of Windows-only code: used by `platform/windows.rs`, tested on every platform.
#[cfg_attr(not(windows), allow(dead_code))]
pub mod autostart;
pub mod deeplink;
pub mod files_frame;
pub mod flyout;
pub mod flyout_window;
pub mod host;
pub mod instance;
pub mod placeholder;
pub mod platform;
pub mod registry;
pub mod state;
pub mod surfaces;
pub mod tray;
// Rendered by `platform/windows.rs`; tested (and looked at) on every platform.
#[cfg_attr(not(windows), allow(dead_code))]
pub mod tray_icon;
pub mod window_state;

use std::sync::{Arc, Mutex};

use rune::{App, AppSpec, Titlebar, WindowSpec};
use surfaces::WindowKind;

use crate::source::{DriveSource, LargeFixtureSource};
use actions::{EngineActions, FixtureEngine, HostActions, ShellActions};
use app::{Dialog, Event, GateState, Route, Section, SettingsPane};
use flyout_window::FlyoutState;
use host::{Shell, Wiring};
use instance::Start;
use state::State;

/// Titlebar height in points — the single source of truth used by both
/// `App::titlebar` and `ui::caption::caption_bar`.
/// Board: brand/caption area is 33px (sidebar starts at y=33, toolbar at y=33).
pub const TITLEBAR_H: f32 = 33.0;

/// Inter (sans) + JetBrains Mono (mono), bundled as TTF.
pub static FONTS: &[&[u8]] = &[
    include_bytes!("../../assets/fonts/inter-latin-400-normal.ttf"),
    include_bytes!("../../assets/fonts/inter-latin-500-normal.ttf"),
    include_bytes!("../../assets/fonts/inter-latin-600-normal.ttf"),
    include_bytes!("../../assets/fonts/inter-latin-700-normal.ttf"),
    include_bytes!("../../assets/fonts/jetbrains-mono-latin-400-normal.ttf"),
    include_bytes!("../../assets/fonts/jetbrains-mono-latin-500-normal.ttf"),
    include_bytes!("../../assets/fonts/jetbrains-mono-latin-600-normal.ttf"),
];
const FAMILIES: (&str, &str) = ("Inter", "JetBrains Mono");

/// Named states for headless renders (`--scene NAME`). The first three are DW-0401's; the
/// rest put the route machine somewhere, so a frame lane can render its group before any
/// navigation exists.
const SCENES: &[(&str, fn(&mut State))] = &[
    ("light", |_| {}),
    ("dark", |_| {}),
    ("large-folder", |s| {
        let source = Box::new(LargeFixtureSource::new(5000));
        s.listing = source.list(None);
        s.source = source;
        s.selected = None;
        s.show_detail_pane = false;
    }),
    ("shared", |s| scene(s, Event::Navigate(Route::section(Section::Shared)))),
    ("photos", |s| scene(s, Event::Navigate(Route::section(Section::Photos)))),
    ("backups", |s| {
        scene(s, Event::Navigate(Route::section(Section::Backups)))
    }),
    ("trash", |s| scene(s, Event::Navigate(Route::section(Section::Trash)))),
    ("sync-history", |s| {
        scene(s, Event::Navigate(Route::section(Section::SyncHistory)))
    }),
    ("available-offline", |s| {
        scene(s, Event::Navigate(Route::section(Section::AvailableOffline)))
    }),
    ("settings", |s| scene(s, Event::OpenSettings(SettingsPane::General))),
    ("shortcuts", |s| scene(s, Event::OpenDialog(Dialog::Shortcuts))),
    ("signed-out", |s| scene(s, Event::Gate(GateState::SignedOut))),
    ("locked", |s| scene(s, Event::Gate(GateState::Locked))),
    ("no-plan", |s| scene(s, Event::Gate(GateState::NoPlan))),
];

fn scene(s: &mut State, event: Event) {
    host::dispatch(s, event);
}

/// Starts the app and returns when it quits.
///
/// In order: read the launch arguments; unless this is a Rune tooling run (`--snap`, …), make
/// this the one running instance (a later launch hands over its arguments and exits) and
/// refuse to start beside another agent that holds the profile's sync lock; then run the two
/// windows, in the notification area when Windows started the app at login.
pub fn run() {
    let launch = instance::parse_args(std::env::args().skip(1));
    let platform = platform::current();
    // The engine lane replaces this with the real engine; nothing else in `run` changes.
    let engine: Arc<dyn EngineActions> = Arc::new(FixtureEngine::ready());
    let first_instance = launch.tooling || rune::single_instance(instance::APP_ID);
    let holder = if first_instance && !launch.tooling {
        engine.profile_holder()
    } else {
        None
    };
    let (tooling, show_window) = match instance::decide(&launch, first_instance, holder) {
        Start::Forwarded => return,
        Start::HeldBy(holder) => {
            platform.show_start_refusal(&instance::held_message(&holder));
            return;
        }
        Start::Tooling => (true, true),
        Start::Run { show_window } => (false, show_window),
    };
    let actions: Arc<dyn ShellActions> = Arc::new(HostActions::new(engine, platform.clone()));

    // Where the main window was. A tooling run reads and writes nothing.
    let state_file = if tooling {
        None
    } else {
        rune::dirs::AppDirs::new(instance::APP_ID).map(|dirs| dirs.state().join(window_state::FILE_NAME))
    };
    let saved = state_file.as_deref().map(window_state::load).unwrap_or_default();
    let saved_main = saved.get(WindowKind::Main.key()).copied();

    let main_spec = AppSpec::<State> {
        fonts: FONTS,
        families: FAMILIES,
        scenes: SCENES,
        min_size: Some(WindowKind::Main.min_size()),
        poll: Some(host::poll),
        on_key: Some(host::on_key),
        ..AppSpec::new(
            "Beebeeb Drive",
            WindowKind::Main.default_size(),
            state::init,
            registry::view,
        )
    };
    let flyout_spec = AppSpec::<FlyoutState> {
        fonts: FONTS,
        families: FAMILIES,
        min_size: Some(WindowKind::Flyout.min_size()),
        on_key: Some(flyout_window::on_key),
        ..AppSpec::new(
            "Beebeeb",
            WindowKind::Flyout.default_size(),
            flyout_window::fixture_state,
            flyout_window::view,
        )
    };

    let window = Arc::new(Mutex::new(None));
    let window_slot = window.clone();
    let mut main_app = App::new(main_spec)
        .titlebar(Titlebar::new(TITLEBAR_H))
        .on_open_url(host::on_open_url)
        .on_reopen(host::on_reopen)
        .on_window(move |native| {
            *window_slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(native.clone())
        });
    let flyout_platform = platform.clone();
    let mut flyout_app = App::new(flyout_spec)
        .on_focus(flyout_window::on_focus)
        .on_window(move |native| flyout_platform.adopt_flyout_window(native));
    if !tooling {
        // The app outlives its window: closing it leaves the b running (DW-1231).
        main_app = main_app.keep_running().on_close(host::on_close).on_quit(host::on_quit);
        flyout_app = flyout_app.keep_running();
        if !show_window {
            main_app = main_app.windowless();
        }
    }
    let (main, flyout_handle) = (main_app.handle(), flyout_app.handle());

    let flyout_hidden_at = Arc::new(Mutex::new(None));
    let wiring = Wiring {
        main: Some(main.clone()),
        flyout: Some(flyout_handle),
        window,
        flyout_hidden_at: flyout_hidden_at.clone(),
        saved,
        state_file,
    };
    let (shell_actions, shell_platform) = (actions.clone(), platform.clone());
    let main_app = main_app.init(move || state::init_with(Shell::new(shell_actions, shell_platform, wiring)));
    let flyout_main = main.clone();
    let flyout_app = flyout_app.init(move || FlyoutState::new(actions, Some(flyout_main), flyout_hidden_at));

    if !tooling {
        match platform.adopt_autostart() {
            Ok(true) => host::log("start at login: the earlier app's entry now points at this one"),
            Ok(false) => {}
            Err(error) => host::log(format!("start at login was not checked ({error})")),
        }
        // Runs on the UI thread once the event loop is up, window or not.
        main.update(|s, _| s.shell.start_tray());
    }

    rune::run_windows(move |displays| {
        let restored = window_state::restore(
            saved_main.as_ref(),
            &surfaces::monitors(displays),
            surfaces::MAIN_MIN_SIZE,
        );
        let main_app = main_app.maximized(restored.maximized);
        vec![
            WindowSpec::from_app(WindowKind::Main.key(), main_app)
                .options(surfaces::main_options(&restored))
                .boxed(),
            WindowSpec::from_app(WindowKind::Flyout.key(), flyout_app)
                .options(surfaces::flyout_options())
                .boxed(),
        ]
    });
}
