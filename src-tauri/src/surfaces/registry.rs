//! The macOS surface registry (two-surface spec `docs/specs/2026-10-24-macos-two-surface-app.md` §4, task 1683
//! slice 1). It replaced the four-kind model of the 30 Sep popover spec (popover, settings, onboarding, review).
//!
//! One entry per KIND, never per window. Asking for a kind that is live returns [`OpenAction::Show`] (focus it),
//! never a second window. Exactly two kinds exist on a Mac: the popover under the status item and the Beebeeb window
//! (`main`). Every other label (`tray`, `windows-onboarding`, `main-app`, `macos-settings`, `conflict-<id>`, and the
//! legacy `settings` and `onboarding` until their slices retire them) is refused by [`SurfaceKind::from_label`].
//!
//! The visibility rule (spec §4): opening the Beebeeb window hides the popover; opening the popover leaves the window
//! where it is (the popover floats above it and hides when it loses focus). So at most two are visible, and the
//! popover never hides the window.
//!
//! The Dock icon is not decided here: the one rule is `policy::dock_icon_wanted`, which `lib.rs` wires (review item
//! 10 on PR #134 removed this module's second, unwired copy).

use super::failure::Visible;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SurfaceKind {
    Popover,
    Main,
}

/// What closing a surface does: the popover is hidden (the one idle webview, reused); the Beebeeb window is
/// destroyed (spec §4: "destroyed when closed", so no webview stays alive for it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosePolicy {
    Hide,
    Destroy,
}

/// The two labels the macOS build may create (spec §4). Anything else is a bug.
pub const MACOS_LABELS: [&str; 2] = ["popover", "main"];

impl SurfaceKind {
    pub const ALL: [SurfaceKind; 2] = [SurfaceKind::Popover, SurfaceKind::Main];

    pub const fn label(self) -> &'static str {
        match self {
            SurfaceKind::Popover => "popover",
            SurfaceKind::Main => "main",
        }
    }

    pub fn from_label(label: &str) -> Option<SurfaceKind> {
        SurfaceKind::ALL.into_iter().find(|k| k.label() == label)
    }

    pub const fn close_policy(self) -> ClosePolicy {
        match self {
            SurfaceKind::Popover => ClosePolicy::Hide,
            SurfaceKind::Main => ClosePolicy::Destroy,
        }
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
    /// Surfaces that must be hidden first: the popover when the Beebeeb window opens. Hidden, never destroyed.
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

    /// Ask for a surface. Records the result, so calling it twice is the "opening a kind twice yields one window"
    /// contract. Opening the Beebeeb window hides a visible popover; opening the popover hides nothing.
    pub fn open(&mut self, kind: SurfaceKind) -> OpenPlan {
        let mut hide = Vec::new();
        if kind == SurfaceKind::Main
            && let Some(visible) = self.live.get_mut(&SurfaceKind::Popover)
            && *visible
        {
            *visible = false;
            hide.push(SurfaceKind::Popover);
        }
        let action = if self.live.contains_key(&kind) {
            OpenAction::Show
        } else {
            OpenAction::Create
        };
        self.live.insert(kind, true);
        OpenPlan { kind, action, hide }
    }

    /// The user closed the surface (red button, ⌘W, Esc or blur for the popover). Returns what to do to the window,
    /// or `None` if no such window was live.
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

    /// Which surfaces are visible, in the form the failure reducer takes. Its `settings` is the Beebeeb window,
    /// whose Home and Settings › Sync show the Finder row inline; there is no other window on a Mac.
    pub fn visible(&self) -> Visible {
        Visible {
            popover: self.is_visible(SurfaceKind::Popover),
            settings: self.is_visible(SurfaceKind::Main),
            other_window: false,
        }
    }

    pub fn live_labels(&self) -> Vec<&'static str> {
        self.live.keys().map(|k| k.label()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use SurfaceKind::*;

    #[test]
    fn exactly_two_labels_and_they_round_trip() {
        // Two-surface spec §4: `MACOS_LABELS` is `["popover", "main"]`.
        assert_eq!(MACOS_LABELS, ["popover", "main"]);
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
            "macos-settings",
            "settings",
            "onboarding",
            "review",
            "conflict-7f3a",
            "",
            "Main",
            "main ",
            "popover-2",
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
    fn a_hidden_popover_is_shown_again_not_rebuilt() {
        let mut r = SurfaceRegistry::new();
        r.open(Popover);
        assert_eq!(r.close(Popover), Some(ClosePolicy::Hide));
        assert!(r.is_live(Popover) && !r.is_visible(Popover), "the one idle webview");
        assert_eq!(r.open(Popover).action, OpenAction::Show);
        assert_eq!(r.live_count(), 1);
    }

    #[test]
    fn the_beebeeb_window_is_destroyed_on_close_and_rebuilt_on_open() {
        let mut r = SurfaceRegistry::new();
        r.open(Main);
        assert_eq!(r.close(Main), Some(ClosePolicy::Destroy));
        assert_eq!(r.live_count(), 0, "a closed Beebeeb window frees its webview");
        assert_eq!(r.open(Main).action, OpenAction::Create);
    }

    #[test]
    fn closing_something_that_is_not_open_does_nothing() {
        let mut r = SurfaceRegistry::new();
        assert_eq!(r.close(Main), None);
        assert_eq!(r.live_count(), 0);
    }

    #[test]
    fn opening_the_beebeeb_window_hides_the_popover() {
        let mut r = SurfaceRegistry::new();
        r.open(Popover);
        let plan = r.open(Main);
        assert_eq!(plan.hide, vec![Popover]);
        assert!(!r.is_visible(Popover) && r.is_live(Popover), "hidden, not destroyed");
        assert!(r.is_visible(Main));
    }

    #[test]
    fn opening_the_popover_leaves_the_beebeeb_window_where_it_is() {
        // Spec §4: the popover floats above the window and hides when it loses focus.
        let mut r = SurfaceRegistry::new();
        r.open(Main);
        let plan = r.open(Popover);
        assert!(plan.hide.is_empty(), "{plan:?}");
        assert!(r.is_visible(Main) && r.is_visible(Popover));
        assert_eq!(r.visible_count(), 2);
    }

    #[test]
    fn at_most_two_surfaces_exist_and_the_window_is_never_hidden_by_the_popover() {
        // Every sequence of up to 6 operations over 2 kinds x {open, close}.
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
                let main_was_visible = r.is_visible(Main);
                if open {
                    r.open(kind);
                } else {
                    r.close(kind);
                }
                assert!(r.live_count() <= 2, "{seq:?}");
                if kind == Popover && main_was_visible {
                    assert!(r.is_visible(Main), "{seq:?}: the popover hid the Beebeeb window");
                }
                if kind == Main && open {
                    assert!(!r.is_visible(Popover), "{seq:?}: the popover stayed over a window that just opened");
                }
            }
            sequences += 1;
            if seq.len() < 6 {
                for i in 0..ops.len() {
                    let mut next = seq.clone();
                    next.push(i);
                    stack.push(next);
                }
            }
        }
        // 1 + 4 + 4^2 + ... + 4^6: proves the loop walked them all.
        assert_eq!(sequences, 5_461);
    }

    #[test]
    fn visible_reports_exactly_what_is_showing() {
        // The failure reducer's `settings` is "a Beebeeb window that shows the Finder row inline": the Beebeeb window
        // (Home's Finder row, Settings › Sync). Nothing else is a window on a Mac any more.
        let none = Visible::default();
        assert_eq!(SurfaceRegistry::new().visible(), none);
        let mut r = SurfaceRegistry::new();
        r.open(Popover);
        assert_eq!(r.visible(), Visible { popover: true, ..none });
        r.open(Main);
        assert_eq!(r.visible(), Visible { settings: true, ..none });
        r.open(Popover);
        assert_eq!(
            r.visible(),
            Visible {
                popover: true,
                settings: true,
                other_window: false
            }
        );
    }

    #[test]
    fn close_policy_table() {
        assert_eq!(Popover.close_policy(), ClosePolicy::Hide);
        assert_eq!(Main.close_policy(), ClosePolicy::Destroy);
    }
}
