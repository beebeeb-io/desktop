//! Screens that show or take a secret (B11 ruling, DW-0009): the 12 words and the lock.
//!
//! - `auth_screen_private(on)`: the web view calls this while such a screen is showing. It does not hide the window.
//!   Hiding it from capture also hides it from the screen share the person is using to type the words, so the call
//!   only clears a leftover capture-exclusion flag (`WDA_NONE`). Elsewhere it is a no-op.
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
    use windows::Win32::UI::WindowsAndMessaging::{SetWindowDisplayAffinity, WDA_NONE};

    pub fn set(raw: isize, on: bool) -> Result<(), String> {
        // `on` stays in the command so the web view can still say the screen opened or closed.
        // The window stays in capture either way: exclusion removed it from the view the person watches.
        let _ = on;
        unsafe { SetWindowDisplayAffinity(HWND(raw as *mut c_void), WDA_NONE) }.map_err(|_| "internal".to_string())
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
    fn the_window_stays_in_capture_while_a_secret_is_on_screen() {
        let src = include_str!("private_screen.rs");
        let excluded = ["WDA_EXCLUDE", "FROMCAPTURE"].concat();
        assert!(!src.contains(&excluded), "that flag removes the window from the capture the person is watching");
        assert!(src.contains("WDA_NONE"));
    }

    #[test]
    fn the_clipboard_is_cleared_only_while_it_still_holds_the_copy() {
        let copied = digest("abandon ability able");
        assert!(still_ours(&copied, "abandon ability able"));
        assert!(!still_ours(&copied, "something the person copied since"));
        assert!(!still_ours(&copied, ""));
        assert_eq!(CLEAR_AFTER.as_secs(), 60);
    }
}
