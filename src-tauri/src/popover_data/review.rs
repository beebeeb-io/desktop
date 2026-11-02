//! The review window's retarget event: `review:open`, its camelCase payload and
//! `ReviewMode`.
//!
//! One `review` window exists, so pointing it at another file is an event, not a new
//! window; `ConflictWindow` reads the payload once at mount and remounts keyed by
//! `fileId`.

use super::*;

/// Event the popover (or a notification click) sends to point the single `review`
/// window at another file (spec section 6, rule 9). `ConflictWindow` reads
/// `fileId`, `fileName` and `isText` from its URL once at mount, so one window
/// needs a way to be retargeted: it handles this event and remounts keyed by
/// `fileId`. The keys are camelCase, like the URL parameters they replace.
pub const REVIEW_OPEN_EVENT: &str = "review:open";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewMode {
    Conflict,
    Versions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewOpen {
    pub file_id: String,
    pub file_name: String,
    pub is_text: bool,
    pub mode: ReviewMode,
}

impl ReviewOpen {
    /// `None` when the event cannot point anywhere (no file id), so a malformed
    /// request never blanks the window.
    #[cfg(test)]
    pub fn new(file_id: &str, file_name: &str, is_text: bool, mode: ReviewMode) -> Option<Self> {
        if file_id.trim().is_empty() {
            return None;
        }
        Some(Self {
            file_id: file_id.to_string(),
            file_name: if file_name.is_empty() {
                "Unknown file".to_string()
            } else {
                file_name.to_string()
            },
            is_text,
            mode,
        })
    }
}

/// Send the retarget event. The `review` window that handles it is slice 6's.
#[allow(dead_code)]
pub fn emit_review_open(app: &tauri::AppHandle, payload: &ReviewOpen) -> tauri::Result<()> {
    use tauri::Emitter;
    app.emit(REVIEW_OPEN_EVENT, payload)
}
