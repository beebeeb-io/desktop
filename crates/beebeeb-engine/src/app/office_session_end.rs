//! The one seam between the engine's session end and the Office host (which the engine cannot name): the host
//! registers ONE hook per state dir, and a vault lock, a server-ended close and a sign-out call it BEFORE the key
//! leaves memory, the recovery blobs are wiped and the scratch key ends ([`Engine::office_before_session_end`]).
//!
//! - [`SessionEnding::Lock`]: the host seals a recovery snapshot of every open edited document (it puts them through
//!   [`Engine::put_office_snapshot_for`], which needs the session still in memory), within a hard bound it owns.
//! - [`SessionEnding::ScreenLock`]: the screen is locked and the key stays. The lock is already published (nothing is
//!   on screen) when the hook is called, and it returns at once: the host begins the seal of every open edited
//!   document and finishes it in the background within its bound. The put needs the session, which a locked screen
//!   refuses to every other call, so the lock records a short-lived [`ScreenFlushGrant`] for the one session that was
//!   open: [`Engine::put_office_snapshot_for`] of that session works until the grant ends (an unlock, a lock, a
//!   sign-out or the bound), and nothing else does.
//! - [`SessionEnding::SignOut`]: the host ends its helpers and drops what they hold, so no snapshot lands after the
//!   wipe that follows. Nothing is snapshotted: sign-out deletes every blob.
//!
//! The hook can neither refuse nor fail the step; a panic in it is caught.
//!
//! Must not: hold a state of its own beyond the registered hook; log a name, a path or a byte.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use zeroize::Zeroizing;

use super::{CredentialStores, Engine};
use crate::account::SessionGeneration;
use beebeeb_api::flows::LockState;

/// How long after a screen lock a recovery snapshot of the session that was open is still sealed. Above the host's
/// own bound (1.5 s), so a late answer is refused by the host's deadline, not by this.
pub const SCREEN_FLUSH_GRANT: Duration = Duration::from_millis(2_500);

/// Which end of the session is coming.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionEnding {
    /// The key is about to leave memory (a vault lock, or the server ended the session).
    Lock,
    /// The account is about to be signed out.
    SignOut,
    /// The screen was just locked (the key stays in memory). The hook returns at once and seals in the background.
    ScreenLock,
}

/// What the host registers.
pub type SessionEndHook = Arc<dyn Fn(SessionEnding) + Send + Sync>;

/// One slot per engine: its state dir and the engine's own identity (the address of its account runtime, which no
/// other live engine shares). Two engines on one state dir (tests, a re-opened handle) never share a hook or a grant;
/// the slot is emptied when the engine drops ([`forget_slots`]).
pub(super) type Slot = (PathBuf, usize);

static HOOKS: Mutex<Option<HashMap<Slot, SessionEndHook>>> = Mutex::new(None);

/// What a screen lock lets through: recovery puts of the session `from` (the one open before the lock) while the
/// session's turn is still `to` (the lock's), the screen is still locked and `until` is not past. It holds a copy of
/// the key, because a locked screen lets no call read the session: the copy is zeroized when the grant ends (the
/// bound, a lock or a sign-out, whichever comes first).
struct ScreenFlushGrant {
    from: SessionGeneration,
    to: SessionGeneration,
    until: Instant,
    key: Zeroizing<[u8; 32]>,
}

/// The session that was open when the screen was locked ([`Engine::office_session_before_screen_lock`]).
pub(super) struct OpenBeforeLock {
    turn: SessionGeneration,
    key: Zeroizing<[u8; 32]>,
}

/// What the host registers to say how many open editors hold edits that no save has landed
/// ([`Engine::office_unsaved_count`]).
pub type UnsavedProbe = Arc<dyn Fn() -> usize + Send + Sync>;

static PROBES: Mutex<Option<HashMap<Slot, UnsavedProbe>>> = Mutex::new(None);

static GRANTS: Mutex<Option<HashMap<Slot, ScreenFlushGrant>>> = Mutex::new(None);

fn set_grant(state_dir: &Slot, grant: Option<ScreenFlushGrant>) {
    let mut all = GRANTS.lock().unwrap_or_else(|p| p.into_inner());
    match grant {
        Some(grant) => {
            all.get_or_insert_with(HashMap::new).insert(state_dir.clone(), grant);
        }
        None => {
            if let Some(all) = all.as_mut() {
                all.remove(state_dir);
            }
        }
    }
}

/// The bound is past: drop the grant of the turn `to` (a later lock's own grant stays).
fn end_grant_of(state_dir: &Slot, to: SessionGeneration) {
    let mut all = GRANTS.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(all) = all.as_mut()
        && all.get(state_dir).is_some_and(|g| g.to == to)
    {
        all.remove(state_dir);
    }
}

fn set_in(state_dir: &Slot, hook: SessionEndHook) {
    let mut all = HOOKS.lock().unwrap_or_else(|p| p.into_inner());
    all.get_or_insert_with(HashMap::new).insert(state_dir.clone(), hook);
}

fn run_in(state_dir: &Slot, ending: SessionEnding) {
    // Cloned out: the hook runs with no lock of this module held.
    let hook = HOOKS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .and_then(|all| all.get(state_dir).cloned());
    if let Some(hook) = hook {
        let _ = catch_unwind(AssertUnwindSafe(|| hook(ending)));
    }
}

/// An engine that goes takes its hook and grant (and the key copy in it) with it.
pub(super) fn forget_slots(slot: &Slot) {
    set_grant(slot, None);
    if let Some(all) = PROBES.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
        all.remove(slot);
    }
    let mut all = HOOKS.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(all) = all.as_mut() {
        all.remove(slot);
    }
}

impl<C: CredentialStores> Engine<C> {
    pub(super) fn office_slot(&self) -> Slot {
        (
            self.config.state_dir.clone(),
            Arc::as_ptr(&self.acct) as *const () as usize,
        )
    }

    /// Register the host's hook for this engine's state dir (a second call replaces the first).
    pub fn set_office_session_end_hook(&self, hook: SessionEndHook) {
        set_in(&self.office_slot(), hook);
    }

    /// Register the host's count of open editors with unsaved edits (a second call replaces the first). A sign-out
    /// is refused while it says more than none, unless the person chose to discard them.
    pub fn set_office_unsaved_probe(&self, probe: UnsavedProbe) {
        let mut all = PROBES.lock().unwrap_or_else(|p| p.into_inner());
        all.get_or_insert_with(HashMap::new).insert(self.office_slot(), probe);
    }

    /// How many open editors hold edits no save has landed (0 with no probe). Runs the probe with no lock held.
    pub(super) fn office_unsaved_count(&self) -> usize {
        let probe = PROBES
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .and_then(|all| all.get(&self.office_slot()).cloned());
        probe.map_or(0, |probe| catch_unwind(AssertUnwindSafe(|| probe())).unwrap_or(0))
    }

    /// Read BEFORE the screen lock is taken (a locked screen refuses the read): the open session's turn and key.
    pub(super) fn office_session_before_screen_lock(&self) -> Option<OpenBeforeLock> {
        self.person_session(|session, turn| OpenBeforeLock {
            turn,
            key: Zeroizing::new(session.master_key),
        })
        .ok()
    }

    /// Called by `lock_screen` once the lock is published. Records the short grant ([`ScreenFlushGrant`]) for the
    /// session read before the lock, and calls the hook with [`SessionEnding::ScreenLock`]; never waits.
    pub(super) fn office_after_screen_lock(&self, open: Option<OpenBeforeLock>) {
        if let Some(open) = open {
            let to = self.acct.session_generation();
            let dir = self.office_slot();
            let grant = ScreenFlushGrant {
                from: open.turn,
                to,
                until: Instant::now() + SCREEN_FLUSH_GRANT,
                key: open.key,
            };
            set_grant(&dir, Some(grant));
            // The copy of the key does not outlive the bound, whether or not anything used it.
            let _ = std::thread::Builder::new()
                .name("office-grant-end".into())
                .spawn(move || {
                    std::thread::sleep(SCREEN_FLUSH_GRANT);
                    end_grant_of(&dir, to);
                });
        }
        run_in(&self.office_slot(), SessionEnding::ScreenLock);
    }

    /// The key for a recovery put of `session` while a screen lock's grant holds, else `None`. The turn and the lock
    /// state are read in the hold every lock move takes, so an unlock, a lock or a sign-out ends it at once.
    pub(super) fn screen_flush_key(&self, session: SessionGeneration) -> Option<Zeroizing<[u8; 32]>> {
        let slot = self.office_slot();
        let all = GRANTS.lock().unwrap_or_else(|p| p.into_inner());
        let grant = all.as_ref()?.get(&slot)?;
        if grant.from != session || Instant::now() >= grant.until {
            return None;
        }
        let _writes = self.writes.try_read().ok()?;
        if self.lock_state() != LockState::ScreenLocked || self.acct.session_generation() != grant.to {
            return None;
        }
        Some(grant.key.clone())
    }

    /// Called by the lock and sign-out paths before anything of the session ends. Blocks as long as the hook does.
    pub(super) fn office_before_session_end(&self, ending: SessionEnding) {
        // A lock or a sign-out ends the screen lock's grant (and zeroizes its key copy) before anything else.
        let slot = self.office_slot();
        set_grant(&slot, None);
        run_in(&slot, ending);
    }
}

#[cfg(test)]
#[path = "office_session_end_tests.rs"]
mod office_session_end_tests;
