//! Screens that show or take a secret (B11 ruling, DW-0009): the 12 words and the lock.
//!
//! - `auth_screen_private(on)`: while such a screen is shown, the window is left out of screenshots, screen sharing
//!   and the Task View / Alt+Tab thumbnails (`SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`). Windows-only code;
//!   it has only been checked on Linux, so it needs the lead's native Windows rung. Elsewhere it is a no-op.
//! - [`copy_for_60s`]: Rust writes a secret to the clipboard itself and clears it after 60 seconds if the clipboard
//!   still holds it. The web view never runs the copy and never reads the clipboard. Only a SHA-256 of the copied text
//!   is kept for the comparison, never the text.
use std::time::Duration;

use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

/// How long a copied secret stays on the clipboard.
pub(crate) const CLEAR_AFTER: Duration = Duration::from_secs(60);

#[tauri::command]
pub(crate) fn auth_screen_private(window: tauri::WebviewWindow, on: bool) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let hwnd = window.hwnd().map_err(|_| "internal".to_string())?;
        imp::set(hwnd.0 as isize, on)?;
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (&window, on);
    }
    Ok(())
}

#[cfg(target_os = "windows")]
mod imp {
    use std::ffi::c_void;

    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_NONE};

    pub fn set(raw: isize, on: bool) -> Result<(), String> {
        let affinity = if on { WDA_EXCLUDEFROMCAPTURE } else { WDA_NONE };
        unsafe { SetWindowDisplayAffinity(HWND(raw as *mut c_void), affinity) }.map_err(|_| "internal".to_string())
    }
}

fn digest(text: &str) -> [u8; 32] {
    Sha256::digest(text.as_bytes()).into()
}

/// True when the clipboard still holds what was copied (compared by digest only).
pub(crate) fn still_ours(copied: &[u8; 32], now: &str) -> bool {
    !now.is_empty() && digest(now) == *copied
}

/// Put `text` on the clipboard from Rust; clear it after [`CLEAR_AFTER`] if it is still there.
pub(crate) fn copy_for_60s(app: &tauri::AppHandle, text: Zeroizing<String>) -> Result<(), String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    let copied = digest(&text);
    app.clipboard()
        .write_text(text.as_str().to_owned())
        .map_err(|_| "clipboard".to_string())?;
    drop(text);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(CLEAR_AFTER).await;
        if let Ok(now) = app.clipboard().read_text() {
            let now = Zeroizing::new(now);
            if still_ours(&copied, &now) {
                let _ = app.clipboard().clear();
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
        let copied = digest("abandon ability able");
        assert!(still_ours(&copied, "abandon ability able"));
        assert!(!still_ours(&copied, "something the person copied since"));
        assert!(!still_ours(&copied, ""));
        assert_eq!(CLEAR_AFTER.as_secs(), 60);
    }
}
