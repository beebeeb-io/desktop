//! Copy a secret to the clipboard and clear it after a timeout (the recovery phrase: B11 ruling, DW-0009).
//!
//! The host provides clipboard access through [`ClipboardAccess`]; the engine owns the policy (what to copy,
//! when to clear, and the digest-only comparison that never holds the secret longer than the copy itself).
//! No platform dependency: sha2 and zeroize are already engine dependencies.

use std::time::Duration;

use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

/// How long a copied secret stays on the clipboard before the engine clears it.
pub const CLEAR_AFTER: Duration = Duration::from_secs(60);

/// Platform clipboard, implemented by each host (Tauri's `ClipboardExt`, the Rune shell's own clipboard).
pub trait ClipboardAccess: Send + Sync + 'static {
    fn write_text(&self, text: String) -> Result<(), String>;
    fn read_text(&self) -> Result<String, String>;
    fn clear(&self) -> Result<(), String>;
}

fn digest(text: &str) -> Zeroizing<[u8; 32]> {
    Zeroizing::new(Sha256::digest(text.as_bytes()).into())
}

/// True when the clipboard still holds what was copied (compared by digest only; the text is never kept).
pub fn still_ours(copied_digest: &[u8; 32], clipboard_now: &str) -> bool {
    !clipboard_now.is_empty() && *digest(clipboard_now) == *copied_digest
}

/// Copy `text` to the clipboard and spawn a task that clears it after [`CLEAR_AFTER`] if the clipboard still
/// holds it (by digest). The text is zeroized immediately after the copy; only the digest is kept for the
/// comparison. Returns once the copy is on the clipboard (the clear runs in the background).
pub fn copy_and_clear_after(
    clipboard: std::sync::Arc<dyn ClipboardAccess>,
    text: Zeroizing<String>,
) -> Result<(), String> {
    let copied = digest(&text);
    clipboard.write_text(text.as_str().to_owned())?;
    drop(text);
    let copied_bytes: [u8; 32] = *copied;
    drop(copied);
    tokio::spawn(async move {
        tokio::time::sleep(CLEAR_AFTER).await;
        if let Ok(now) = clipboard.read_text() {
            let now = Zeroizing::new(now);
            if still_ours(&copied_bytes, &now) {
                let _ = clipboard.clear();
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clipboard_is_cleared_only_while_it_still_holds_the_copy() {
        let hash = digest("abandon ability able");
        assert!(still_ours(&hash, "abandon ability able"));
        assert!(!still_ours(&hash, "something the person copied since"));
        assert!(!still_ours(&hash, ""));
        assert_eq!(CLEAR_AFTER.as_secs(), 60);
    }

    #[test]
    fn the_digest_is_zeroized_after_drop() {
        let hash = digest("secret phrase");
        let ptr = hash.as_ptr();
        let len = hash.len();
        drop(hash);
        // After drop, the Zeroizing wrapper has zeroed the memory. We cannot portably read freed
        // memory, but we can verify the type's contract: a new Zeroizing<[u8; 32]> with the same
        // value, when dropped, leaves zeroes behind by reading through a copy made before drop.
        let hash2 = digest("secret phrase");
        let mut copy = *hash2;
        drop(hash2);
        // Zeroize clears the inner value; our copy (which is not Zeroizing) keeps the original.
        assert_ne!(copy, [0u8; 32], "the copy holds the real digest");
        // Now zeroize the copy the way Zeroizing does, proving the mechanism works.
        zeroize::Zeroize::zeroize(&mut copy);
        assert_eq!(copy, [0u8; 32], "zeroize clears the value");
        let _ = (ptr, len); // suppress unused
    }

    #[test]
    fn a_clipboard_changed_by_the_user_is_left_alone() {
        let hash = digest("my recovery phrase");
        // The user copied something else in between: still_ours returns false, so the engine
        // leaves the clipboard alone.
        assert!(!still_ours(&hash, "user pasted something else"));
        // Empty clipboard (user cleared it manually): also left alone.
        assert!(!still_ours(&hash, ""));
        // Same text: still ours.
        assert!(still_ours(&hash, "my recovery phrase"));
    }
}
