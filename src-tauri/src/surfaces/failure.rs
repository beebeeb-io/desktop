//! One failure, one visible error surface (spec section 7).
//!
//! The defect this encodes (screenshot 1): `install_finder_location` failed by
//! BOTH persisting the message (rendered as a banner) and returning `Err`
//! (rendered as a toast). Two sources, one string, nothing de-duplicated them.
//! Slice 5 fixed the source on today's UI; this reducer is the rule the new
//! popover and Settings are built on, so the two sources can no longer become
//! two surfaces:
//!
//! - A failure has an id. Reporting the same id again REPLACES the report (two
//!   sources for one failure fold into one entry). Starting a new attempt or
//!   succeeding clears it.
//! - [`place`] maps a failure to at most ONE surface for the windows that are
//!   visible right now. A Finder failure is shown in the surface where the user
//!   started it while that surface is visible, otherwise in the one that is, and
//!   never in both.
//! - Decision D1 (kept as recorded): a failure that GATES a control is inline
//!   (Finder install, whole-engine state, a file row, the vault password); only
//!   a transient failure that gates nothing is a toast; a background event is a
//!   native notification and never a toast in an app window.

use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// Add to Finder / Repair. Gates "Open in Finder", so it is inline.
    FinderInstall,
    /// The engine as a whole (states d, d2, s).
    EngineWhole,
    /// One file could not sync: a row in the activity list.
    FileRow,
    /// Wrong vault password: the inline line in state c1b.
    VaultPassword,
    /// A one-off action that gates nothing (a toggle that did not save).
    TransientAction,
    /// A background event with no user action (conflict found, update ready).
    Background,
}

/// The surface where the user started the action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Popover,
    Settings,
}

/// Which app windows are visible right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Visible {
    pub popover: bool,
    pub settings: bool,
    /// Onboarding or review.
    pub other_window: bool,
}

impl Visible {
    fn any(&self) -> bool {
        self.popover || self.settings || self.other_window
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorSurface {
    /// The popover's body state (f2, d, d2 or s).
    PopoverState,
    /// An inline line under the row in Settings.
    SettingsRow,
    /// A row in the popover's activity list.
    ActivityRow,
    /// The inline line in the unlock body (c1b).
    UnlockInline,
    Toast,
    NativeNotification,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureReport {
    pub id: String,
    pub kind: FailureKind,
    pub origin: Option<Origin>,
}

impl FailureReport {
    pub fn new(id: impl Into<String>, kind: FailureKind, origin: Option<Origin>) -> Self {
        Self {
            id: id.into(),
            kind,
            origin,
        }
    }
}

/// The one place a failure may be shown, given what is visible. `None` means
/// "no surface yet": it is rendered when one opens, never as a toast.
pub fn place(kind: FailureKind, origin: Option<Origin>, vis: Visible) -> Option<ErrorSurface> {
    match kind {
        FailureKind::FinderInstall => match (vis.popover, vis.settings) {
            (true, true) => Some(if origin == Some(Origin::Popover) {
                ErrorSurface::PopoverState
            } else {
                ErrorSurface::SettingsRow
            }),
            (false, true) => Some(ErrorSurface::SettingsRow),
            (true, false) => Some(ErrorSurface::PopoverState),
            (false, false) => None,
        },
        FailureKind::EngineWhole => vis.popover.then_some(ErrorSurface::PopoverState),
        FailureKind::FileRow => vis.popover.then_some(ErrorSurface::ActivityRow),
        FailureKind::VaultPassword => vis.popover.then_some(ErrorSurface::UnlockInline),
        FailureKind::TransientAction => vis.any().then_some(ErrorSurface::Toast),
        FailureKind::Background => Some(ErrorSurface::NativeNotification),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureEvent {
    Failed(FailureReport),
    AttemptStarted { id: String },
    Succeeded { id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    pub id: String,
    pub surface: ErrorSurface,
}

#[derive(Debug, Default)]
pub struct FailureLedger {
    reports: BTreeMap<String, FailureReport>,
}

impl FailureLedger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply(&mut self, event: FailureEvent) {
        match event {
            FailureEvent::Failed(report) => {
                self.reports.insert(report.id.clone(), report);
            }
            FailureEvent::AttemptStarted { id } | FailureEvent::Succeeded { id } => {
                self.reports.remove(&id);
            }
        }
    }

    pub fn len(&self) -> usize {
        self.reports.len()
    }

    /// Every error surface on screen. At most one entry per failure id.
    pub fn rendered(&self, vis: Visible) -> Vec<Rendered> {
        self.reports
            .values()
            .filter_map(|r| {
                place(r.kind, r.origin, vis).map(|surface| Rendered {
                    id: r.id.clone(),
                    surface,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ErrorSurface::*;
    use FailureKind::*;

    const ALL_KINDS: [FailureKind; 6] = [
        FinderInstall,
        EngineWhole,
        FileRow,
        VaultPassword,
        TransientAction,
        Background,
    ];

    fn all_visibility() -> Vec<Visible> {
        let mut v = Vec::new();
        for popover in [false, true] {
            for settings in [false, true] {
                for other_window in [false, true] {
                    v.push(Visible {
                        popover,
                        settings,
                        other_window,
                    });
                }
            }
        }
        v
    }

    #[test]
    fn screenshot_one_two_sources_for_one_finder_failure_render_one_surface() {
        // The persisted `finder_install_last_error` and the returned `Err` are
        // two reports of the same failure id.
        let mut ledger = FailureLedger::new();
        let failed = FailureReport::new("finder-install", FinderInstall, Some(Origin::Settings));
        ledger.apply(FailureEvent::Failed(failed.clone())); // persisted banner
        ledger.apply(FailureEvent::Failed(failed)); // returned error
        assert_eq!(ledger.len(), 1);

        let vis = Visible {
            settings: true,
            ..Visible::default()
        };
        assert_eq!(
            ledger.rendered(vis),
            vec![Rendered {
                id: "finder-install".into(),
                surface: SettingsRow
            }]
        );
    }

    #[test]
    fn a_finder_failure_is_never_a_toast_and_never_in_two_surfaces() {
        for origin in [Some(Origin::Popover), Some(Origin::Settings)] {
            for vis in all_visibility() {
                let mut ledger = FailureLedger::new();
                let r = FailureReport::new("finder-install", FinderInstall, origin);
                // Reported by both sources, twice each, as the old code did.
                for _ in 0..4 {
                    ledger.apply(FailureEvent::Failed(r.clone()));
                }
                let shown = ledger.rendered(vis);
                assert!(shown.len() <= 1, "{origin:?} {vis:?}: {shown:?}");
                assert!(shown.iter().all(|s| s.surface != Toast), "{origin:?} {vis:?}");
            }
        }
    }

    #[test]
    fn a_finder_failure_shows_where_it_started_while_that_surface_is_visible() {
        let both = Visible {
            popover: true,
            settings: true,
            other_window: false,
        };
        assert_eq!(place(FinderInstall, Some(Origin::Popover), both), Some(PopoverState));
        assert_eq!(place(FinderInstall, Some(Origin::Settings), both), Some(SettingsRow));
        // Started in the popover, then Settings opened (the popover hid): it
        // moves to the surface that is on screen, still exactly one.
        let settings_only = Visible {
            settings: true,
            ..Visible::default()
        };
        assert_eq!(
            place(FinderInstall, Some(Origin::Popover), settings_only),
            Some(SettingsRow)
        );
        // Nothing visible: no surface, and above all no toast.
        assert_eq!(place(FinderInstall, Some(Origin::Popover), Visible::default()), None);
    }

    #[test]
    fn gating_failures_are_inline_and_never_a_toast() {
        for kind in [FinderInstall, EngineWhole, FileRow, VaultPassword] {
            for vis in all_visibility() {
                for origin in [None, Some(Origin::Popover), Some(Origin::Settings)] {
                    let s = place(kind, origin, vis);
                    assert_ne!(s, Some(Toast), "{kind:?} {origin:?} {vis:?}");
                    assert_ne!(s, Some(NativeNotification), "{kind:?} {origin:?} {vis:?}");
                }
            }
        }
    }

    #[test]
    fn each_gating_failure_has_its_one_home() {
        let popover = Visible {
            popover: true,
            ..Visible::default()
        };
        assert_eq!(place(EngineWhole, None, popover), Some(PopoverState));
        assert_eq!(place(FileRow, None, popover), Some(ActivityRow));
        assert_eq!(place(VaultPassword, None, popover), Some(UnlockInline));
        // Not visible: nowhere (the menu-bar badge is ambient, not a surface).
        assert_eq!(place(EngineWhole, None, Visible::default()), None);
        let settings = Visible {
            settings: true,
            ..Visible::default()
        };
        assert_eq!(place(EngineWhole, None, settings), None);
    }

    #[test]
    fn a_transient_failure_that_gates_nothing_is_a_toast_when_a_window_is_open() {
        for vis in all_visibility() {
            let expected = if vis.any() { Some(Toast) } else { None };
            assert_eq!(place(TransientAction, Some(Origin::Settings), vis), expected, "{vis:?}");
        }
    }

    #[test]
    fn a_background_event_is_a_native_notification_never_a_toast() {
        for vis in all_visibility() {
            assert_eq!(place(Background, None, vis), Some(NativeNotification), "{vis:?}");
        }
    }

    #[test]
    fn a_new_attempt_clears_the_failure() {
        let mut ledger = FailureLedger::new();
        ledger.apply(FailureEvent::Failed(FailureReport::new(
            "finder-install",
            FinderInstall,
            Some(Origin::Popover),
        )));
        let vis = Visible {
            popover: true,
            ..Visible::default()
        };
        assert_eq!(ledger.rendered(vis).len(), 1);
        ledger.apply(FailureEvent::AttemptStarted {
            id: "finder-install".into(),
        });
        assert_eq!(ledger.rendered(vis), vec![]);
        assert_eq!(ledger.len(), 0);
    }

    #[test]
    fn success_clears_the_failure_and_clearing_an_unknown_id_is_harmless() {
        let mut ledger = FailureLedger::new();
        ledger.apply(FailureEvent::Failed(FailureReport::new("engine", EngineWhole, None)));
        ledger.apply(FailureEvent::Succeeded { id: "engine".into() });
        ledger.apply(FailureEvent::Succeeded {
            id: "never-failed".into(),
        });
        assert_eq!(ledger.len(), 0);
    }

    #[test]
    fn distinct_failures_each_get_one_surface() {
        // Two files fail: two rows, each one surface. Not merged, not doubled.
        let mut ledger = FailureLedger::new();
        for id in ["file:a", "file:b"] {
            ledger.apply(FailureEvent::Failed(FailureReport::new(id, FileRow, None)));
            ledger.apply(FailureEvent::Failed(FailureReport::new(id, FileRow, None)));
        }
        let vis = Visible {
            popover: true,
            ..Visible::default()
        };
        let shown = ledger.rendered(vis);
        assert_eq!(shown.len(), 2);
        assert!(shown.iter().all(|s| s.surface == ActivityRow));
    }

    #[test]
    fn exhaustive_no_failure_ever_has_two_surfaces_across_every_kind_and_visibility() {
        let mut combos = 0;
        for kind in ALL_KINDS {
            for origin in [None, Some(Origin::Popover), Some(Origin::Settings)] {
                for vis in all_visibility() {
                    let mut ledger = FailureLedger::new();
                    let r = FailureReport::new("x", kind, origin);
                    ledger.apply(FailureEvent::Failed(r.clone()));
                    ledger.apply(FailureEvent::Failed(r));
                    assert!(ledger.rendered(vis).len() <= 1, "{kind:?} {origin:?} {vis:?}");
                    combos += 1;
                }
            }
        }
        assert_eq!(combos, 6 * 3 * 8);
    }
}
