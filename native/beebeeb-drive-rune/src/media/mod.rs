//! In-app playback of audio and video, decrypted in memory only (decision 2539; the media plan,
//! `briefs/plan-and-build/media/PLAN.md`).
//!
//! **The boundary** (the plan, (d); decision D-MED-3). This process holds the keys, the audio device and its clock,
//! the OS's media controls and the file's name, and it parses nothing of the file. Every container, codec and tag
//! is parsed by `beebeeb-media-helper`, a second process started confined (`beebeeb-office-sandbox`: an
//! AppContainer with no capabilities, in a job object). Between the two run byte ranges (only the ones the helper
//! asked for, of the ONE file it was started for), RGBA frames of the size this side asked for, f32 PCM, two short
//! tags and a small picture, and codes (`beebeeb-media-protocol`, whose decoder checks every length before it
//! allocates). The helper gets no name, no id and no path. Whatever it answers is checked here against what was
//! asked; anything else kills it and shows "This file could not be played." with "Open in another app…".
//!
//! **No decrypted byte on disk, ever:** no temporary file, no cache file, no artwork file, and no log line with a
//! name, an id, a path, a title, a tag or a byte. Decrypted bytes are held in `Zeroizing` buffers.
//!
//! - [`player`]: the ONE player of the shell ([`set_shell_player`]); it outlives the viewer (music keeps playing when
//!   the view is left) and owns the helper, the sound, the frames and the OS's media controls of what plays. The
//!   seams its tests use instead of a device, a helper or a window are traits there.
//! - [`transport`]: the confined helper and its two pipes.
//! - [`audio_out`]: the sound device, and the clock everything else follows.
//! - [`os_controls`]: Windows' media controls (DW-1504): the title, the artist and the picture while it plays,
//!   from memory, never a path; cleared at once on a stop.
//! - [`video`]: the frames, into the toolkit's pixels.
//!
//! **A lock, a sign-out or an account change** stops playback within a second ([`stop_all`], from the shell's poll):
//! the helper killed, the sound dropped, the chunk cache wiped, the OS's media session cleared, the last frame
//! blanked.
//!
//! The scaffold: every type and signature the media slices call; [`set_shell_player`], [`shell_player`] and
//! [`stop_all`] are real, the rest answers "nothing plays" until its slice fills it.

// The scaffold's stubs have no caller until the slices that bind them (M2, M7, M8) land.
#![allow(dead_code)]

pub mod audio_out;
pub mod os_controls;
pub mod player;
pub mod thumb_frame;
pub mod transport;
pub mod video;

use std::sync::{Mutex, PoisonError};

use crate::shell::app::GateState;
use player::PlayerHandle;

/// Why playback stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    /// The person closed it (a video's view left, Esc).
    Closed,
    /// The app locked.
    Locked,
    /// The account signed out.
    SignedOut,
    /// Another account is in the window now.
    AccountChanged,
    /// The helper broke the protocol, timed out or failed.
    Failed,
}

impl StopReason {
    /// Why playback stops when the session closed under the window (`shell::host`'s poll): a sign-out, another
    /// account, a lock. Any other gate the session went to (no key on this PC, no plan, the credential store
    /// unanswered, the first-run steps) is the same account's session closing: as a lock.
    pub fn at_session_end(gate: GateState, other_account: bool) -> StopReason {
        match gate {
            GateState::SignedOut => StopReason::SignedOut,
            _ if other_account => StopReason::AccountChanged,
            _ => StopReason::Locked,
        }
    }
}

/// The shell's one player. Nobody adds another global: the media slices reach it through these three functions.
static SHELL: Mutex<Option<PlayerHandle>> = Mutex::new(None);

/// The shell's player from now on. One already there (the engine of another account) is stopped first.
pub fn set_shell_player(h: PlayerHandle) {
    let before = SHELL.lock().unwrap_or_else(PoisonError::into_inner).replace(h.clone());
    if let Some(before) = before.filter(|before| !before.same_as(&h)) {
        before.stop(StopReason::AccountChanged);
    }
}

/// The shell's player, if one was set.
pub fn shell_player() -> Option<PlayerHandle> {
    SHELL.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// Stops whatever plays (the shell's poll on a lock, a sign-out or an account change).
pub fn stop_all(reason: StopReason) {
    if let Some(h) = shell_player() {
        h.stop(reason)
    }
}
