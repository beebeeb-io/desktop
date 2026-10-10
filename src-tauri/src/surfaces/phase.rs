//! The popover's phase: ONE reducer decides what the popover body shows
//! (spec section 3 "State machine", section 9 "Popover phase").
//!
//! It replaces the UI-side guess `locked = logged_in && engine !== 'running'`,
//! which today says "Needs unlock" for a paused, offline or not-yet-started
//! engine. The function reads only the fields the snapshot command will return
//! (slice 2 builds them); it never reads the network, the keychain or a clock.
//!
//! Precedence, first match wins (spec section 3):
//! signed out, session ended, locked, Finder failed, Finder turned off in System
//! Settings, Finder adding, Finder missing, PAUSED (nothing is attempted while
//! paused, so no failure is shown), storage full, offline, error, syncing, synced.
//!
//! Two calls the spec left implicit, made here so they are tested:
//! - "Signed out or session ended" is two phases (states e and e2). Not logged
//!   in is checked first: with no session there is nothing to have ended.
//!   `auth_expired` is independent of `logged_in` in `sync_status` (a stale
//!   token can still be installed), so `SessionEnded` needs `logged_in`.
//! - A conflict is not a phase. It is a row in the list states (a1) and a badge
//!   on the menu-bar icon, so it does not appear here.
//! - Finder turned off in System Settings (`FinderUserDisabled`, added in slice
//!   2 by lead ruling) is its own phase, not `FinderMissing`: the spec's action
//!   for Missing is "Add to Finder", which cannot succeed while the user has the
//!   extension off. The copy and the action ("Open Login Items & Extensions",
//!   the neutral notice slice 5 built) are not in the approved design, so slice 3
//!   must have that state drawn before it renders one.

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FinderSetup {
    Ready,
    Missing,
    /// An install attempt is in flight (state f1).
    Adding,
    /// The last attempt failed (state f2).
    Failed,
    /// The user (or macOS) turned the Beebeeb File Provider off in System
    /// Settings. Not a failure of ours and not something "Add to Finder" can fix.
    UserDisabled,
}

impl FinderSetup {
    /// The snake_case name serde writes, for the lifecycle log.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Missing => "missing",
            Self::Adding => "adding",
            Self::Failed => "failed",
            Self::UserDisabled => "user_disabled",
        }
    }
}

/// Why Finder setup failed (spec 2026-10-06 §6.2). One closed vocabulary shared by the
/// reconciler, the popover snapshot, the lifecycle log, "Copy details" and the frontend
/// copy module (`src/finderSetupCopy.ts`). `UserDisabled` becomes the
/// `FinderSetup::UserDisabled` state, never `Failed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinderFailureReason {
    ExtensionLoading,
    UserDisabled,
    NotInApplications,
    FolderTaken,
    Signing,
    Timeout,
    /// Also what a reason this build does not know reads as (one a later build wrote, read after a downgrade): one
    /// unknown reason must never make all of `desktop.toml` unreadable.
    #[serde(other)]
    Unknown,
}

impl FinderFailureReason {
    pub const ALL: [Self; 7] = [
        Self::ExtensionLoading,
        Self::UserDisabled,
        Self::NotInApplications,
        Self::FolderTaken,
        Self::Signing,
        Self::Timeout,
        Self::Unknown,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExtensionLoading => "extension_loading",
            Self::UserDisabled => "user_disabled",
            Self::NotInApplications => "not_in_applications",
            Self::FolderTaken => "folder_taken",
            Self::Signing => "signing",
            Self::Timeout => "timeout",
            Self::Unknown => "unknown",
        }
    }
}

/// Why the engine cannot reach Beebeeb, when it cannot (spec section 9: NEW in
/// slice 2). `ServerDidNotAnswer` covers timeouts and 5xx; `Offline` is a
/// connection-level failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Connectivity {
    Online,
    Offline,
    ServerDidNotAnswer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct PopoverSnapshot {
    pub logged_in: bool,
    pub auth_expired: bool,
    pub vault_unlocked: bool,
    pub finder: FinderSetup,
    /// Why setup failed, alongside `finder` (spec 2026-10-06 §5.2); `PopoverSnapshot` stays `Copy`.
    pub finder_reason: Option<FinderFailureReason>,
    pub paused: bool,
    pub storage_full: bool,
    pub connectivity: Connectivity,
    /// Files are in flight (`sync_in_flight_count > 0`).
    pub syncing: bool,
    /// A failed Finder install is being shown by ANOTHER surface right now
    /// (`failure::finder_failure_shown_elsewhere`). Spec section 7: the popover
    /// shows f2 only while it is the surface that shows the failure, never both
    /// at once, so when this is set `FinderSetup::Failed` does not produce
    /// `FinderFailed`. Slice 2's snapshot builder fills it from the failure
    /// ledger and the registry.
    pub finder_failure_elsewhere: bool,
}

impl PopoverSnapshot {
    /// Signed in, unlocked, Finder ready, nothing wrong, nothing moving.
    pub const fn healthy() -> Self {
        Self {
            logged_in: true,
            auth_expired: false,
            vault_unlocked: true,
            finder: FinderSetup::Ready,
            finder_reason: None,
            paused: false,
            storage_full: false,
            connectivity: Connectivity::Online,
            syncing: false,
            finder_failure_elsewhere: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PopoverPhase {
    SignedOut,
    SessionEnded,
    Locked,
    FinderFailed,
    FinderUserDisabled,
    FinderAdding,
    FinderMissing,
    Paused,
    StorageFull,
    Offline,
    Error,
    Syncing,
    Synced,
}

/// The precedence order, highest first. Kept as data so the tests can walk it.
pub const PRECEDENCE: [PopoverPhase; 13] = [
    PopoverPhase::SignedOut,
    PopoverPhase::SessionEnded,
    PopoverPhase::Locked,
    PopoverPhase::FinderFailed,
    PopoverPhase::FinderUserDisabled,
    PopoverPhase::FinderAdding,
    PopoverPhase::FinderMissing,
    PopoverPhase::Paused,
    PopoverPhase::StorageFull,
    PopoverPhase::Offline,
    PopoverPhase::Error,
    PopoverPhase::Syncing,
    PopoverPhase::Synced,
];

pub fn popover_phase(s: &PopoverSnapshot) -> PopoverPhase {
    use PopoverPhase::*;
    if !s.logged_in {
        SignedOut
    } else if s.auth_expired {
        SessionEnded
    } else if !s.vault_unlocked {
        Locked
    } else if s.finder == FinderSetup::Failed && !s.finder_failure_elsewhere {
        FinderFailed
    } else if s.finder == FinderSetup::UserDisabled {
        FinderUserDisabled
    } else if s.finder == FinderSetup::Adding {
        FinderAdding
    } else if s.finder == FinderSetup::Missing || s.finder == FinderSetup::Failed {
        // A failed install that another surface is already reporting (spec
        // section 7) is still "Finder is not set up" here, so the popover
        // says that, plainly, and does not repeat the failure.
        FinderMissing
    } else if s.paused {
        Paused
    } else if s.storage_full {
        StorageFull
    } else if s.connectivity == Connectivity::Offline {
        Offline
    } else if s.connectivity == Connectivity::ServerDidNotAnswer {
        Error
    } else if s.syncing {
        Syncing
    } else {
        Synced
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use PopoverPhase::*;

    /// For every phase but `Synced`, the smallest change to a healthy snapshot
    /// that makes it the answer.
    fn trigger(phase: PopoverPhase) -> fn(&mut PopoverSnapshot) {
        match phase {
            SignedOut => |s| s.logged_in = false,
            SessionEnded => |s| s.auth_expired = true,
            Locked => |s| s.vault_unlocked = false,
            FinderFailed => |s| s.finder = FinderSetup::Failed,
            FinderUserDisabled => |s| s.finder = FinderSetup::UserDisabled,
            FinderAdding => |s| s.finder = FinderSetup::Adding,
            FinderMissing => |s| s.finder = FinderSetup::Missing,
            Paused => |s| s.paused = true,
            StorageFull => |s| s.storage_full = true,
            Offline => |s| s.connectivity = Connectivity::Offline,
            Error => |s| s.connectivity = Connectivity::ServerDidNotAnswer,
            Syncing => |s| s.syncing = true,
            Synced => |_| {},
        }
    }

    #[test]
    fn a_finder_failure_shown_by_another_surface_is_not_repeated_as_f2() {
        let failed = PopoverSnapshot {
            finder: FinderSetup::Failed,
            ..PopoverSnapshot::healthy()
        };
        assert_eq!(popover_phase(&failed), FinderFailed);
        let elsewhere = PopoverSnapshot {
            finder_failure_elsewhere: true,
            ..failed
        };
        assert_eq!(popover_phase(&elsewhere), FinderMissing);
        // The flag changes nothing unless the install actually failed...
        for finder in [FinderSetup::Ready, FinderSetup::Missing, FinderSetup::Adding] {
            let base = PopoverSnapshot {
                finder,
                ..PopoverSnapshot::healthy()
            };
            let flagged = PopoverSnapshot {
                finder_failure_elsewhere: true,
                ..base
            };
            assert_eq!(popover_phase(&flagged), popover_phase(&base), "{finder:?}");
        }
        // ...and it does not lift the higher phases.
        let locked = PopoverSnapshot {
            vault_unlocked: false,
            ..elsewhere
        };
        assert_eq!(popover_phase(&locked), Locked);
    }

    #[test]
    fn finder_turned_off_in_system_settings_is_its_own_phase_not_missing() {
        // Lead ruling (slice 2): "Add to Finder" cannot fix this, so it must not be
        // `FinderMissing`, whose action is exactly that.
        let off = PopoverSnapshot {
            finder: FinderSetup::UserDisabled,
            ..PopoverSnapshot::healthy()
        };
        assert_eq!(popover_phase(&off), FinderUserDisabled);
        // It is not a failure of ours, so another surface showing a failure changes nothing.
        let elsewhere = PopoverSnapshot {
            finder_failure_elsewhere: true,
            ..off
        };
        assert_eq!(popover_phase(&elsewhere), FinderUserDisabled);
        // It yields to the higher phases and outranks everything below the Finder group.
        let locked = PopoverSnapshot {
            vault_unlocked: false,
            ..off
        };
        assert_eq!(popover_phase(&locked), Locked);
        let busy = PopoverSnapshot {
            paused: true,
            storage_full: true,
            connectivity: Connectivity::Offline,
            syncing: true,
            ..off
        };
        assert_eq!(popover_phase(&busy), FinderUserDisabled);
    }

    #[test]
    fn a_healthy_snapshot_is_synced() {
        assert_eq!(popover_phase(&PopoverSnapshot::healthy()), Synced);
    }

    #[test]
    fn each_trigger_alone_yields_its_phase() {
        for phase in PRECEDENCE {
            let mut s = PopoverSnapshot::healthy();
            trigger(phase)(&mut s);
            assert_eq!(popover_phase(&s), phase, "{phase:?} alone");
        }
    }

    /// Phases that are values of the SAME snapshot field (`finder`,
    /// `connectivity`) cannot hold at once, so they have no pair to order.
    fn same_field(a: PopoverPhase, b: PopoverPhase) -> bool {
        let finder = [FinderFailed, FinderUserDisabled, FinderAdding, FinderMissing];
        let net = [Offline, Error];
        (finder.contains(&a) && finder.contains(&b)) || (net.contains(&a) && net.contains(&b))
    }

    #[test]
    fn the_higher_phase_always_wins_a_pair() {
        let mut pairs = 0;
        for (i, &high) in PRECEDENCE.iter().enumerate() {
            for &low in &PRECEDENCE[i + 1..] {
                if same_field(high, low) {
                    continue;
                }
                // Apply the lower trigger first, then the higher, and the other
                // way round: order of mutation must not matter.
                for flip in [false, true] {
                    let mut s = PopoverSnapshot::healthy();
                    if flip {
                        trigger(high)(&mut s);
                        trigger(low)(&mut s);
                    } else {
                        trigger(low)(&mut s);
                        trigger(high)(&mut s);
                    }
                    assert_eq!(popover_phase(&s), high, "{high:?} must beat {low:?}");
                }
                pairs += 1;
            }
        }
        // C(13, 2) = 78, minus 6 Finder pairs and 1 network pair.
        assert_eq!(pairs, 71);
    }

    #[test]
    fn everything_wrong_at_once_is_signed_out() {
        let mut s = PopoverSnapshot::healthy();
        for phase in PRECEDENCE {
            trigger(phase)(&mut s);
        }
        assert_eq!(popover_phase(&s), SignedOut);
    }

    #[test]
    fn paused_hides_every_failure_below_it() {
        // Nothing is attempted while paused, so no failure may be shown.
        let s = PopoverSnapshot {
            paused: true,
            storage_full: true,
            connectivity: Connectivity::ServerDidNotAnswer,
            syncing: true,
            ..PopoverSnapshot::healthy()
        };
        assert_eq!(popover_phase(&s), Paused);
        let offline = PopoverSnapshot {
            connectivity: Connectivity::Offline,
            ..s
        };
        assert_eq!(popover_phase(&offline), Paused);
    }

    #[test]
    fn finder_problems_outrank_paused_and_the_network_states() {
        let s = PopoverSnapshot {
            finder: FinderSetup::Missing,
            paused: true,
            connectivity: Connectivity::Offline,
            ..PopoverSnapshot::healthy()
        };
        assert_eq!(popover_phase(&s), FinderMissing);
    }

    #[test]
    fn a_stopped_or_unstarted_engine_is_not_locked() {
        // The UI-side guess this reducer replaces said "Needs unlock" here.
        // Unlocked and signed in, but nothing is moving: that is Synced.
        let s = PopoverSnapshot {
            logged_in: true,
            vault_unlocked: true,
            syncing: false,
            ..PopoverSnapshot::healthy()
        };
        assert_eq!(popover_phase(&s), Synced);
    }

    #[test]
    fn locked_needs_a_session_and_a_locked_vault() {
        let locked = PopoverSnapshot {
            vault_unlocked: false,
            ..PopoverSnapshot::healthy()
        };
        assert_eq!(popover_phase(&locked), Locked);
        let signed_out = PopoverSnapshot {
            logged_in: false,
            vault_unlocked: false,
            ..PopoverSnapshot::healthy()
        };
        assert_eq!(popover_phase(&signed_out), SignedOut);
    }

    #[test]
    fn session_ended_needs_a_stale_token_that_is_still_installed() {
        let s = PopoverSnapshot {
            auth_expired: true,
            ..PopoverSnapshot::healthy()
        };
        assert_eq!(popover_phase(&s), SessionEnded);
        // expired flag without a session: there is nothing to have ended.
        let gone = PopoverSnapshot {
            logged_in: false,
            auth_expired: true,
            ..PopoverSnapshot::healthy()
        };
        assert_eq!(popover_phase(&gone), SignedOut);
    }

    #[test]
    fn the_precedence_table_lists_every_phase_once() {
        let mut seen = std::collections::HashSet::new();
        for p in PRECEDENCE {
            assert!(seen.insert(format!("{p:?}")), "{p:?} listed twice");
        }
        assert_eq!(seen.len(), 13);
    }

    #[test]
    fn finder_names_match_what_serde_writes() {
        for setup in [
            FinderSetup::Ready,
            FinderSetup::Missing,
            FinderSetup::Adding,
            FinderSetup::Failed,
            FinderSetup::UserDisabled,
        ] {
            assert_eq!(
                serde_json::to_value(setup).unwrap(),
                serde_json::Value::String(setup.as_str().into())
            );
        }
        for reason in FinderFailureReason::ALL {
            assert_eq!(
                serde_json::to_value(reason).unwrap(),
                serde_json::Value::String(reason.as_str().into())
            );
            let back: FinderFailureReason =
                serde_json::from_value(serde_json::Value::String(reason.as_str().into())).unwrap();
            assert_eq!(back, reason);
        }
    }
}
