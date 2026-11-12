//! One Beebeeb per identity, user AND Windows session (exclusivity across sessions comes from the
//! engine/profile lock, not linked yet): the pure half of starting up.
//!
//! Three mechanisms:
//! - **The control channel** (`control.rs`; a named pipe on Windows, open to this user only).
//!   Whoever serves it IS the running instance: a later launch hands it one request (`show`,
//!   `open <link>`, `quit-for-update`), waits for the acknowledgement and exits. Rune's
//!   `single_instance` is not used: its relaunch forwards links only and drops every other
//!   argument, so it cannot carry `--quit-for-update`, and it gives the caller no answer.
//! - **The sync lock** (`.beebeeb-sync.lock` in the state dir, `lockfile.rs`): the engine's.
//!   It is what keeps a 0.8.x Tauri app and the native app from syncing one profile together.
//!   The shell reads its verdict ([`ProfileHolder`]) through `EngineActions::profile_holder`,
//!   refuses to start over it, and `ShellActions::start_sync` refuses again at the moment sync
//!   would start.
//! - **The command line**, which is untrusted when the OS starts the app for a link:
//!   see [`parse_args`].

use std::path::PathBuf;

use super::app::Intent;
use super::autostart::AUTOSTART_FLAG;
use super::control::Request;
use super::deeplink::{self, DeepLinkError};
use super::identity::AppIdentity;

/// The installer's request to a running instance: stop the engine and exit (installer spec §5).
pub const QUIT_FOR_UPDATE_FLAG: &str = "--quit-for-update";
/// The ONLY form in which the app takes a link from its command line: `--open-link <link>`.
pub const OPEN_LINK_FLAG: &str = "--open-link";
/// Development only: show the files of that `state.db` (read-only) instead of the fixture.
pub const DB_FLAG: &str = "--db";
/// Development only: the frame gate's element export (`ui::gate::run_from_args`).
pub const GATE_EXPORT_FLAG: &str = "--gate-export";

/// Development only, and only in a build with the `feedback` feature: where the live feedback
/// mode writes its notes (`feedback/paths.rs`).
#[cfg(feature = "feedback")]
pub const FEEDBACK_DIR_FLAG: &str = "--feedback-dir";

/// Whether this build honours development and diagnostic flags (Rune's `--snap`, `--out`,
/// `--html`, `--script`, …, and `--db`): debug builds, and release builds made with the
/// `dev-tools` feature. A shipped build is neither, and ignores every one of them.
pub const DEV_BUILD: bool = cfg!(debug_assertions) || cfg!(feature = "dev-tools");

/// What this launch's arguments ask for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Launch {
    /// Started by Windows at login: stay in the notification area.
    pub autostart: bool,
    /// The installer asks the running instance to quit; with none running, exit at once.
    pub quit_for_update: bool,
    /// The argument after `--open-link`, unparsed and untrusted. When this is set, nothing
    /// else on the command line was honoured.
    pub link: Option<String>,
    /// A Rune tooling run (`--snap`, `--tree`, `--script`, …): a renderer, not the app. It
    /// must not claim the instance, show a tray icon or touch saved state. Dev builds only.
    pub tooling: bool,
    /// `--db <path>` (dev builds only). `Err`: the flag was given without a path.
    pub db: Option<Result<PathBuf, &'static str>>,
    /// `--feedback-dir <path>` (dev builds with the `feedback` feature only).
    #[cfg(feature = "feedback")]
    pub feedback_dir: Option<PathBuf>,
    /// What Rune may read (`App::args`): empty unless this is a dev build without a link.
    pub rune_args: Vec<String>,
}

/// Rune's headless tooling flags (`crates/rune/src/tooling/args.rs`). Only these make a run a
/// renderer, and only in a dev build: an unknown flag must never switch off the single
/// instance or the profile check.
const TOOLING_FLAGS: [&str; 25] = [
    GATE_EXPORT_FLAG,
    "--snap",
    "--tree",
    "--script",
    "--video",
    "--out",
    "--strip",
    "--html",
    "--trace",
    "--trace-all",
    "--frames",
    "--duration",
    "--playground",
    "--list-scenes",
    "--list-windows",
    "--first-pixels",
    "--desktop",
    "--scene",
    "--window",
    "--crop",
    "--native",
    "--realtime",
    "--chrome",
    "-h",
    "--help",
];

/// Reads the command line (without the program name). `dev`: [`DEV_BUILD`].
///
/// - **A link wins and silences everything else.** If `--open-link` appears anywhere, the
///   launch is that link and nothing more: no other flag is honoured, in any build. The OS
///   substitutes the link into the registered command line verbatim, so a link that closes
///   its quote can append arguments; they must do nothing. A bare `scheme://…` argument
///   without `--open-link` is not a link to this app.
/// - `--autostart` and `--quit-for-update` are the app's own flags.
/// - Dev and diagnostic flags are read only when `dev` is true.
pub fn parse_args<I: IntoIterator<Item = String>>(args: I, dev: bool) -> Launch {
    let args: Vec<String> = args.into_iter().collect();
    if let Some(at) = args.iter().position(|arg| arg == OPEN_LINK_FLAG) {
        return Launch {
            // Missing or oversized: still a link launch, of a link that will not parse.
            link: Some(
                args.get(at + 1)
                    .filter(|link| link.len() <= deeplink::MAX_LEN)
                    .cloned()
                    .unwrap_or_default(),
            ),
            ..Launch::default()
        };
    }
    let mut launch = Launch::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        #[cfg(feature = "feedback")]
        if dev && arg == FEEDBACK_DIR_FLAG {
            launch.feedback_dir = args
                .next()
                .filter(|dir| !dir.is_empty() && !dir.starts_with("--"))
                .map(PathBuf::from);
            continue;
        }
        if arg == AUTOSTART_FLAG {
            launch.autostart = true;
        } else if arg == QUIT_FOR_UPDATE_FLAG {
            launch.quit_for_update = true;
        } else if !dev {
            // A shipped build: nothing else means anything.
        } else if arg == DB_FLAG {
            launch.db = Some(match args.next() {
                Some(path) if !path.is_empty() && !path.starts_with("--") => Ok(PathBuf::from(path)),
                _ => Err("--db needs the path of a state.db"),
            });
        } else {
            launch.tooling |= TOOLING_FLAGS.contains(&arg.as_str());
            launch.rune_args.push(arg);
        }
    }
    if launch.quit_for_update {
        // The installer's call is never a renderer and never opens a database.
        launch.tooling = false;
        launch.db = None;
        launch.rune_args.clear();
        #[cfg(feature = "feedback")]
        {
            launch.feedback_dir = None;
        }
    }
    launch
}

/// What a later launch asks of the running instance.
pub fn request_for(launch: &Launch, identity: AppIdentity) -> Request {
    if launch.quit_for_update {
        return Request::QuitForUpdate;
    }
    match &launch.link {
        Some(link) => Request::open(link, identity),
        None => Request::Show,
    }
}

/// Another Beebeeb agent that holds this profile's sync lock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileHolder {
    /// The lock's `agent` field: "desktop" for the 0.8.x app, "cli" for `bb watch`.
    pub agent: String,
    pub pid: u32,
}

/// What asking to serve the control channel said.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Claim {
    /// This process serves the channel: it is the instance.
    First,
    /// Another process serves it.
    Running,
    /// The channel could not be created and nobody is known to serve it.
    Unavailable,
}

/// What handing a request to the running instance said.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    Acknowledged,
    /// It is there, and did not answer in time (or answered that its window thread is busy).
    NoAnswer,
    /// The channel exists and cannot be reached.
    Unreachable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Another agent (the 0.8.x app) is syncing this profile.
    HeldBy(ProfileHolder),
    /// A running instance did not answer.
    NotAnswering,
    /// The control channel is unusable. Fail closed: two instances on one identity would both
    /// write the window file and both show a b.
    ChannelUnavailable,
}

/// What to do once the channel has been asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plan {
    /// A tooling run: render and exit, none of the app's lifetime (the channel is not asked).
    Tooling,
    /// Be the app, if the profile is free ([`start`]).
    BeTheApp,
    /// Hand this to the running instance, then [`after_hand`].
    Hand(Request),
    /// `--quit-for-update` with nothing running: exit at once, successfully.
    ExitQuietly,
    Refuse(Refusal),
}

pub fn plan(launch: &Launch, claim: Claim, identity: AppIdentity) -> Plan {
    if launch.tooling {
        return Plan::Tooling;
    }
    match claim {
        Claim::First if launch.quit_for_update => Plan::ExitQuietly,
        Claim::First => Plan::BeTheApp,
        Claim::Running => Plan::Hand(request_for(launch, identity)),
        Claim::Unavailable => Plan::Refuse(Refusal::ChannelUnavailable),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Start {
    /// Be the app. `show_window` is false for a start at login.
    Run { show_window: bool },
    /// The running instance took this launch's request.
    Forwarded,
    /// Say why (when there is someone to say it to) and exit with a failure code.
    Refuse(Refusal),
}

pub fn after_hand(delivery: Delivery) -> Start {
    match delivery {
        Delivery::Acknowledged => Start::Forwarded,
        Delivery::NoAnswer => Start::Refuse(Refusal::NotAnswering),
        Delivery::Unreachable => Start::Refuse(Refusal::ChannelUnavailable),
    }
}

/// The first instance: run, unless another agent holds the profile.
pub fn start(launch: &Launch, holder: Option<ProfileHolder>) -> Start {
    match holder {
        Some(holder) => Start::Refuse(Refusal::HeldBy(holder)),
        None => Start::Run {
            show_window: !launch.autostart,
        },
    }
}

/// Whether a refusal is shown in a native message box. Not for a start at login (a box at
/// every login) and not for the installer's call (its exit code is the answer).
pub fn refusal_is_shown(launch: &Launch) -> bool {
    !launch.autostart && !launch.quit_for_update
}

/// What the running instance does with one argument a later launch (or the OS) handed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Forwarded {
    /// A link on the allow-list: bring the window forward and ask the route machine.
    Show(Intent),
    /// Anything else: bring the window forward, and nothing more.
    ShowOnly(DeepLinkError),
}

pub fn forwarded(link: &str) -> Forwarded {
    match deeplink::parse(link) {
        Ok(intent) => Forwarded::Show(intent),
        Err(error) => Forwarded::ShowOnly(error),
    }
}

/// The words for a refused start.
pub fn refusal_message(refusal: &Refusal, identity: AppIdentity) -> String {
    let name = identity.product_name();
    match refusal {
        Refusal::HeldBy(holder) => {
            let who = match holder.agent.as_str() {
                "desktop" => format!("An earlier version of {name}"),
                "cli" => "The bb command line".to_owned(),
                _ => "Another Beebeeb program".to_owned(),
            };
            format!(
                "{who} is already syncing on this PC (process {}). Quit it first, then open {name} again.",
                holder.pid
            )
        }
        Refusal::NotAnswering => format!(
            "{name} is already running, but it is not answering. Wait a moment and try again. \
             If it stays like this, end {name} in Task Manager and open it again."
        ),
        Refusal::ChannelUnavailable => format!(
            "{name} could not check whether it is already running, so it did not start a second time. \
             Sign out of Windows and back in, then open {name} again."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::super::app::{Route, Section, SettingsPane};
    use super::*;

    const REAL: AppIdentity = AppIdentity::Real;

    /// Task 2455: the control channel's name is taken by a server that is not this user's
    /// Beebeeb. Creating our own pipe fails (the name exists), so the launch tries to hand
    /// over, the platform refuses to write to a stranger (`Delivery::Unreachable`), and the
    /// start is refused with words and exit code 1: never a second instance, never "forwarded".
    #[test]
    fn a_channel_served_by_someone_else_refuses_the_start_with_a_clear_message() {
        for list in [
            &[][..],
            &["--open-link", "beebeeb://open/photos"],
            &["--quit-for-update"],
            &["--autostart"],
        ] {
            let launch = parse(list, false);
            let Plan::Hand(_) = plan(&launch, Claim::Running, REAL) else {
                panic!("{list:?}: a taken name is handed to, never started beside");
            };
            let start = after_hand(Delivery::Unreachable);
            assert_eq!(start, Start::Refuse(Refusal::ChannelUnavailable), "{list:?}");
            assert_ne!(start, Start::Forwarded);
        }
        let words = refusal_message(&Refusal::ChannelUnavailable, REAL);
        assert!(
            words.contains("could not check whether it is already running")
                && words.contains("did not start a second time"),
            "{words}"
        );
        // A person who opened the app is told; a start at login and the installer get the exit code.
        assert!(refusal_is_shown(&parse(&[], false)));
        assert!(refusal_is_shown(&parse(
            &["--open-link", "beebeeb://open/photos"],
            false
        )));
        assert!(!refusal_is_shown(&parse(&["--quit-for-update"], false)));
        assert!(!refusal_is_shown(&parse(&["--autostart"], false)));
    }

    fn parse(list: &[&str], dev: bool) -> Launch {
        parse_args(list.iter().map(|s| (*s).to_owned()), dev)
    }

    fn holder() -> ProfileHolder {
        ProfileHolder {
            agent: "desktop".into(),
            pid: 4242,
        }
    }

    #[test]
    fn a_plain_launch_opens_the_window() {
        let launch = parse(&[], false);
        assert_eq!(launch, Launch::default());
        assert_eq!(plan(&launch, Claim::First, REAL), Plan::BeTheApp);
        assert_eq!(start(&launch, None), Start::Run { show_window: true });
    }

    #[test]
    fn a_start_at_login_stays_in_the_notification_area() {
        let launch = parse(&["--autostart"], false);
        assert!(launch.autostart && !launch.tooling);
        assert_eq!(start(&launch, None), Start::Run { show_window: false });
        assert!(!refusal_is_shown(&launch), "no message box at every login");
    }

    /// P2-5: a shipped build ignores every development and diagnostic flag.
    #[test]
    fn a_release_configured_parse_ignores_every_dev_flag() {
        let dev_flags = [
            &["--snap", "C:\\x.png"][..],
            &["--out", "C:\\dir"],
            &["--html", "C:\\x.html"],
            &["--script", "C:\\s.json"],
            &["--db", "C:\\state.db"],
            &["--feedback-dir", "C:\\f"],
            &["--gate-export", "C:\\g.json", "--scene", "DW-0401"],
            &["--video", "v.mp4"],
            &["--tree", "t.json"],
            &["--list-windows"],
            &["--help"],
        ];
        for flags in dev_flags {
            let launch = parse(flags, false);
            assert_eq!(launch, Launch::default(), "{flags:?}");
            assert!(launch.rune_args.is_empty(), "Rune is handed nothing: {flags:?}");
        }
        for flag in TOOLING_FLAGS {
            assert_eq!(parse(&[flag, "x"], false), Launch::default(), "{flag}");
        }
        // The app's own two flags still work.
        assert!(parse(&["--snap", "x.png", "--autostart"], false).autostart);
        assert!(parse(&["--quit-for-update", "--db", "x"], false).quit_for_update);
    }

    #[test]
    fn a_dev_build_reads_the_tooling_flags_and_db() {
        let launch = parse(&["--snap", "out.png", "--db", "a/state.db", "--scale", "2"], true);
        assert!(launch.tooling);
        assert_eq!(launch.db, Some(Ok(PathBuf::from("a/state.db"))));
        assert_eq!(launch.rune_args, ["--snap", "out.png", "--scale", "2"]);
        assert_eq!(plan(&launch, Claim::Running, REAL), Plan::Tooling);
        for bad in [&["--db"][..], &["--db", "--snap"], &["--db", ""]] {
            assert!(matches!(parse(bad, true).db, Some(Err(_))), "{bad:?}");
        }
        // An unknown flag never makes it a renderer.
        let launch = parse(&["--foo", "-x", "--snapshot", "C:\\file.txt"], true);
        assert!(!launch.tooling);
        assert_eq!(plan(&launch, Claim::Running, REAL), Plan::Hand(Request::Show));
    }

    /// P2-5: with a link, no other flag is honoured, in a dev build either.
    #[test]
    fn a_link_launch_honours_no_other_flag() {
        let injected = [
            &[
                "--open-link",
                "beebeeb://open/photos",
                "--snap",
                "C:\\Users\\x\\Startup\\a.png",
            ][..],
            &["--snap", "C:\\a.png", "--open-link", "beebeeb://open/photos"],
            &[
                "--open-link",
                "beebeeb://open/photos",
                "--autostart",
                "--quit-for-update",
                "--db",
                "x",
            ],
            &[
                "--db",
                "x",
                "--out",
                "C:\\d",
                "--open-link",
                "beebeeb://open/photos",
                "--html",
                "h",
            ],
        ];
        for args in injected {
            for dev in [false, true] {
                let launch = parse(args, dev);
                assert_eq!(
                    launch,
                    Launch {
                        link: Some("beebeeb://open/photos".into()),
                        ..Launch::default()
                    },
                    "{args:?} dev={dev}"
                );
                assert_eq!(
                    request_for(&launch, REAL),
                    Request::Open("beebeeb://open/photos".into())
                );
            }
        }
        // The flag with nothing after it, or with an oversized link: a link launch of nothing.
        assert_eq!(parse(&["--snap", "x", "--open-link"], true).link, Some(String::new()));
        let long = "x".repeat(deeplink::MAX_LEN + 1);
        assert_eq!(parse(&["--open-link", &long], true).link, Some(String::new()));
        assert_eq!(request_for(&parse(&["--open-link"], false), REAL), Request::Show);
    }

    #[test]
    fn a_bare_link_argument_is_not_a_link_launch() {
        for dev in [false, true] {
            let launch = parse(&["beebeeb://open/photos"], dev);
            assert_eq!(launch.link, None);
            assert_eq!(request_for(&launch, REAL), Request::Show);
        }
    }

    /// P2-4: the installer's flag.
    #[test]
    fn quit_for_update_asks_the_running_instance_and_exits_at_once_without_one() {
        for dev in [false, true] {
            let launch = parse(&["--quit-for-update", "--snap", "x.png", "--db", "y"], dev);
            assert!(launch.quit_for_update && !launch.tooling && launch.db.is_none());
            assert!(launch.rune_args.is_empty());
            assert_eq!(plan(&launch, Claim::Running, REAL), Plan::Hand(Request::QuitForUpdate));
            assert_eq!(plan(&launch, Claim::First, REAL), Plan::ExitQuietly);
            assert_eq!(
                plan(&launch, Claim::Unavailable, REAL),
                Plan::Refuse(Refusal::ChannelUnavailable)
            );
            assert!(!refusal_is_shown(&launch), "the installer reads the exit code");
        }
    }

    /// P2-9: a first instance that does not answer is said, never a silent exit; an
    /// unreachable channel never lets a second full instance run.
    #[test]
    fn a_second_launch_is_handed_over_and_a_silent_first_instance_is_reported() {
        let launch = parse(&["--open-link", "beebeeb://settings/security"], false);
        assert_eq!(
            plan(&launch, Claim::Running, REAL),
            Plan::Hand(Request::Open("beebeeb://settings/security".into()))
        );
        assert_eq!(after_hand(Delivery::Acknowledged), Start::Forwarded);
        assert_eq!(after_hand(Delivery::NoAnswer), Start::Refuse(Refusal::NotAnswering));
        assert_eq!(
            after_hand(Delivery::Unreachable),
            Start::Refuse(Refusal::ChannelUnavailable)
        );
        for launch in [parse(&[], false), parse(&["--autostart"], false)] {
            assert_eq!(
                plan(&launch, Claim::Unavailable, REAL),
                Plan::Refuse(Refusal::ChannelUnavailable),
                "fail closed"
            );
        }
        assert!(refusal_is_shown(&parse(&[], false)));
        let words = refusal_message(&Refusal::NotAnswering, REAL);
        assert!(words.starts_with("Beebeeb is already running, but it is not answering."));
        let words = refusal_message(&Refusal::ChannelUnavailable, AppIdentity::Test);
        assert!(words.starts_with("Beebeeb Test could not check whether it is already running"));
    }

    #[test]
    fn a_profile_held_by_the_old_app_refuses_the_start_and_says_who() {
        assert_eq!(
            start(&parse(&[], false), Some(holder())),
            Start::Refuse(Refusal::HeldBy(holder()))
        );
        assert_eq!(
            start(&parse(&["--autostart"], false), Some(holder())),
            Start::Refuse(Refusal::HeldBy(holder()))
        );
        let message = refusal_message(&Refusal::HeldBy(holder()), REAL);
        assert!(message.starts_with("An earlier version of Beebeeb is already syncing"));
        assert!(message.contains("4242"));
        let by = |agent: &str| {
            refusal_message(
                &Refusal::HeldBy(ProfileHolder {
                    agent: agent.into(),
                    pid: 7,
                }),
                REAL,
            )
        };
        assert!(by("cli").starts_with("The bb command line"));
        assert!(by("x").starts_with("Another Beebeeb program"));
    }

    #[test]
    fn a_forwarded_link_becomes_an_intent_or_only_shows_the_window() {
        assert_eq!(
            forwarded(&deeplink::link("open/photos")),
            Forwarded::Show(Intent::Go(Route::section(Section::Photos)))
        );
        assert_eq!(
            forwarded(&deeplink::link("settings/security")),
            Forwarded::Show(Intent::Settings(SettingsPane::Security))
        );
        assert_eq!(
            forwarded(&deeplink::link("quit")),
            Forwarded::ShowOnly(DeepLinkError::UnknownTarget)
        );
        assert_eq!(
            forwarded("https://example.com/"),
            Forwarded::ShowOnly(DeepLinkError::NotOurScheme)
        );
        assert_eq!(
            forwarded(&"x".repeat(5_000)),
            Forwarded::ShowOnly(DeepLinkError::TooLong)
        );
    }
}
