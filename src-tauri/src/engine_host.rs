//! The Tauri shell as the engine's host (`beebeeb_engine::host::EngineHost`): each engine event goes out under
//! its Tauri event name with the JSON it always had; notices and the conflict window go through the commands'
//! own helpers. Must not: add a behaviour the engine did not have when it held the `AppHandle` itself.

use std::sync::Arc;

use beebeeb_engine::host::{ConflictNotice, EngineEvent, EngineHost, Notice};
use tauri::{Emitter, Manager};

pub(crate) struct TauriHost {
    app: tauri::AppHandle,
}

/// The host the engine runner is started with.
pub(crate) fn host(app: &tauri::AppHandle) -> Arc<dyn EngineHost> {
    Arc::new(TauriHost { app: app.clone() })
}

impl EngineHost for TauriHost {
    fn event(&self, event: EngineEvent) {
        let name = event.name();
        let sent = match event.window() {
            Some(window) => self.app.emit_to(window, name, event.payload()),
            None => self.app.emit(name, event.payload()),
        };
        if let Err(error) = sent {
            tracing::warn!(%error, event = name, "failed to emit an engine event");
        }
    }

    fn notify(&self, notice: Notice) -> Result<(), String> {
        match notice {
            Notice::SyncComplete => crate::notify_sync_complete_impl(&self.app),
            Notice::QuotaWarning {
                usage_bytes,
                limit_bytes,
            } => crate::notify_quota_warning_impl(&self.app, usage_bytes, limit_bytes),
            Notice::Conflict { file_name } => crate::notify_conflict_impl(&self.app, &file_name),
            Notice::BackupRefusal { message } => {
                use tauri_plugin_notification::NotificationExt;
                self.app
                    .notification()
                    .builder()
                    .title("Beebeeb")
                    .body(message)
                    .show()
                    .map_err(|e| e.to_string())
            }
        }
    }

    fn conflict_opens_window(&self) -> bool {
        crate::surfaces::policy::conflict_auto_opens_window(crate::surfaces::policy::Platform::current())
    }

    fn open_conflict(&self, conflict: &ConflictNotice) -> Result<(), String> {
        crate::open_conflict_window_impl(
            &self.app,
            &conflict.file_id,
            &conflict.file_name,
            conflict.is_text,
            None,
        )
    }

    // Task 1683 slice 2: the board the popover snapshot reads (managed app state); a runner without app state
    // (tests) keeps a private one.
    fn transfer_board(&self) -> Option<Arc<beebeeb_engine::transfer_progress::TransferBoard>> {
        self.app
            .try_state::<crate::popover_data::PopoverRuntime>()
            .map(|runtime| runtime.transfers.clone())
    }
}
