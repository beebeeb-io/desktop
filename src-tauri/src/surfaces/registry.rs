//! The macOS surface registry (spec section 2 and section 6 rules 3, 6, 7, 10).
//!
//! The model slice 6 mirrors onto real windows: one entry per KIND, never per
//! window. Asking for a kind that is live returns [`OpenAction::Show`] (focus
//! it), never a second window. Only the four kinds below may exist on macOS;
//! the labels `tray`, `windows-onboarding`, `main-app` and `conflict-<id>` are
//! removed from the macOS config in slice 6 and [`SurfaceKind::from_label`]
//! refuses them.
//!
//! At most ONE surface is visible at a time (spec section 2, "Visible at the
//! same time: 1"; device rung 8, "at most one window visible"): opening any
//! kind hides whichever other one is showing. Slice 1 originally hid only the
//! popover, which left settings, onboarding and review free to stack up to three
//! deep, the P3 window sprawl this task exists to remove. The consequence for
//! the popover (a tray click hides a visible Settings window) is recorded for
//! Guus in `.claude/tasks/decisions/1683-s1-spec-reconciliations.md`.

use super::failure::Visible;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SurfaceKind {
    Popover,
    Settings,
    Onboarding,
    Review,
}

/// What closing a surface does (spec rule 10): popover and settings are hidden
/// (they reopen instantly and Settings keeps its state); onboarding and review
/// are destroyed (transient, and a destroyed window frees its WKWebView).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosePolicy {
    Hide,
    Destroy,
}

/// The four labels the macOS build may create. Anything else is a bug.
pub const MACOS_LABELS: [&str; 4] = ["popover", "settings", "onboarding", "review"];

impl SurfaceKind {
    pub const ALL: [SurfaceKind; 4] = [
        SurfaceKind::Popover,
        SurfaceKind::Settings,
        SurfaceKind::Onboarding,
        SurfaceKind::Review,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            SurfaceKind::Popover => "popover",
            SurfaceKind::Settings => "settings",
            SurfaceKind::Onboarding => "onboarding",
            SurfaceKind::Review => "review",
        }
    }

    pub fn from_label(label: &str) -> Option<SurfaceKind> {
        SurfaceKind::ALL.into_iter().find(|k| k.label() == label)
    }

    pub const fn close_policy(self) -> ClosePolicy {
        match self {
            SurfaceKind::Popover | SurfaceKind::Settings => ClosePolicy::Hide,
            SurfaceKind::Onboarding | SurfaceKind::Review => ClosePolicy::Destroy,
        }
    }

    /// Spec rule 7 (ruling 2): a Dock icon exists only while Settings,
    /// onboarding or review is open. The popover never has one.
    pub const fn needs_dock_icon(self) -> bool {
        !matches!(self, SurfaceKind::Popover)
    }
}

pub fn is_allowed_macos_label(label: &str) -> bool {
    SurfaceKind::from_label(label).is_some()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAction {
    /// No window of this kind exists: build one.
    Create,
    /// One exists (visible or hidden): show and focus it. Never build another.
    Show,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenPlan {
    pub kind: SurfaceKind,
    pub action: OpenAction,
    /// Surfaces that must be hidden first. Spec section 2 ("Visible at the same
    /// time: 1") and rule 3: opening a surface hides every other visible one.
    /// They are HIDDEN, never destroyed, even onboarding and review (whose
    /// close policy is destroy): superseding a window is not the user closing
    /// it, and destroying it would throw away work in progress. Ordered by
    /// kind so the list is deterministic.
    pub hide: Vec<SurfaceKind>,
}

#[derive(Debug, Default)]
pub struct SurfaceRegistry {
    /// kind -> visible. Present means a window of that kind exists.
    live: BTreeMap<SurfaceKind, bool>,
}

impl SurfaceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask for a surface. Records the result, so calling it twice is the
    /// "opening a kind twice yields one window" contract, and leaves `kind` as
    /// the ONE visible surface (every other visible kind is in `hide`).
    pub fn open(&mut self, kind: SurfaceKind) -> OpenPlan {
        let mut hide = Vec::new();
        for (other, visible) in self.live.iter_mut() {
            if *other != kind && *visible {
                *visible = false;
                hide.push(*other);
            }
        }
        let action = if self.live.contains_key(&kind) {
            OpenAction::Show
        } else {
            OpenAction::Create
        };
        self.live.insert(kind, true);
        OpenPlan { kind, action, hide }
    }

    /// The user closed the surface (red button, Esc, blur). Returns what to do
    /// to the window, or `None` if no such window was live.
    pub fn close(&mut self, kind: SurfaceKind) -> Option<ClosePolicy> {
        if !self.live.contains_key(&kind) {
            return None;
        }
        let policy = kind.close_policy();
        match policy {
            ClosePolicy::Hide => {
                self.live.insert(kind, false);
            }
            ClosePolicy::Destroy => {
                self.live.remove(&kind);
            }
        }
        Some(policy)
    }

    pub fn is_live(&self, kind: SurfaceKind) -> bool {
        self.live.contains_key(&kind)
    }

    pub fn is_visible(&self, kind: SurfaceKind) -> bool {
        self.live.get(&kind).copied().unwrap_or(false)
    }

    /// Windows that exist (hidden ones included): the WKWebView count.
    pub fn live_count(&self) -> usize {
        self.live.len()
    }

    pub fn visible_count(&self) -> usize {
        self.live.values().filter(|v| **v).count()
    }

    /// Which app windows are visible, in the form the failure reducer takes.
    pub fn visible(&self) -> Visible {
        Visible {
            popover: self.is_visible(SurfaceKind::Popover),
            settings: self.is_visible(SurfaceKind::Settings),
            other_window: self.is_visible(SurfaceKind::Onboarding) || self.is_visible(SurfaceKind::Review),
        }
    }

    pub fn live_labels(&self) -> Vec<&'static str> {
        self.live.keys().map(|k| k.label()).collect()
    }

    /// Whether the app should currently have a Dock icon (activation policy
    /// `regular`) rather than `accessory`.
    pub fn dock_icon_wanted(&self) -> bool {
        self.live.iter().any(|(k, visible)| *visible && k.needs_dock_icon())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use SurfaceKind::*;

    #[test]
    fn exactly_four_labels_and_they_round_trip() {
        assert_eq!(MACOS_LABELS.len(), 4);
        for (kind, label) in SurfaceKind::ALL.into_iter().zip(MACOS_LABELS) {
            assert_eq!(kind.label(), label);
            assert_eq!(SurfaceKind::from_label(label), Some(kind));
            assert!(is_allowed_macos_label(label));
        }
    }

    #[test]
    fn every_other_label_is_refused() {
        for label in [
            "tray",
            "windows-onboarding",
            "main-app",
            "conflict",
            "conflict-7f3a",
            "main",
            "",
            "Popover",
            "settings ",
            "review-2",
        ] {
            assert_eq!(SurfaceKind::from_label(label), None, "{label:?}");
            assert!(!is_allowed_macos_label(label), "{label:?}");
        }
    }

    #[test]
    fn opening_a_kind_twice_yields_one_window() {
        for kind in SurfaceKind::ALL {
            let mut r = SurfaceRegistry::new();
            assert_eq!(r.open(kind).action, OpenAction::Create, "{kind:?} first open");
            assert_eq!(r.open(kind).action, OpenAction::Show, "{kind:?} second open");
            assert_eq!(r.open(kind).action, OpenAction::Show, "{kind:?} third open");
            assert_eq!(r.live_count(), 1, "{kind:?} must have exactly one window");
        }
    }

    #[test]
    fn a_hidden_popover_or_settings_is_shown_again_not_rebuilt() {
        for kind in [Popover, Settings] {
            let mut r = SurfaceRegistry::new();
            r.open(kind);
            assert_eq!(r.close(kind), Some(ClosePolicy::Hide));
            assert!(r.is_live(kind) && !r.is_visible(kind));
            assert_eq!(r.open(kind).action, OpenAction::Show, "{kind:?}");
            assert_eq!(r.live_count(), 1);
        }
    }

    #[test]
    fn onboarding_and_review_are_destroyed_on_close_and_rebuilt_on_open() {
        for kind in [Onboarding, Review] {
            let mut r = SurfaceRegistry::new();
            r.open(kind);
            assert_eq!(r.close(kind), Some(ClosePolicy::Destroy));
            assert_eq!(r.live_count(), 0, "{kind:?} must free its webview");
            assert_eq!(r.open(kind).action, OpenAction::Create, "{kind:?}");
        }
    }

    #[test]
    fn closing_something_that_is_not_open_does_nothing() {
        let mut r = SurfaceRegistry::new();
        assert_eq!(r.close(Settings), None);
        assert_eq!(r.live_count(), 0);
    }

    #[test]
    fn the_most_windows_that_can_ever_exist_is_one_per_kind() {
        let mut r = SurfaceRegistry::new();
        for _ in 0..3 {
            for kind in SurfaceKind::ALL {
                r.open(kind);
            }
        }
        assert_eq!(r.live_count(), 4);
        assert!(r.live_labels().iter().all(|l| is_allowed_macos_label(l)));
    }

    #[test]
    fn opening_any_other_window_hides_the_popover() {
        for kind in [Settings, Onboarding, Review] {
            let mut r = SurfaceRegistry::new();
            r.open(Popover);
            let plan = r.open(kind);
            assert_eq!(plan.hide, vec![Popover], "{kind:?}");
            assert!(!r.is_visible(Popover));
            assert!(r.is_visible(kind));
            assert_eq!(r.visible_count(), 1, "one visible surface after opening {kind:?}");
        }
    }

    #[test]
    fn opening_the_popover_hides_a_visible_settings_window() {
        // Spec section 2: "Visible at the same time: 1". The popover is a
        // surface like any other, so it does not sit on top of Settings.
        let mut r = SurfaceRegistry::new();
        r.open(Settings);
        let plan = r.open(Popover);
        assert_eq!(plan.hide, vec![Settings]);
        assert!(!r.is_visible(Settings) && r.is_live(Settings), "hidden, not destroyed");
        assert_eq!(r.visible_count(), 1);
    }

    #[test]
    fn opening_a_window_hides_every_other_visible_window_not_only_the_popover() {
        // Starts from Settings, not from the popover: the slice-1 review case.
        // Settings, then review, then onboarding must never show 3 at once.
        let mut r = SurfaceRegistry::new();
        r.open(Settings);
        let to_review = r.open(Review);
        assert_eq!(to_review.hide, vec![Settings]);
        assert_eq!(r.visible_count(), 1);
        let to_onboarding = r.open(Onboarding);
        assert_eq!(to_onboarding.hide, vec![Review]);
        assert_eq!(r.visible_count(), 1);
        assert!(r.is_visible(Onboarding));
        // Superseded windows are hidden, never destroyed: no in-progress
        // onboarding or review is lost because the user opened something else.
        assert_eq!(r.live_count(), 3);
        // ...and coming back to one is a Show, not a rebuild.
        assert_eq!(r.open(Settings).action, OpenAction::Show);
        assert_eq!(r.visible_count(), 1);
    }

    #[test]
    fn at_most_one_surface_is_visible_after_any_sequence_of_opens_and_closes() {
        // Every sequence of up to 5 operations over 4 kinds x {open, close}.
        let ops: Vec<(SurfaceKind, bool)> = SurfaceKind::ALL
            .into_iter()
            .flat_map(|k| [(k, true), (k, false)])
            .collect();
        let mut sequences = 0usize;
        let mut stack: Vec<Vec<usize>> = vec![vec![]];
        while let Some(seq) = stack.pop() {
            let mut r = SurfaceRegistry::new();
            for &i in &seq {
                let (kind, open) = ops[i];
                if open {
                    r.open(kind);
                } else {
                    r.close(kind);
                }
                assert!(r.visible_count() <= 1, "{seq:?} left {} visible", r.visible_count());
            }
            sequences += 1;
            if seq.len() < 5 {
                for i in 0..ops.len() {
                    let mut next = seq.clone();
                    next.push(i);
                    stack.push(next);
                }
            }
        }
        // 1 + 8 + 8^2 + ... + 8^5: proves the loop walked them all.
        assert_eq!(sequences, 37_449);
    }

    #[test]
    fn a_hidden_popover_is_not_reported_as_hidden_again() {
        let mut r = SurfaceRegistry::new();
        r.open(Popover);
        r.close(Popover);
        assert!(r.open(Settings).hide.is_empty());
    }

    #[test]
    fn dock_icon_only_while_settings_onboarding_or_review_is_visible() {
        let mut r = SurfaceRegistry::new();
        assert!(!r.dock_icon_wanted(), "nothing open");
        r.open(Popover);
        assert!(!r.dock_icon_wanted(), "popover alone has no Dock icon");
        r.open(Settings);
        assert!(r.dock_icon_wanted());
        r.close(Settings); // hidden, still live
        assert!(
            !r.dock_icon_wanted(),
            "a hidden Settings window must not keep the Dock icon"
        );
        r.open(Review);
        assert!(r.dock_icon_wanted());
        r.close(Review); // destroyed
        assert!(!r.dock_icon_wanted());
        r.open(Onboarding);
        assert!(r.dock_icon_wanted());
        r.close(Onboarding);
        assert!(!r.dock_icon_wanted());
    }

    #[test]
    fn visible_reports_exactly_what_is_showing() {
        let none = Visible::default();
        assert_eq!(SurfaceRegistry::new().visible(), none);
        for (kind, expected) in [
            (Popover, Visible { popover: true, ..none }),
            (Settings, Visible { settings: true, ..none }),
            (
                Onboarding,
                Visible {
                    other_window: true,
                    ..none
                },
            ),
            (
                Review,
                Visible {
                    other_window: true,
                    ..none
                },
            ),
        ] {
            let mut r = SurfaceRegistry::new();
            r.open(kind);
            assert_eq!(r.visible(), expected, "{kind:?} open");
            r.open(Popover);
            r.close(Popover);
            assert_eq!(
                r.visible(),
                none,
                "{kind:?} hidden by the popover, which was then closed"
            );
        }
    }

    #[test]
    fn close_policy_table() {
        assert_eq!(Popover.close_policy(), ClosePolicy::Hide);
        assert_eq!(Settings.close_policy(), ClosePolicy::Hide);
        assert_eq!(Onboarding.close_policy(), ClosePolicy::Destroy);
        assert_eq!(Review.close_policy(), ClosePolicy::Destroy);
    }
}
