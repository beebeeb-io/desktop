//! One Beebeeb per profile: the pure half of starting up.
//!
//! Two mechanisms, as the engine boundary spec lists them (section 6 and step 10):
//! - **The sync lock** (`.beebeeb-sync.lock` in the state dir, `lockfile.rs`): the engine's.
//!   It is what keeps a 0.8.x Tauri app and the native app from syncing one profile together,
//!   and it moves into the engine crate unchanged. The shell only reads its verdict
//!   ([`ProfileHolder`]) through `EngineActions::profile_holder` and refuses to start over it.
//! - **A named pipe**, so a second launch of the native app focuses the first and hands it its
//!   arguments: Rune's `single_instance` (per user, named by [`APP_ID`]). Links among the
//!   arguments reach `on_open_url`; a launch without one reopens the window.

use super::app::Intent;
use super::autostart::AUTOSTART_FLAG;
use super::deeplink::{self, DeepLinkError};

/// The app's identifier: Tauri's `identifier`, so the state and config folders are the same.
pub const APP_ID: &str = "io.beebeeb.app";

/// What this launch's arguments ask for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Launch {
    /// Started by Windows at login: stay in the notification area.
    pub autostart: bool,
    /// A Rune tooling run (`--snap`, `--tree`, `--script`, …): a renderer, not the app. It must
    /// not claim the instance, show a tray icon or touch saved state.
    pub tooling: bool,
}

/// Rune's headless tooling flags (`crates/rune/src/tooling/args.rs`). Only these make a run a
/// renderer: an unknown flag must never switch off the single instance or the profile check.
const TOOLING_FLAGS: [&str; 22] = [
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
    "-h",
    "--help",
];

pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Launch {
    let mut launch = Launch::default();
    for arg in args {
        if arg == AUTOSTART_FLAG {
            launch.autostart = true;
        } else if TOOLING_FLAGS.contains(&arg.as_str()) {
            launch.tooling = true;
        }
        // Links (`scheme://…`) are not read here: Rune hands each to `host::on_open_url`.
    }
    launch
}

/// Another Beebeeb agent that holds this profile's sync lock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileHolder {
    /// The lock's `agent` field: "desktop" for the 0.8.x app, "cli" for `bb watch`.
    pub agent: String,
    pub pid: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Start {
    /// Be the app. `show_window` is false for a start at login.
    Run { show_window: bool },
    /// A tooling run: render and exit, none of the app's lifetime.
    Tooling,
    /// A running native instance took this launch's arguments and came forward.
    Forwarded,
    /// Another agent (the 0.8.x app) is syncing this profile. Say so and exit.
    HeldBy(ProfileHolder),
}

/// `first_instance`: what Rune's `single_instance` returned (only asked when not tooling).
pub fn decide(launch: &Launch, first_instance: bool, holder: Option<ProfileHolder>) -> Start {
    if launch.tooling {
        return Start::Tooling;
    }
    if !first_instance {
        return Start::Forwarded;
    }
    match holder {
        Some(holder) => Start::HeldBy(holder),
        None => Start::Run {
            show_window: !launch.autostart,
        },
    }
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

/// The words for a start refused because another agent holds the profile.
pub fn held_message(holder: &ProfileHolder) -> String {
    let who = match holder.agent.as_str() {
        "desktop" => "An earlier version of Beebeeb",
        "cli" => "The bb command line",
        _ => "Another Beebeeb program",
    };
    format!(
        "{who} is already syncing on this PC (process {}). Quit it first, then open Beebeeb again.",
        holder.pid
    )
}

#[cfg(test)]
mod tests {
    use super::super::app::{Route, Section, SettingsPane};
    use super::*;

    fn args(list: &[&str]) -> Launch {
        parse_args(list.iter().map(|s| (*s).to_owned()))
    }

    #[test]
    fn a_plain_launch_opens_the_window() {
        let launch = args(&[]);
        assert_eq!(launch, Launch::default());
        assert_eq!(decide(&launch, true, None), Start::Run { show_window: true });
    }

    #[test]
    fn a_start_at_login_stays_in_the_notification_area() {
        let launch = args(&["--autostart"]);
        assert!(launch.autostart && !launch.tooling);
        assert_eq!(decide(&launch, true, None), Start::Run { show_window: false });
    }

    #[test]
    fn links_files_and_unknown_flags_leave_it_the_app() {
        let launch = args(&["beebeeb://open/photos", "C:\\file.txt", "--foo", "-x", "--snapshot"]);
        assert_eq!(launch, Launch::default());
        // So the single instance and the profile check still apply.
        assert_eq!(decide(&launch, false, None), Start::Forwarded);
    }

    #[test]
    fn rune_tooling_flags_make_it_a_renderer_not_the_app() {
        for flags in [
            &["--snap", "out.png"][..],
            &["--tree", "t.json"],
            &["--list-windows"],
            &["-h"],
            &["--window", "flyout", "--snap", "x.png"],
        ] {
            let launch = args(flags);
            assert!(launch.tooling, "{flags:?}");
            // Whatever single_instance or the lock would say.
            assert_eq!(
                decide(
                    &launch,
                    false,
                    Some(ProfileHolder {
                        agent: "desktop".into(),
                        pid: 1
                    })
                ),
                Start::Tooling
            );
        }
    }

    #[test]
    fn a_second_launch_is_forwarded_before_anything_else() {
        let launch = args(&["beebeeb://settings/security"]);
        assert_eq!(decide(&launch, false, None), Start::Forwarded);
        let holder = ProfileHolder {
            agent: "desktop".into(),
            pid: 4242,
        };
        assert_eq!(decide(&launch, false, Some(holder)), Start::Forwarded);
    }

    #[test]
    fn a_profile_held_by_the_old_app_refuses_the_start_and_says_who() {
        let holder = ProfileHolder {
            agent: "desktop".into(),
            pid: 4242,
        };
        assert_eq!(
            decide(&args(&[]), true, Some(holder.clone())),
            Start::HeldBy(holder.clone())
        );
        assert_eq!(
            decide(&args(&["--autostart"]), true, Some(holder.clone())),
            Start::HeldBy(holder.clone())
        );
        let message = held_message(&holder);
        assert!(message.starts_with("An earlier version of Beebeeb is already syncing"));
        assert!(message.contains("4242"));
        assert!(
            held_message(&ProfileHolder {
                agent: "cli".into(),
                pid: 7
            })
            .starts_with("The bb command line")
        );
        assert!(
            held_message(&ProfileHolder {
                agent: "x".into(),
                pid: 7
            })
            .starts_with("Another Beebeeb program")
        );
    }

    #[test]
    fn a_forwarded_link_becomes_an_intent_or_only_shows_the_window() {
        assert_eq!(
            forwarded("beebeeb://open/photos"),
            Forwarded::Show(Intent::Go(Route::section(Section::Photos)))
        );
        assert_eq!(
            forwarded("beebeeb://settings/security"),
            Forwarded::Show(Intent::Settings(SettingsPane::Security))
        );
        assert_eq!(
            forwarded("beebeeb://quit"),
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
