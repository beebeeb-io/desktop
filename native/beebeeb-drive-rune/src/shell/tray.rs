//! The b in the notification area, as a pure model: which icon, which tooltip, which menu, and
//! what a left click does (DW-1230, DW-1231, DW-1231b). The OS half is `platform::TrayIcon`.
//!
//! The precedence and the words are `src-tauri/src/tray_window.rs`'s `tray_menu_state`, so 0.9
//! says what 0.8.x said for the same state. The menu is the board's (DW-1231), not 0.8.x's.

use super::identity::AppIdentity;

/// What the sync engine last said, as far as the b needs it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EngineState {
    /// Its last check reached Beebeeb and left nothing to do: the one state that is "up to date".
    /// The default is the fixture's; the running app always says which state it is in.
    #[default]
    Idle,
    Syncing,
    Offline,
    /// Stopped: an error the engine could not get past (DW-1207, "Sync stopped").
    Error,
    /// It runs, and since it started or was resumed it has not finished a check, or what it said
    /// last is not known (DW-1212: "never a guess such as Up to date").
    Checking,
    /// It does not run on this PC (before its first start, after a start that failed, or without
    /// the account's keys): nothing is checked, so nothing is up to date.
    NotRunning,
}

/// What the b reads. The session lane and the engine fill this; see `ShellActions::tray_inputs`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrayInputs {
    pub signed_in: bool,
    /// Signed out because the session ended after 90 days without use; the key and the file
    /// list are still on this PC, so only the password is needed (DW-1219). `signed_in` is
    /// false with it.
    pub session_expired: bool,
    pub locked: bool,
    pub paused: bool,
    /// Beebeeb is in File Explorer (the sync root is registered).
    pub explorer_ready: bool,
    pub update_available: bool,
    /// The account's storage is full (DW-1209, "Your storage is full"). It can hold while the
    /// engine is offline or stopped, and wins over both (DW-1201's order).
    pub storage_full: bool,
    pub conflicts: u32,
    pub engine: EngineState,
    pub files_remaining: Option<u32>,
}

/// The six icons of DW-1230.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TrayIconState {
    /// The plain b.
    UpToDate,
    /// A turning badge.
    Syncing,
    /// A pause badge.
    Paused,
    /// An orange ! badge.
    NeedsDecision,
    /// A red ! badge: stopped, storage full, signed out after 90 days.
    Stopped,
    /// Faded and crossed out.
    Offline,
}

impl TrayIconState {
    pub const ALL: [TrayIconState; 6] = [
        TrayIconState::UpToDate,
        TrayIconState::Syncing,
        TrayIconState::Paused,
        TrayIconState::NeedsDecision,
        TrayIconState::Stopped,
        TrayIconState::Offline,
    ];

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn name(self) -> &'static str {
        match self {
            TrayIconState::UpToDate => "up-to-date",
            TrayIconState::Syncing => "syncing",
            TrayIconState::Paused => "paused",
            TrayIconState::NeedsDecision => "needs-decision",
            TrayIconState::Stopped => "stopped",
            TrayIconState::Offline => "offline",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MenuAction {
    OpenBeebeeb,
    OpenInFileExplorer,
    PauseSyncing,
    ResumeSyncing,
    Settings,
    Quit,
}

impl MenuAction {
    const ALL: [MenuAction; 6] = [
        MenuAction::OpenBeebeeb,
        MenuAction::OpenInFileExplorer,
        MenuAction::PauseSyncing,
        MenuAction::ResumeSyncing,
        MenuAction::Settings,
        MenuAction::Quit,
    ];

    /// The id the OS menu hands back.
    pub fn id(self) -> &'static str {
        match self {
            MenuAction::OpenBeebeeb => "open",
            MenuAction::OpenInFileExplorer => "explorer",
            MenuAction::PauseSyncing => "pause",
            MenuAction::ResumeSyncing => "resume",
            MenuAction::Settings => "settings",
            MenuAction::Quit => "quit",
        }
    }

    pub fn from_id(id: &str) -> Option<MenuAction> {
        MenuAction::ALL.into_iter().find(|a| a.id() == id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuEntry {
    Item {
        action: MenuAction,
        label: String,
        enabled: bool,
    },
    Separator,
}

/// What a left click on the b does. DW-1201: "Signed out: a click on the b opens the window on
/// sign-in (DW-A02) instead of the flyout." DW-1230: "Signed out after 90 days: the red !
/// badge, […] a click opens the flyout with Sign in (DW-1219)."
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeftClick {
    ToggleFlyout,
    OpenSignIn,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayModel {
    pub icon: TrayIconState,
    /// The state in two or three words.
    pub label: &'static str,
    /// The app's name plus the state (DW-1230: "\"Beebeeb\" plus the state, e.g. \"Beebeeb:
    /// needs a decision\""); Narrator reads it.
    pub tooltip: String,
    pub menu: Vec<MenuEntry>,
    pub left_click: LeftClick,
}

pub fn tray_model(inputs: &TrayInputs) -> TrayModel {
    tray_model_for(inputs, AppIdentity::CURRENT)
}

/// The order is DW-1201's: "update required, locked, checking, no plan, signed out after 90
/// days, couldn't add to File Explorer (each reason), adding to File Explorer, paused, storage
/// full, offline, stopped; then needs a decision and update available over any of the quieter
/// states; then storage almost full, syncing, up to date." The states of that list this model
/// has inputs for are in that order below; a plain signed-out PC (no account at all) comes
/// first, since none of the others can hold without an account. Checking, and sync not running,
/// come after paused, as in the flyout (`frames::tray::live`): the pause switch is known at once,
/// and a paused engine checks nothing, so "checking" would hold for as long as it is paused.
///
/// The words after the app's name are the board's names for the states (DW-1230's six, DW-1201's
/// "checking", and the flyout titles "Signed out" DW-1219, "Your storage is full" DW-1209), and
/// "not syncing" for the flyout's "Sync isn't running", which the board does not draw.
pub fn tray_model_for(inputs: &TrayInputs, identity: AppIdentity) -> TrayModel {
    use TrayIconState as I;
    let (icon, label, state) = if inputs.session_expired {
        // DW-1230: "Signed out after 90 days: the red ! badge […] because it needs Anna".
        (I::Stopped, "Signed out", "signed out")
    } else if !inputs.signed_in {
        // No account on this PC: nothing is wrong, so no badge (DW-1230 gives the red ! only
        // to "signed out after 90 days").
        (I::UpToDate, "Signed out", "signed out")
    } else if inputs.locked {
        // DW-1230: "Locked, no plan, update available and storage almost full keep the plain b".
        (I::UpToDate, "Locked", "locked")
    } else if inputs.paused {
        (I::Paused, "Paused", "paused")
    } else if inputs.engine == EngineState::NotRunning {
        // Nothing runs, so nothing was checked: the plain b, never "up to date".
        (I::UpToDate, "Not syncing", "not syncing")
    } else if inputs.engine == EngineState::Checking {
        // DW-1212: "never a guess such as Up to date". The plain b, as for the quiet states.
        (I::UpToDate, "Checking", "checking")
    } else if inputs.storage_full {
        (I::Stopped, "Storage full", "storage full")
    } else if inputs.engine == EngineState::Offline {
        (I::Offline, "Offline", "offline")
    } else if inputs.engine == EngineState::Error {
        (I::Stopped, "Stopped", "stopped")
    } else if inputs.conflicts > 0 {
        (I::NeedsDecision, "Needs a decision", "needs a decision")
    } else if inputs.update_available {
        (I::UpToDate, "Update available", "update available")
    } else if inputs.engine == EngineState::Syncing || inputs.files_remaining.unwrap_or(0) > 0 {
        (I::Syncing, "Syncing", "syncing")
    } else {
        // Only `Idle` is left: the engine's last check reached Beebeeb and left nothing to do.
        (I::UpToDate, "Up to date", "up to date")
    };
    let name = identity.product_name();
    let tooltip = format!("{name}: {state}");
    let item = |action, label: String, enabled| MenuEntry::Item { action, label, enabled };
    let pause = if inputs.paused {
        item(MenuAction::ResumeSyncing, "Resume syncing".into(), inputs.signed_in)
    } else {
        item(MenuAction::PauseSyncing, "Pause syncing".into(), inputs.signed_in)
    };
    let menu = vec![
        item(MenuAction::OpenBeebeeb, format!("Open {name}"), true),
        // Disabled while the app is not in File Explorer (DW-1215, DW-1216).
        item(
            MenuAction::OpenInFileExplorer,
            "Open in File Explorer".into(),
            inputs.signed_in && inputs.explorer_ready,
        ),
        MenuEntry::Separator,
        pause,
        // Settings is unavailable until signed in (DW-A02).
        item(MenuAction::Settings, "Settings\u{2026}".into(), inputs.signed_in),
        MenuEntry::Separator,
        // Quit lives only here, and always works.
        item(MenuAction::Quit, format!("Quit {name}"), true),
    ];
    let left_click = if inputs.signed_in || inputs.session_expired {
        LeftClick::ToggleFlyout
    } else {
        LeftClick::OpenSignIn
    };
    TrayModel {
        icon,
        label,
        tooltip,
        menu,
        left_click,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signed_in() -> TrayInputs {
        TrayInputs {
            signed_in: true,
            explorer_ready: true,
            ..TrayInputs::default()
        }
    }
    fn labels(model: &TrayModel) -> Vec<(&str, bool)> {
        model
            .menu
            .iter()
            .filter_map(|e| match e {
                MenuEntry::Item { label, enabled, .. } => Some((label.as_str(), *enabled)),
                MenuEntry::Separator => None,
            })
            .collect()
    }

    #[test]
    fn each_state_has_its_icon_and_tooltip() {
        let with = |change: fn(&mut TrayInputs)| {
            let mut inputs = signed_in();
            change(&mut inputs);
            inputs
        };
        let cases: [(TrayInputs, TrayIconState, &str, &str); 13] = [
            // No account on this PC: the plain b, no badge (DW-1230).
            (
                TrayInputs::default(),
                TrayIconState::UpToDate,
                "Signed out",
                "Beebeeb: signed out",
            ),
            // Signed out after 90 days: the red ! (DW-1230, DW-1219).
            (
                TrayInputs {
                    session_expired: true,
                    explorer_ready: true,
                    ..TrayInputs::default()
                },
                TrayIconState::Stopped,
                "Signed out",
                "Beebeeb: signed out",
            ),
            (
                with(|i| i.locked = true),
                TrayIconState::UpToDate,
                "Locked",
                "Beebeeb: locked",
            ),
            (
                with(|i| i.paused = true),
                TrayIconState::Paused,
                "Paused",
                "Beebeeb: paused",
            ),
            // Sync does not run, and the engine has not finished a check: never "up to date".
            (
                with(|i| i.engine = EngineState::NotRunning),
                TrayIconState::UpToDate,
                "Not syncing",
                "Beebeeb: not syncing",
            ),
            (
                with(|i| i.engine = EngineState::Checking),
                TrayIconState::UpToDate,
                "Checking",
                "Beebeeb: checking",
            ),
            (
                with(|i| i.storage_full = true),
                TrayIconState::Stopped,
                "Storage full",
                "Beebeeb: storage full",
            ),
            (
                with(|i| i.engine = EngineState::Offline),
                TrayIconState::Offline,
                "Offline",
                "Beebeeb: offline",
            ),
            (
                with(|i| i.engine = EngineState::Error),
                TrayIconState::Stopped,
                "Stopped",
                "Beebeeb: stopped",
            ),
            (
                with(|i| i.conflicts = 2),
                TrayIconState::NeedsDecision,
                "Needs a decision",
                "Beebeeb: needs a decision",
            ),
            (
                with(|i| i.update_available = true),
                TrayIconState::UpToDate,
                "Update available",
                "Beebeeb: update available",
            ),
            (
                with(|i| i.engine = EngineState::Syncing),
                TrayIconState::Syncing,
                "Syncing",
                "Beebeeb: syncing",
            ),
            (
                signed_in(),
                TrayIconState::UpToDate,
                "Up to date",
                "Beebeeb: up to date",
            ),
        ];
        for (inputs, icon, label, tooltip) in cases {
            let model = tray_model_for(&inputs, AppIdentity::Real);
            assert_eq!(
                (model.icon, model.label, model.tooltip.as_str()),
                (icon, label, tooltip),
                "{inputs:?}"
            );
            // The icon group's caption is never one state's name.
            assert_ne!(model.label, "Stopped and storage full");
            let test = tray_model_for(&inputs, AppIdentity::Test);
            assert_eq!(test.tooltip, tooltip.replacen("Beebeeb", "Beebeeb Test", 1));
        }
        // Files still to move count as syncing even before the engine says so.
        let model = tray_model(&with(|i| i.files_remaining = Some(3)));
        assert_eq!(model.icon, TrayIconState::Syncing);
    }

    /// DW-1201: "locked, […] signed out after 90 days, […] paused, storage full, offline,
    /// stopped; then needs a decision and update available […]; then […] syncing, up to date."
    #[test]
    fn the_first_state_in_the_boards_order_wins() {
        let mut inputs = TrayInputs {
            signed_in: true,
            session_expired: false,
            locked: true,
            paused: true,
            explorer_ready: true,
            update_available: true,
            storage_full: true,
            conflicts: 4,
            engine: EngineState::Offline,
            files_remaining: Some(9),
        };
        let label = |inputs: &TrayInputs| tray_model(inputs).label;
        assert_eq!(label(&inputs), "Locked");
        inputs.locked = false;
        assert_eq!(label(&inputs), "Paused");
        // Paused over checking and over sync not running: a paused engine checks nothing.
        for engine in [EngineState::Checking, EngineState::NotRunning] {
            assert_eq!(
                label(&TrayInputs {
                    engine,
                    ..inputs.clone()
                }),
                "Paused"
            );
        }
        inputs.paused = false;
        // Checking, and sync not running, over everything quieter (DW-1201: checking before
        // storage full); a decision or numbers left are no finished check.
        let checking = TrayInputs {
            engine: EngineState::Checking,
            ..inputs.clone()
        };
        assert_eq!(label(&checking), "Checking");
        let not_running = TrayInputs {
            engine: EngineState::NotRunning,
            ..inputs.clone()
        };
        assert_eq!(label(&not_running), "Not syncing");
        // Storage full before offline.
        assert_eq!(label(&inputs), "Storage full");
        inputs.storage_full = false;
        assert_eq!(label(&inputs), "Offline");
        inputs.storage_full = true;
        inputs.engine = EngineState::Error;
        assert_eq!(label(&inputs), "Storage full", "and before stopped");
        inputs.storage_full = false;
        assert_eq!(label(&inputs), "Stopped");
        inputs.engine = EngineState::Syncing;
        assert_eq!(label(&inputs), "Needs a decision");
        inputs.conflicts = 0;
        assert_eq!(label(&inputs), "Update available");
        inputs.update_available = false;
        assert_eq!(label(&inputs), "Syncing");
        inputs.engine = EngineState::Idle;
        inputs.files_remaining = None;
        assert_eq!(label(&inputs), "Up to date");
        inputs.signed_in = false;
        inputs.locked = true;
        inputs.paused = true;
        assert_eq!(
            (label(&inputs), tray_model(&inputs).icon),
            ("Signed out", TrayIconState::UpToDate)
        );
        inputs.session_expired = true;
        assert_eq!(
            (label(&inputs), tray_model(&inputs).icon),
            ("Signed out", TrayIconState::Stopped)
        );
    }

    /// DW-1201 and DW-1230: which signed-out state opens what.
    #[test]
    fn plain_signed_out_opens_sign_in_and_an_expired_session_opens_the_flyout() {
        assert_eq!(tray_model(&TrayInputs::default()).left_click, LeftClick::OpenSignIn);
        let expired = TrayInputs {
            session_expired: true,
            ..TrayInputs::default()
        };
        assert_eq!(tray_model(&expired).left_click, LeftClick::ToggleFlyout);
        assert_eq!(tray_model(&signed_in()).left_click, LeftClick::ToggleFlyout);
    }

    #[test]
    fn the_menu_is_the_boards_five_items_in_order() {
        let name = AppIdentity::CURRENT.product_name();
        let model = tray_model(&signed_in());
        assert_eq!(
            labels(&model),
            vec![
                (&*format!("Open {name}"), true),
                ("Open in File Explorer", true),
                ("Pause syncing", true),
                ("Settings\u{2026}", true),
                (&*format!("Quit {name}"), true)
            ]
        );
        assert_eq!(model.left_click, LeftClick::ToggleFlyout);
    }

    #[test]
    fn paused_turns_pause_into_resume() {
        let model = tray_model(&TrayInputs {
            paused: true,
            ..signed_in()
        });
        assert_eq!(labels(&model)[2], ("Resume syncing", true));
        let action = model.menu.iter().find_map(|e| match e {
            MenuEntry::Item { action, label, .. } if label == "Resume syncing" => Some(*action),
            _ => None,
        });
        assert_eq!(action, Some(MenuAction::ResumeSyncing));
    }

    #[test]
    fn explorer_is_disabled_until_beebeeb_is_in_file_explorer() {
        let model = tray_model(&TrayInputs {
            explorer_ready: false,
            ..signed_in()
        });
        assert_eq!(labels(&model)[1], ("Open in File Explorer", false));
    }

    #[test]
    fn signed_out_leaves_open_and_quit_and_a_click_opens_sign_in() {
        let name = AppIdentity::CURRENT.product_name();
        let model = tray_model(&TrayInputs {
            explorer_ready: true,
            ..TrayInputs::default()
        });
        assert_eq!(
            labels(&model),
            vec![
                (&*format!("Open {name}"), true),
                ("Open in File Explorer", false),
                ("Pause syncing", false),
                ("Settings\u{2026}", false),
                (&*format!("Quit {name}"), true)
            ]
        );
        assert_eq!(model.left_click, LeftClick::OpenSignIn);
    }

    #[test]
    fn menu_ids_round_trip_and_unknown_ids_are_nothing() {
        for action in MenuAction::ALL {
            assert_eq!(MenuAction::from_id(action.id()), Some(action));
        }
        assert_eq!(MenuAction::from_id("delete-everything"), None);
        assert_eq!(MenuAction::from_id(""), None);
    }
}
