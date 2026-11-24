//! Explorer's "Free up space" command (task 2490): Disabled and freeing nothing unless the engine handle hands out
//! the local state; no handle, or one that is gone, answers the same. The engine's own answers are
//! `app/explorer_api_tests.rs` (which holds the one test of the process's slot).

use std::sync::atomic::{AtomicU32, Ordering};

use super::*;

/// An engine handle that answers `refusal` and counts the reclaims it was asked for.
struct Owner {
    refusal: Option<LocalStateRefused>,
    reclaims: AtomicU32,
}

impl Owner {
    fn new(refusal: Option<LocalStateRefused>) -> Self {
        Owner {
            refusal,
            reclaims: AtomicU32::new(0),
        }
    }
}

impl ReclaimOwner for Owner {
    fn refusal(&self) -> Option<LocalStateRefused> {
        self.refusal
    }
    fn free_up_space(&self) -> Result<u64, String> {
        self.reclaims.fetch_add(1, Ordering::SeqCst);
        Ok(4_096)
    }
}

#[test]
fn the_command_frees_only_while_the_engine_hands_out_the_local_state() {
    let refused = [
        LocalStateRefused::SignedOut,
        LocalStateRefused::Locked,
        LocalStateRefused::SessionEnded,
        LocalStateRefused::OtherAccount,
        LocalStateRefused::Unowned,
        LocalStateRefused::IdentityUnknown,
        LocalStateRefused::NothingSynced,
        LocalStateRefused::Unreadable,
    ];
    for refusal in refused {
        let owner = Owner::new(Some(refusal));
        assert_eq!(state_of(Some(&owner)), CommandState::Disabled, "{refusal}");
        assert_eq!(invoke(Some(&owner)), Invoked::Refused(refusal.kind()), "{refusal}");
        assert_eq!(
            owner.reclaims.load(Ordering::SeqCst),
            0,
            "{refusal}: nothing was asked to free"
        );
    }
    let owner = Owner::new(None);
    assert_eq!(state_of(Some(&owner)), CommandState::Enabled);
    assert_eq!(invoke(Some(&owner)), Invoked::Freed(4_096));
    assert_eq!(owner.reclaims.load(Ordering::SeqCst), 1);
}

#[test]
fn without_an_engine_handle_the_command_is_disabled() {
    assert_eq!(state_of(None), CommandState::Disabled);
    assert_eq!(invoke(None), Invoked::Refused("no_engine"));
}
