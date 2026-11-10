//! The one notification gate and the one hop to the OS toast.
//!
//! Settings › Notifications (slice 13) stores one switch per event. Before this module every switch controlled nothing:
//! the three old call sites each read one key and none read `notify_allow`, and the other six events had no path at
//! all. This module is the single decision point (`should_notify`) and the single delivery point (`notify_event_impl`).
//!
//! The gate is pure: the persisted config plus the OS's own switch go in, send or suppress with a reason code comes
//! out. The TypeScript twin (`src/v2/sections/settings/shell/notificationsGate.ts`) is what the window uses to decide
//! whether to ask for a toast; this Rust copy is what the engine-side listener uses, and the command re-checks it so a
//! caller cannot bypass the switches.
//!
//! Security rules for the hop:
//! - a toast never carries a token, a path or a folder name;
//! - a file name is used only while the vault is unlocked, and only when it is a bare name (no separators, no `..`);
//! - a signed-out app sends nothing.

use serde::{Deserialize, Serialize};

use crate::config::DesktopConfig;

/// The events the pane names, in the pane's order. `file_updated` is the pane's "File updates" row (stored as
/// `notify_conflicts`); `sync_finished` is "Sync finished" (stored as `notify_sync_complete`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationEvent {
    SharedWithYou,
    FileUpdated,
    StorageWarning,
    SyncStopped,
    NeedsDecision,
    SyncFinished,
    LocalSpace,
    BackupFinished,
    BackupAttention,
}

impl NotificationEvent {
    /// Every event, so a test can prove the table is complete.
    pub(crate) const ALL: [NotificationEvent; 9] = [
        NotificationEvent::SharedWithYou,
        NotificationEvent::FileUpdated,
        NotificationEvent::StorageWarning,
        NotificationEvent::SyncStopped,
        NotificationEvent::NeedsDecision,
        NotificationEvent::SyncFinished,
        NotificationEvent::LocalSpace,
        NotificationEvent::BackupFinished,
        NotificationEvent::BackupAttention,
    ];

    /// The config key this event reads. One row per event, so a new event cannot silently reuse another's switch.
    pub(crate) fn switch(self) -> fn(&NotificationConfig) -> bool {
        match self {
            NotificationEvent::SharedWithYou => |c| c.notify_shared,
            NotificationEvent::FileUpdated => |c| c.notify_conflicts,
            NotificationEvent::StorageWarning => |c| c.notify_quota_warnings,
            NotificationEvent::SyncStopped => |c| c.notify_sync_stopped,
            NotificationEvent::NeedsDecision => |c| c.notify_needs_decision,
            NotificationEvent::SyncFinished => |c| c.notify_sync_complete,
            NotificationEvent::LocalSpace => |c| c.notify_local_space,
            NotificationEvent::BackupFinished => |c| c.notify_backup_finished,
            NotificationEvent::BackupAttention => |c| c.notify_backup_attention,
        }
    }

    /// The toast's title. Short, no file name, no path.
    pub(crate) fn title(self) -> &'static str {
        match self {
            NotificationEvent::SharedWithYou => "Shared with you",
            NotificationEvent::FileUpdated => "File updated",
            NotificationEvent::StorageWarning => "Storage warning",
            NotificationEvent::SyncStopped => "Sync stopped",
            NotificationEvent::NeedsDecision => "Files need a decision",
            NotificationEvent::SyncFinished => "All caught up",
            NotificationEvent::LocalSpace => "Space on this PC",
            NotificationEvent::BackupFinished => "Backup finished",
            NotificationEvent::BackupAttention => "Backup needs attention",
        }
    }
}

/// The switches the gate reads, copied out of `DesktopConfig` so the gate is pure and a test needs no file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotificationConfig {
    pub notify_allow: bool,
    pub notify_shared: bool,
    pub notify_conflicts: bool,
    pub notify_quota_warnings: bool,
    pub notify_sync_stopped: bool,
    pub notify_needs_decision: bool,
    pub notify_sync_complete: bool,
    pub notify_local_space: bool,
    pub notify_backup_finished: bool,
    pub notify_backup_attention: bool,
}

impl From<&DesktopConfig> for NotificationConfig {
    fn from(config: &DesktopConfig) -> Self {
        Self {
            notify_allow: config.notify_allow,
            notify_shared: config.notify_shared,
            notify_conflicts: config.notify_conflicts,
            notify_quota_warnings: config.notify_quota_warnings,
            notify_sync_stopped: config.notify_sync_stopped,
            notify_needs_decision: config.notify_needs_decision,
            notify_sync_complete: config.notify_sync_complete,
            notify_local_space: config.notify_local_space,
            notify_backup_finished: config.notify_backup_finished,
            notify_backup_attention: config.notify_backup_attention,
        }
    }
}

/// Why a notification was not sent. Stable codes, so a caller can log or test them without matching copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuppressReason {
    OsOff,
    AllowOff,
    PreferenceOff,
}

impl SuppressReason {
    pub(crate) fn code(self) -> &'static str {
        match self {
            SuppressReason::OsOff => "os_off",
            SuppressReason::AllowOff => "allow_off",
            SuppressReason::PreferenceOff => "preference_off",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Send,
    Suppress(SuppressReason),
}

/// Send or suppress one event. The OS switch wins first (Beebeeb cannot turn Windows' own switch back on), then Allow,
/// then the event's own switch.
pub(crate) fn should_notify(event: NotificationEvent, config: &NotificationConfig, os_allowed: bool) -> Decision {
    if !os_allowed {
        return Decision::Suppress(SuppressReason::OsOff);
    }
    if !config.notify_allow {
        return Decision::Suppress(SuppressReason::AllowOff);
    }
    if !(event.switch())(config) {
        return Decision::Suppress(SuppressReason::PreferenceOff);
    }
    Decision::Send
}

/// A file name is safe to show only when it is a bare name: no separators, no parent hop, not empty, not a path.
pub(crate) fn safe_file_name(name: &str) -> Option<&str> {
    let name = name.trim();
    if name.is_empty() || name.len() > 255 {
        return None;
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return None;
    }
    Some(name)
}

/// The toast's body. A file name is used only for the three events that are about a file, only when the vault is
/// unlocked, and only when it is a bare name. Every other event has fixed copy.
pub(crate) fn body_for(event: NotificationEvent, file_name: Option<&str>) -> String {
    let name = file_name.and_then(safe_file_name);
    match event {
        NotificationEvent::SharedWithYou => match name {
            Some(name) => format!("{name} was shared with you."),
            None => "Someone shared a file with you.".to_string(),
        },
        NotificationEvent::FileUpdated => match name {
            Some(name) => format!("{name} changed on another device."),
            None => "A file changed on another device.".to_string(),
        },
        NotificationEvent::StorageWarning => "You're close to your storage limit.".to_string(),
        NotificationEvent::SyncStopped => "Some files couldn't upload. Nothing was lost.".to_string(),
        NotificationEvent::NeedsDecision => match name {
            Some(name) => format!("{name} changed in two places. Open Beebeeb to choose."),
            None => "A file changed in two places. Open Beebeeb to choose.".to_string(),
        },
        NotificationEvent::SyncFinished => "Your Beebeeb files are up to date.".to_string(),
        NotificationEvent::LocalSpace => "Downloaded files are near their limit.".to_string(),
        NotificationEvent::BackupFinished => "Your backup completed.".to_string(),
        NotificationEvent::BackupAttention => "Open Beebeeb to continue the backup.".to_string(),
    }
}

/// The one hop: gate, then one native toast. `file_name` is dropped while the vault is locked, and a signed-out app
/// sends nothing. The OS switch is the window's `offInWindows`; the engine-side listener cannot read it, so it passes
/// `true` and Windows itself drops a toast when the person turned Beebeeb off there.
pub(crate) fn notify_event_impl<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    state: &crate::AppState,
    event: NotificationEvent,
    file_name: Option<&str>,
    os_allowed: bool,
) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt;

    let config = DesktopConfig::load().unwrap_or_default();
    let decision = should_notify(event, &NotificationConfig::from(&config), os_allowed);
    if let Decision::Suppress(reason) = decision {
        tracing::debug!(event = ?event, reason = reason.code(), "notification suppressed");
        return Ok(());
    }

    let signed_in = crate::account_commands::vault_unlocked(state)
        || state.auth_present.lock().map(|guard| *guard).unwrap_or(false);
    if !signed_in {
        tracing::debug!(event = ?event, reason = "signed_out", "notification suppressed");
        return Ok(());
    }

    let unlocked = crate::account_commands::vault_unlocked(state);
    let body = body_for(event, file_name.filter(|_| unlocked));
    app.notification()
        .builder()
        .title(event.title())
        .body(body)
        .show()
        .map_err(|error| error.to_string())
}

/// The IPC command. The window calls it for the events it raises; the engine-side listener calls the impl directly.
#[tauri::command]
pub(crate) async fn notify_event(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
    event: NotificationEvent,
    file_name: Option<String>,
) -> Result<(), String> {
    notify_event_impl(&app, &state, event, file_name.as_deref(), true)
}

/// Fire "Sync stopped" once when the engine settles into paused or stopped. The runner emits `engine-status` on every
/// tick; only the transition into a stopped state is a notification, so a paused engine does not toast every second.
pub(crate) fn attach_engine_status_listener<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    use tauri::{Listener, Manager};

    let app_for_listener = app.clone();
    let last = std::sync::Arc::new(std::sync::Mutex::new(String::from("running")));
    app.listen(crate::engine_status::EVENT, move |event| {
        let Ok(payload) = serde_json::from_str::<serde_json::Value>(event.payload()) else {
            return;
        };
        let state = payload.get("state").and_then(|value| value.as_str()).unwrap_or("");
        let stopped = matches!(state, "paused" | "stopped");
        let was_stopped = {
            let Ok(mut last) = last.lock() else { return };
            let was_stopped = matches!(last.as_str(), "paused" | "stopped");
            *last = state.to_string();
            was_stopped
        };
        if !stopped || was_stopped {
            return;
        }
        let Some(app_state) = app_for_listener.try_state::<crate::AppState>() else {
            return;
        };
        if let Err(error) = notify_event_impl(
            &app_for_listener,
            &app_state,
            NotificationEvent::SyncStopped,
            None,
            true,
        ) {
            tracing::warn!(%error, "sync stopped notification failed");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_on() -> NotificationConfig {
        NotificationConfig {
            notify_allow: true,
            notify_shared: true,
            notify_conflicts: true,
            notify_quota_warnings: true,
            notify_sync_stopped: true,
            notify_needs_decision: true,
            notify_sync_complete: true,
            notify_local_space: true,
            notify_backup_finished: true,
            notify_backup_attention: true,
        }
    }

    #[test]
    fn notifications_allow_off_suppresses_every_event() {
        let config = NotificationConfig {
            notify_allow: false,
            ..all_on()
        };
        for event in NotificationEvent::ALL {
            assert_eq!(
                should_notify(event, &config, true),
                Decision::Suppress(SuppressReason::AllowOff),
                "{event:?}"
            );
        }
    }

    #[test]
    fn notifications_os_off_suppresses_every_event() {
        for event in NotificationEvent::ALL {
            assert_eq!(
                should_notify(event, &all_on(), false),
                Decision::Suppress(SuppressReason::OsOff),
                "{event:?}"
            );
        }
    }

    #[test]
    fn notifications_each_switch_suppresses_only_its_event() {
        for event in NotificationEvent::ALL {
            let mut config = all_on();
            match event {
                NotificationEvent::SharedWithYou => config.notify_shared = false,
                NotificationEvent::FileUpdated => config.notify_conflicts = false,
                NotificationEvent::StorageWarning => config.notify_quota_warnings = false,
                NotificationEvent::SyncStopped => config.notify_sync_stopped = false,
                NotificationEvent::NeedsDecision => config.notify_needs_decision = false,
                NotificationEvent::SyncFinished => config.notify_sync_complete = false,
                NotificationEvent::LocalSpace => config.notify_local_space = false,
                NotificationEvent::BackupFinished => config.notify_backup_finished = false,
                NotificationEvent::BackupAttention => config.notify_backup_attention = false,
            }
            assert_eq!(
                should_notify(event, &config, true),
                Decision::Suppress(SuppressReason::PreferenceOff),
                "{event:?}"
            );
            for other in NotificationEvent::ALL {
                if other != event {
                    assert_eq!(
                        should_notify(other, &config, true),
                        Decision::Send,
                        "{other:?} with {event:?} off"
                    );
                }
            }
        }
    }

    #[test]
    fn notifications_all_on_sends_every_event() {
        for event in NotificationEvent::ALL {
            assert_eq!(should_notify(event, &all_on(), true), Decision::Send, "{event:?}");
        }
    }

    #[test]
    fn notifications_os_off_is_checked_before_allow() {
        let config = NotificationConfig {
            notify_allow: false,
            ..all_on()
        };
        assert_eq!(
            should_notify(NotificationEvent::SyncStopped, &config, false),
            Decision::Suppress(SuppressReason::OsOff)
        );
    }

    #[test]
    fn notifications_file_name_is_dropped_while_locked() {
        // The caller passes the name only when unlocked; the hop's filter is what a locked vault relies on.
        assert_eq!(
            body_for(NotificationEvent::SharedWithYou, None),
            "Someone shared a file with you."
        );
        assert_eq!(
            body_for(NotificationEvent::FileUpdated, None),
            "A file changed on another device."
        );
        assert_eq!(
            body_for(NotificationEvent::NeedsDecision, None),
            "A file changed in two places. Open Beebeeb to choose."
        );
    }

    #[test]
    fn notifications_file_name_with_a_path_is_dropped() {
        assert_eq!(safe_file_name("Contract draft.pdf"), Some("Contract draft.pdf"));
        assert_eq!(safe_file_name("C:\\Users\\anna\\Contract.pdf"), None);
        assert_eq!(safe_file_name("/home/anna/Contract.pdf"), None);
        assert_eq!(safe_file_name("../Contract.pdf"), None);
        assert_eq!(safe_file_name("   "), None);
        assert_eq!(
            body_for(NotificationEvent::SharedWithYou, Some("C:\\Users\\anna\\Contract.pdf")),
            "Someone shared a file with you."
        );
    }

    #[test]
    fn notifications_never_put_a_path_in_a_body() {
        for event in NotificationEvent::ALL {
            let body = body_for(event, Some("C:\\Users\\anna\\Contract.pdf"));
            assert!(!body.contains("C:\\"), "{event:?}: {body}");
            assert!(!body.contains('/'), "{event:?}: {body}");
        }
    }
}
