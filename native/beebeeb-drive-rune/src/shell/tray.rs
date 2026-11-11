//! The b in the notification area, as a pure model: which icon, which tooltip, which menu, and
//! what a left click does (DW-1230, DW-1231, DW-1231b). The OS half is `platform::TrayIcon`.
//!
//! The precedence and the words are `src-tauri/src/tray_window.rs`'s `tray_menu_state`, so 0.9
//! says what 0.8.x said for the same state. The menu is the board's (DW-1231), not 0.8.x's.

/// What the sync engine last said, as far as the b needs it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EngineState {
    #[default]
    Idle,
    Syncing,
    Offline,
    /// Stopped: an error the engine could not get past, or storage full.
    Error,
}

/// What the b reads. The session lane and the engine fill this; see `ShellActions::tray_inputs`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrayInputs {
    pub signed_in: bool,
    pub locked: bool,
    pub paused: bool,
    /// Beebeeb is in File Explorer (the sync root is registered).
    pub explorer_ready: bool,
    pub update_available: bool,
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
    /// A red ! badge: stopped, storage full, signed out.
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
        label: &'static str,
        enabled: bool,
    },
    Separator,
}

/// What a left click on the b does (DW-1201: "Signed out: a click on the b opens the window on
/// sign-in instead of the flyout").
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
    /// "Beebeeb" plus the state; Narrator reads it.
    pub tooltip: &'static str,
    pub menu: Vec<MenuEntry>,
    pub left_click: LeftClick,
}

pub fn tray_model(inputs: &TrayInputs) -> TrayModel {
    use TrayIconState as I;
    let (icon, label, tooltip) = if !inputs.signed_in {
        (I::Stopped, "Signed out", "Beebeeb: not signed in")
    } else if inputs.locked {
        // Locked keeps the plain b: it needs no attention from the notification area.
        (I::UpToDate, "Locked", "Beebeeb: locked")
    } else if inputs.paused {
        (I::Paused, "Paused", "Beebeeb: paused")
    } else if inputs.engine == EngineState::Offline {
        (I::Offline, "Offline", "Beebeeb: no connection")
    } else if inputs.engine == EngineState::Error {
        (I::Stopped, "Stopped and storage full", "Beebeeb: stopped")
    } else if inputs.conflicts > 0 {
        (I::NeedsDecision, "Needs a decision", "Beebeeb: needs a decision")
    } else if inputs.update_available {
        (I::UpToDate, "Update available", "Beebeeb: update available")
    } else if inputs.engine == EngineState::Syncing || inputs.files_remaining.unwrap_or(0) > 0 {
        (I::Syncing, "Syncing", "Beebeeb: syncing")
    } else {
        (I::UpToDate, "Up to date", "Beebeeb: up to date")
    };
    let item = |action, label, enabled| MenuEntry::Item { action, label, enabled };
    let pause = if inputs.paused {
        item(MenuAction::ResumeSyncing, "Resume syncing", inputs.signed_in)
    } else {
        item(MenuAction::PauseSyncing, "Pause syncing", inputs.signed_in)
    };
    let menu = vec![
        item(MenuAction::OpenBeebeeb, "Open Beebeeb", true),
        // Disabled while Beebeeb is not in File Explorer (DW-1215, DW-1216).
        item(
            MenuAction::OpenInFileExplorer,
            "Open in File Explorer",
            inputs.signed_in && inputs.explorer_ready,
        ),
        MenuEntry::Separator,
        pause,
        // Settings is unavailable until signed in (DW-A02).
        item(MenuAction::Settings, "Settings\u{2026}", inputs.signed_in),
        MenuEntry::Separator,
        // Quit lives only here, and always works.
        item(MenuAction::Quit, "Quit Beebeeb", true),
    ];
    let left_click = if inputs.signed_in {
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
    fn labels(model: &TrayModel) -> Vec<(&'static str, bool)> {
        model
            .menu
            .iter()
            .filter_map(|e| match e {
                MenuEntry::Item { label, enabled, .. } => Some((*label, *enabled)),
                MenuEntry::Separator => None,
            })
            .collect()
    }

    #[test]
    fn each_state_has_its_icon_and_tooltip() {
        let cases: [(TrayInputs, TrayIconState, &str); 9] = [
            (TrayInputs::default(), TrayIconState::Stopped, "Beebeeb: not signed in"),
            (
                TrayInputs {
                    locked: true,
                    ..signed_in()
                },
                TrayIconState::UpToDate,
                "Beebeeb: locked",
            ),
            (
                TrayInputs {
                    paused: true,
                    ..signed_in()
                },
                TrayIconState::Paused,
                "Beebeeb: paused",
            ),
            (
                TrayInputs {
                    engine: EngineState::Offline,
                    ..signed_in()
                },
                TrayIconState::Offline,
                "Beebeeb: no connection",
            ),
            (
                TrayInputs {
                    engine: EngineState::Error,
                    ..signed_in()
                },
                TrayIconState::Stopped,
                "Beebeeb: stopped",
            ),
            (
                TrayInputs {
                    conflicts: 2,
                    ..signed_in()
                },
                TrayIconState::NeedsDecision,
                "Beebeeb: needs a decision",
            ),
            (
                TrayInputs {
                    update_available: true,
                    ..signed_in()
                },
                TrayIconState::UpToDate,
                "Beebeeb: update available",
            ),
            (
                TrayInputs {
                    engine: EngineState::Syncing,
                    ..signed_in()
                },
                TrayIconState::Syncing,
                "Beebeeb: syncing",
            ),
            (signed_in(), TrayIconState::UpToDate, "Beebeeb: up to date"),
        ];
        for (inputs, icon, tooltip) in cases {
            let model = tray_model(&inputs);
            assert_eq!((model.icon, model.tooltip), (icon, tooltip), "{inputs:?}");
            assert!(model.tooltip.starts_with("Beebeeb: "));
        }
        // Files still to move count as syncing even before the engine says so.
        let model = tray_model(&TrayInputs {
            files_remaining: Some(3),
            ..signed_in()
        });
        assert_eq!(model.icon, TrayIconState::Syncing);
    }

    #[test]
    fn the_first_state_in_the_order_wins() {
        let everything = TrayInputs {
            signed_in: true,
            locked: true,
            paused: true,
            explorer_ready: true,
            update_available: true,
            conflicts: 4,
            engine: EngineState::Offline,
            files_remaining: Some(9),
        };
        assert_eq!(tray_model(&everything).label, "Locked");
        assert_eq!(
            tray_model(&TrayInputs {
                locked: false,
                ..everything.clone()
            })
            .label,
            "Paused"
        );
        assert_eq!(
            tray_model(&TrayInputs {
                locked: false,
                paused: false,
                ..everything.clone()
            })
            .label,
            "Offline"
        );
        let online = TrayInputs {
            locked: false,
            paused: false,
            engine: EngineState::Syncing,
            ..everything.clone()
        };
        assert_eq!(tray_model(&online).label, "Needs a decision");
        assert_eq!(
            tray_model(&TrayInputs {
                conflicts: 0,
                ..online.clone()
            })
            .label,
            "Update available"
        );
        assert_eq!(
            tray_model(&TrayInputs {
                conflicts: 0,
                update_available: false,
                ..online
            })
            .label,
            "Syncing"
        );
        assert_eq!(
            tray_model(&TrayInputs {
                signed_in: false,
                ..everything
            })
            .label,
            "Signed out"
        );
    }

    #[test]
    fn the_menu_is_the_boards_five_items_in_order() {
        let model = tray_model(&signed_in());
        assert_eq!(
            labels(&model),
            vec![
                ("Open Beebeeb", true),
                ("Open in File Explorer", true),
                ("Pause syncing", true),
                ("Settings\u{2026}", true),
                ("Quit Beebeeb", true)
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
            MenuEntry::Item {
                action,
                label: "Resume syncing",
                ..
            } => Some(*action),
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
        let model = tray_model(&TrayInputs {
            explorer_ready: true,
            ..TrayInputs::default()
        });
        assert_eq!(
            labels(&model),
            vec![
                ("Open Beebeeb", true),
                ("Open in File Explorer", false),
                ("Pause syncing", false),
                ("Settings\u{2026}", false),
                ("Quit Beebeeb", true)
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
