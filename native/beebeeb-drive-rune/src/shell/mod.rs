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
//! | `identity.rs` | which app this build is (real or test): every per-app name |
//! | `instance.rs`, `control.rs` | the command line, one instance, the control channel |
//! | `deeplink.rs`, `autostart.rs` | links, start at login |
//! | `platform/` | the OS seam: `windows` real, `macos` and `linux` stubs |

pub mod actions;
pub mod app;
// The pure half of Windows-only code: used by `platform/windows.rs`, tested on every platform.
#[cfg_attr(not(windows), allow(dead_code))]
pub mod autostart;
// The transport is Windows-only (`platform/windows_control.rs`); the protocol is tested everywhere.
#[cfg_attr(not(windows), allow(dead_code))]
pub mod control;
pub mod deeplink;
pub mod files_frame;
pub mod flyout;
pub mod flyout_window;
pub mod host;
pub mod identity;
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
// The pure half of `platform/windows_tray.rs`; tested on every platform.
#[cfg_attr(not(windows), allow(dead_code))]
pub mod tray_win;
pub mod window_state;

use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use rune::{App, AppSpec, Titlebar, WindowSpec};
use surfaces::WindowKind;

use crate::source::{DriveSource, LargeFixtureSource};
use actions::{EngineActions, FixtureEngine, HostActions, ShellActions};
use app::{Dialog, Event, GateState, Route, Section, SettingsPane};
use control::Reply;
use flyout_window::FlyoutState;
use host::{Shell, Wiring};
use identity::AppIdentity;
use instance::{Claim, Plan, Start};
use platform::ServeError;
use state::State;

/// Titlebar height in points — the single source of truth used by both
/// `App::titlebar` and `ui::caption::caption_bar`.
/// Board: `.kit-wincap { height: 32px }`.
pub const TITLEBAR_H: f32 = 32.0;

pub const FAMILIES: (&str, &str) = ("Inter", "JetBrains Mono");
/// Inter (sans) + JetBrains Mono (mono), bundled as TTF.
pub static FONTS: &[&[u8]] = &[
    // Inter, variable (optical size 14-32, weight 100-900): the build the board draws with, so
    // the wordmark's optical size and the 650 weights are the board's.
    include_bytes!("../../assets/fonts/inter-latin-variable-opsz-wght.ttf"),
    include_bytes!("../../assets/fonts/jetbrains-mono-latin-400-normal.ttf"),
    include_bytes!("../../assets/fonts/jetbrains-mono-latin-500-normal.ttf"),
    include_bytes!("../../assets/fonts/jetbrains-mono-latin-600-normal.ttf"),
];

/// Named states for headless renders (`--snap out.png --scene NAME`) and the frame gate
/// (`scripts/compare.sh DW-0401`): one per board frame, by its frame id, and the ones that put
/// the route machine somewhere, so a frame lane can render its group before any navigation
/// exists.
const SCENES: &[crate::ui::gate::Scene<State>] = &[
    ("light", |_| {}),
    ("dark", |_| {}),
    ("large-folder", |s| {
        let source = Box::new(LargeFixtureSource::new(5000));
        (s.listing, s.list_error) = state::load(source.as_ref(), None);
        s.source = source;
        s.selected = None;
        s.show_detail_pane = false;
    }),
    ("DW-0401", |_| {}),
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
    ("first-run", |s| scene(s, Event::Gate(GateState::FirstRun))),
    ("DW-A01", |s| crate::frames::signin::set_scene(s, "DW-A01")),
    ("DW-A02", |s| crate::frames::signin::set_scene(s, "DW-A02")),
    ("DW-A03", |s| crate::frames::signin::set_scene(s, "DW-A03")),
    ("DW-A04", |s| crate::frames::signin::set_scene(s, "DW-A04")),
    ("DW-A05", |s| crate::frames::signin::set_scene(s, "DW-A05")),
    ("DW-A06", |s| crate::frames::signin::set_scene(s, "DW-A06")),
    ("DW-A07", |s| crate::frames::signin::set_scene(s, "DW-A07")),
    ("DW-A08", |s| crate::frames::signin::set_scene(s, "DW-A08")),
    ("DW-A09", |s| crate::frames::signin::set_scene(s, "DW-A09")),
    ("DW-A10", |s| crate::frames::signin::set_scene(s, "DW-A10")),
    ("DW-A18", |s| crate::frames::signin::set_scene(s, "DW-A18")),
    ("DW-A19", |s| crate::frames::signin::set_scene(s, "DW-A19")),
    ("DW-A20", |s| crate::frames::signin::set_scene(s, "DW-A20")),
    ("DW-A21", |s| crate::frames::signin::set_scene(s, "DW-A21")),
    ("DW-A22", |s| crate::frames::signin::set_scene(s, "DW-A22")),
    ("DW-0601", |s| crate::frames::photos::set_scene(s, "DW-0601")),
    ("DW-0116", |s| crate::frames::photos::set_scene(s, "DW-0116")),
    ("DW-0602", |s| crate::frames::photos::set_scene(s, "DW-0602")),
    ("DW-0603", |s| crate::frames::photos::set_scene(s, "DW-0603")),
    ("DW-0604", |s| crate::frames::photos::set_scene(s, "DW-0604")),
    ("DW-0605", |s| crate::frames::photos::set_scene(s, "DW-0605")),
    ("DW-0605b", |s| crate::frames::photos::set_scene(s, "DW-0605b")),
    ("DW-0606", |s| crate::frames::photos::set_scene(s, "DW-0606")),
    ("DW-0607", |s| crate::frames::photos::set_scene(s, "DW-0607")),
    ("DW-0608", |s| crate::frames::photos::set_scene(s, "DW-0608")),
    ("DW-0609", |s| crate::frames::photos::set_scene(s, "DW-0609")),
    ("DW-0610", |s| crate::frames::photos::set_scene(s, "DW-0610")),
    ("DW-0611", |s| crate::frames::photos::set_scene(s, "DW-0611")),
    // Lane f-shared-search-trash: Shared and Trash, by board frame id.
    ("DW-0501", crate::frames::shared::scenes::dw_0501),
    ("DW-0502", crate::frames::shared::scenes::dw_0502),
    ("DW-0503", crate::frames::shared::scenes::dw_0503),
    ("DW-0505", crate::frames::shared::scenes::dw_0505),
    ("DW-0506", crate::frames::shared::scenes::dw_0506),
    ("DW-0507", crate::frames::shared::scenes::dw_0507),
    ("DW-0508", crate::frames::shared::scenes::dw_0508),
    ("DW-0509", crate::frames::shared::scenes::dw_0509),
    ("DW-0510", crate::frames::shared::scenes::dw_0510),
    ("DW-0747", crate::frames::trash::scenes::dw_0747),
    ("DW-0748", crate::frames::trash::scenes::dw_0748),
    ("DW-0749b", crate::frames::trash::scenes::dw_0749b),
    ("DW-0117", crate::frames::trash::scenes::dw_0117),
];

fn scene(s: &mut State, event: Event) {
    host::dispatch(s, event);
}

/// How long the control channel's thread waits for the window thread to take a request
/// before it answers "busy".
const CONTROL_ACK_WAIT: Duration = Duration::from_secs(3);
/// How long a later launch waits for the running instance's answer before it says that it is
/// not answering. Longer than [`CONTROL_ACK_WAIT`], so a "busy" arrives as such.
const CONTROL_SEND_WAIT: Duration = Duration::from_secs(5);

/// The control channel's handler: hands the request to the window thread and waits for it to
/// be taken, so the caller's acknowledgement means "the app has it", not "a pipe exists".
fn control_handler(target: Arc<Mutex<Option<rune::AppHandle<State>>>>) -> platform::ControlHandler {
    Arc::new(move |request| {
        let main = target.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
        // Not up yet (the first moments of a start).
        let Some(main) = main else { return Reply::Busy };
        let (done, taken) = mpsc::channel();
        main.update(move |s: &mut State, tx| {
            host::on_control(s, request, tx);
            let _ = done.send(());
        });
        match taken.recv_timeout(CONTROL_ACK_WAIT) {
            Ok(()) => Reply::Ok,
            Err(_) => Reply::Busy,
        }
    })
}

/// Starts the app and returns when it quits.
///
/// In order: read the launch arguments (`instance::parse_args`: a link silences every other
/// flag; a shipped build ignores every dev flag); unless this is a tooling run of a dev build,
/// claim this identity's control channel (serving it IS being the one instance; a later launch
/// hands over one request, waits for the answer and exits, or says why it could not) and
/// refuse to start beside another agent that holds the profile's sync lock; then run the two
/// windows, in the notification area when Windows started the app at login.
pub fn run() {
    let identity = AppIdentity::CURRENT;
    let launch = instance::parse_args(std::env::args().skip(1), instance::DEV_BUILD);
    // `--gate-export out.json --scene DW-0401 [--theme dark] [--size 1278x798]`: the frame gate's
    // native half (`ui::gate`). A diagnostic flag: dev builds only, never on a link's command line.
    if launch.tooling
        && launch.rune_args.iter().any(|arg| arg == instance::GATE_EXPORT_FLAG)
        && let Some(code) = crate::ui::gate::run_from_args(
            crate::frames::scenes(SCENES),
            state::init,
            registry::view,
            FONTS,
            FAMILIES,
        )
    {
        std::process::exit(code);
    }
    // Before any state exists: the feedback mode's inbox is chosen when the state is made.
    #[cfg(feature = "feedback")]
    crate::feedback::paths::set_flag_dir(launch.feedback_dir.clone());
    let platform = platform::current();

    // --- engine wiring (E6): identity, state paths, and the engine handle ---
    // Before any engine work: initialise the process-wide identity and state paths.
    identity.assert_macos_ready();
    beebeeb_engine::identity::init(&identity.engine_identity());

    // The real engine, for the running app (not tooling runs).
    let real_engine: Option<Arc<beebeeb_engine::app::Engine<beebeeb_engine::app::PlatformCredentials>>> =
        if launch.tooling {
            None
        } else {
            match setup_engine(identity, &launch) {
                Ok(engine) => Some(engine),
                Err(error) => {
                    host::log(format!("engine could not be opened: {error}"));
                    None
                }
            }
        };

    let engine: Arc<dyn EngineActions> = match &real_engine {
        Some(e) => Arc::new(crate::engine_host::LiveEngine::new(e.clone())),
        None => Arc::new(FixtureEngine::ready()),
    };

    let control_target = Arc::new(Mutex::new(None));
    let (claim, _control_server) = if launch.tooling {
        (Claim::First, None)
    } else {
        match platform.serve_control(identity, control_handler(control_target.clone())) {
            Ok(server) => (Claim::First, Some(server)),
            Err(ServeError::AlreadyRunning) => (Claim::Running, None),
            Err(ServeError::NotBuilt(what)) => {
                host::log(format!(
                    "not built yet: {what}; nothing keeps a second instance from starting"
                ));
                (Claim::First, None)
            }
            Err(ServeError::Os(error)) => {
                host::log(format!("the control channel could not be created ({error})"));
                (Claim::Unavailable, None)
            }
        }
    };
    let start = match instance::plan(&launch, claim, identity) {
        Plan::Tooling => None,
        Plan::ExitQuietly => return,
        Plan::BeTheApp => Some(instance::start(&launch, engine.profile_holder())),
        Plan::Hand(request) => Some(instance::after_hand(platform.send_control(
            identity,
            &request,
            CONTROL_SEND_WAIT,
        ))),
        Plan::Refuse(refusal) => Some(Start::Refuse(refusal)),
    };
    let (tooling, show_window) = match start {
        None => (true, true),
        Some(Start::Run { show_window }) => (false, show_window),
        Some(Start::Forwarded) => return,
        Some(Start::Refuse(refusal)) => {
            let message = instance::refusal_message(&refusal, identity);
            host::log(&message);
            if instance::refusal_is_shown(&launch) {
                platform.show_start_refusal(identity.product_name(), &message);
            }
            // The installer and scripts read this: the request was NOT carried out.
            std::process::exit(1);
        }
    };
    let actions: Arc<dyn ShellActions> = Arc::new(HostActions::new(engine.clone(), platform.clone()));

    // `--db` (dev builds only; a shipped build never parses it).
    let source: Option<Box<dyn DriveSource>> = match launch.db.clone() {
        None => None,
        Some(Err(error)) => {
            eprintln!("beebeeb-drive: {error}");
            std::process::exit(2);
        }
        Some(Ok(path)) => match crate::source_engine::EngineSource::open(&path) {
            Ok(source) => Some(Box::new(source)),
            Err(error) => {
                // A fixed word and the path the person typed; never a name from the database.
                eprintln!("beebeeb-drive: cannot open --db {}: {error}", path.display());
                std::process::exit(2);
            }
        },
    };

    // Where the main window was. A tooling run reads and writes nothing.
    let dirs = if tooling {
        None
    } else {
        rune::dirs::AppDirs::new(identity.state_dir_id())
    };
    let state_file = dirs.as_ref().map(|dirs| dirs.state().join(window_state::FILE_NAME));
    let saved = state_file.as_deref().map(window_state::load).unwrap_or_default();
    let saved_main = saved.get(WindowKind::Main.key()).copied();
    // The 0.8.x app's frame, once: only while this app has no window file of its own.
    let legacy_frame = match (&dirs, &state_file) {
        (Some(dirs), Some(own)) if saved_main.is_none() => {
            window_state::read_legacy_once(own, &dirs.config().join(window_state::LEGACY_FRAME_FILE_NAME))
        }
        _ => None,
    };
    // Signed out, the one window opens as the first-run window (960 x 640), not where the
    // main window was.
    let first_kind = surfaces::kind_for(&app::AppMachine::for_session(engine.session()).screen());

    let main_spec = AppSpec::<State> {
        fonts: FONTS,
        families: FAMILIES,
        scenes: crate::frames::scenes(SCENES),
        min_size: Some(WindowKind::Main.min_size()),
        poll: Some(host::poll),
        on_key: Some(host::on_key),
        ..AppSpec::new(
            identity.main_window_title(),
            first_kind.default_size(),
            state::init,
            registry::view,
        )
    };
    // The live feedback mode (test builds) draws over the main window's view and looks at its
    // inbox in the poll; its keys come through `host::on_key`.
    #[cfg(feature = "feedback")]
    let main_spec = crate::feedback::hook_spec(main_spec);
    let flyout_spec = AppSpec::<FlyoutState> {
        fonts: FONTS,
        families: FAMILIES,
        min_size: Some(WindowKind::Flyout.min_size()),
        on_key: Some(flyout_window::on_key),
        ..AppSpec::new(
            identity.flyout_window_title(),
            WindowKind::Flyout.default_size(),
            flyout_window::fixture_state,
            flyout_window::view,
        )
    };

    let window = Arc::new(Mutex::new(None));
    let window_slot = window.clone();
    // Rune reads only what `parse_args` let through: nothing in a shipped build, nothing on a
    // command line that carries a link.
    let main_app = App::new(main_spec)
        .args(launch.rune_args.clone())
        .titlebar(Titlebar::new(TITLEBAR_H))
        .on_open_url(host::on_open_url)
        .on_reopen(host::on_reopen)
        .on_focus(|s: &mut State, focused, _| crate::frames::signin::on_window_focus(s, focused))
        .on_window(move |native| {
            *window_slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(native.clone())
        });
    // The compositor that reads the live window's tree and pixels for a note.
    #[cfg(feature = "feedback")]
    let main_app = crate::feedback::install(main_app);
    let mut main_app = main_app;
    let flyout_platform = platform.clone();
    let mut flyout_app = App::new(flyout_spec)
        .args(launch.rune_args.clone())
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
    let init_engine = real_engine.clone();
    let main_app = main_app.init(move || {
        let shell = Shell::new(shell_actions, shell_platform, wiring);
        match (source, init_engine) {
            (Some(source), _) => state::init_from(shell, source, None),
            (None, Some(engine)) => {
                let drive: Box<dyn DriveSource> = Box::new(crate::engine_host::EngineDriveSource::new(engine.clone()));
                let mut s = state::init_from(shell, drive, None);
                s.actions = crate::frames::files::Actions::new(Box::new(crate::engine_host::EngineFilesSource::new(
                    engine.clone(),
                )));
                s.signin = Some(crate::frames::signin::Ui::new(Box::new(
                    crate::engine_host::EngineSignInSource::new(engine),
                )));
                s
            }
            (None, None) => state::init_with(shell),
        }
    });
    let flyout_main = main.clone();
    let flyout_app = flyout_app.init(move || FlyoutState::new(actions, Some(flyout_main), flyout_hidden_at));

    if !tooling {
        match platform.adopt_autostart() {
            Ok(true) => host::log("start at login: the earlier app's entry for this program is now in this app's form"),
            Ok(false) => {}
            Err(error) => host::log(format!("start at login was not checked ({error})")),
        }
        // Runs on the UI thread once the event loop is up, window or not.
        main.update(|s, _| {
            s.shell.start_tray();
            s.shell.start_sync();
        });
        // From here a later launch's request reaches the window thread.
        *control_target.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(main.clone());
    }

    rune::run_windows(move |displays| {
        let monitors = surfaces::monitors(displays);
        let first_place = saved_main.or_else(|| {
            legacy_frame
                .as_deref()
                .and_then(|text| window_state::import_legacy_frame(text, &monitors))
        });
        let (main_app, main_options) = if first_kind == WindowKind::Main {
            let restored = window_state::restore(first_place.as_ref(), &monitors, surfaces::MAIN_MIN_SIZE);
            (
                main_app.maximized(restored.maximized),
                surfaces::main_options(&restored),
            )
        } else {
            (main_app, surfaces::first_run_options())
        };
        vec![
            WindowSpec::from_app(WindowKind::Main.key(), main_app)
                .options(main_options)
                .boxed(),
            WindowSpec::from_app(WindowKind::Flyout.key(), flyout_app)
                .options(surfaces::flyout_options())
                .boxed(),
        ]
    });
}

/// Build the engine handle: identity must be initialised before this is called. Runs BEFORE any
/// engine thread (no race on `BB_API_BASE` or state paths). The `api_base` comes from
/// `--api-base` (dev builds only) or from `runner::api_base_url()` (which reads `BB_API_BASE`).
fn setup_engine(
    identity: AppIdentity,
    launch: &instance::Launch,
) -> Result<Arc<beebeeb_engine::app::Engine<beebeeb_engine::app::PlatformCredentials>>, String> {
    use beebeeb_engine::app::{Engine, EngineConfig, PlatformCredentials};
    use beebeeb_engine::host::ChannelHost;

    // State paths: the app-local data dir from Rune's dirs, under the identity.
    let dirs = rune::dirs::AppDirs::new(identity.state_dir_id())
        .ok_or_else(|| "could not resolve the app-local data directory".to_string())?;
    beebeeb_engine::state_paths::init_from_app_local_data_dir(&dirs.data())?;

    // The account id from desktop.toml (created on first run).
    let account_id = beebeeb_engine::config::DesktopConfig::ensure_account_id()?;

    // API base: from --api-base (non-release only), else the engine's own default.
    let api_base = match &launch.api_base {
        Some(url) if instance::DEV_BUILD => url.clone(),
        _ => beebeeb_engine::runner::api_base_url(),
    };

    let (host, _messages) = ChannelHost::new(|| {
        // The wake hook: nothing yet. The UI refreshes on every poll, which runs at the frame
        // rate. A more precise wake (calling `main.update`) needs the AppHandle, which is not
        // available here: the engine is created before the App. A future commit can fill this
        // with an `Arc<Mutex<Option<AppHandle>>>` as `control_target` does.
    });

    let config = EngineConfig {
        account_id,
        state_dir: dirs.data().to_path_buf(),
        api_base,
    };

    let engine = Engine::open(config, host, PlatformCredentials)?;
    let engine = Arc::new(engine);

    // Restore the session (reads credential store, no network).
    let gate = engine.restore();
    host::log(format!("engine restore: {gate:?}"));

    Ok(engine)
}
