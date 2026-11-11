//! The seam between the engine and the shell that shows it: what the engine tells the UI ([`EngineEvent`]) and
//! what it asks of the program around it ([`EngineHost`]).
//!
//! The Tauri shell implements [`EngineHost`] by emitting each event under its old Tauri name with the same JSON
//! ([`EngineEvent::name`], [`EngineEvent::payload`]); the native shell takes the typed values from a
//! [`ChannelHost`]. Must not: name a UI toolkit, a window, or block in [`EngineHost::event`] (the engine calls it
//! from its own runtime's tasks).

use std::sync::Arc;
use std::sync::mpsc;

use serde::Serialize;

use crate::drive_v2::dto::DriveEvent;
use crate::engine_status::StatusPayload;
use crate::transfer_progress::TransferBoard;
use crate::unknown_check::StartupGuardProgress;

/// One `engine-status` event: the engine's state and the start-up guard's progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineStatus {
    pub status: StatusPayload,
    pub startup_guard: StartupGuardProgress,
}

impl Serialize for EngineStatus {
    /// The JSON the web view has always received: the status keys, then `startup_guard`.
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut json = self.status.to_json();
        if let Some(object) = json.as_object_mut() {
            object.insert("startup_guard".to_string(), serde_json::json!(self.startup_guard));
        }
        json.serialize(serializer)
    }
}

/// `engine-conflict`: the engine found a conflict on this file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConflictNotice {
    pub file_id: String,
    pub file_name: String,
    pub is_text: bool,
}

/// `conflict-auto-resolved`: a conflict older than its deadline was settled by keeping both copies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConflictAutoResolved {
    pub file_id: String,
    pub file_name: String,
    pub conflict_copy_name: String,
}

/// `conflict-resolved`: the person settled a conflict (by a choice on a file, or by a queued operation).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ConflictResolved {
    Choice { file_id: String, choice: String },
    Operation { op_id: String },
}

/// `engine-status` as the pause switch sends it: the state alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PauseChanged {
    pub state: PauseState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PauseState {
    Paused,
    Idle,
}

/// `sign-out-send-progress`: how many changes the sign-out still waits for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignOutSendProgress {
    pub changes_left: u64,
}

/// `update-available`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpdateAvailable {
    pub version: String,
    pub channel: String,
    pub body: String,
    pub release_notes_url: String,
}

/// `version-center-review`: a version restore was queued.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VersionReview {
    pub file_id: String,
    pub version_id: String,
    pub kind: VersionReviewKind,
    pub operation_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum VersionReviewKind {
    Restore,
}

/// `menu:files-added`: how many files "Add files" copied into the folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct FilesAdded {
    pub count: u64,
}

/// `menu:folder-created`: where "New folder" made it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FolderCreated {
    pub path: String,
}

/// Everything the engine side tells the UI. One variant per event name the Tauri shell emits for the engine today;
/// [`EngineEvent::NOT_TYPED_YET`] names the three whose payload types still live in the shell.
#[derive(Debug, Clone, PartialEq)]
pub enum EngineEvent {
    Status(EngineStatus),
    PauseChanged(PauseChanged),
    Conflict(ConflictNotice),
    ConflictAutoResolved(ConflictAutoResolved),
    ConflictResolved(ConflictResolved),
    Drive(DriveEvent),
    KeptFolderChanged,
    /// To the one window that owns the sign-out flow (`window`), not to all.
    SignOutFlowClosed {
        window: String,
    },
    /// To the window that owns the sign-out (`window`).
    SignOutSendProgress {
        window: String,
        progress: SignOutSendProgress,
    },
    UpdateAvailable(UpdateAvailable),
    VersionReview(VersionReview),
    FilesAdded(FilesAdded),
    FolderCreated(FolderCreated),
}

impl EngineEvent {
    /// Engine-side event names that have no variant yet: their payload types are still in the Tauri shell
    /// (`browser_login` step bodies, `drive_v2::status` transfer rows, `sign_out::send_phase` ends).
    pub const NOT_TYPED_YET: [&'static str; 3] = ["browser-login", "drive://transfers", "sign-out-send-ended"];

    /// The Tauri event name this event has always had.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Status(_) | Self::PauseChanged(_) => crate::engine_status::EVENT,
            Self::Conflict(_) => "engine-conflict",
            Self::ConflictAutoResolved(_) => "conflict-auto-resolved",
            Self::ConflictResolved(_) => "conflict-resolved",
            Self::Drive(_) => "drive://event",
            Self::KeptFolderChanged => "kept-folder-changed",
            Self::SignOutFlowClosed { .. } => "sign-out-flow-closed",
            Self::SignOutSendProgress { .. } => "sign-out-send-progress",
            Self::UpdateAvailable(_) => "update-available",
            Self::VersionReview(_) => "version-center-review",
            Self::FilesAdded(_) => "menu:files-added",
            Self::FolderCreated(_) => "menu:folder-created",
        }
    }

    /// The one window the event goes to; `None` is every window.
    pub fn window(&self) -> Option<&str> {
        match self {
            Self::SignOutFlowClosed { window } | Self::SignOutSendProgress { window, .. } => Some(window),
            _ => None,
        }
    }

    /// The event's JSON body, as the web view receives it.
    pub fn payload(&self) -> serde_json::Value {
        let json = match self {
            Self::Status(body) => serde_json::to_value(body),
            Self::PauseChanged(body) => serde_json::to_value(body),
            Self::Conflict(body) => serde_json::to_value(body),
            Self::ConflictAutoResolved(body) => serde_json::to_value(body),
            Self::ConflictResolved(body) => serde_json::to_value(body),
            Self::Drive(body) => serde_json::to_value(body),
            Self::KeptFolderChanged | Self::SignOutFlowClosed { .. } => Ok(serde_json::Value::Null),
            Self::SignOutSendProgress { progress, .. } => serde_json::to_value(progress),
            Self::UpdateAvailable(body) => serde_json::to_value(body),
            Self::VersionReview(body) => serde_json::to_value(body),
            Self::FilesAdded(body) => serde_json::to_value(body),
            Self::FolderCreated(body) => serde_json::to_value(body),
        };
        // Every body is plain data (strings, numbers, options): serializing it cannot fail.
        json.unwrap_or(serde_json::Value::Null)
    }
}

/// What the engine shows the person outside a window (a toast, a banner).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// A sync pass that moved files has finished.
    SyncComplete,
    /// The local cache is close to its limit.
    QuotaWarning { usage_bytes: i64, limit_bytes: i64 },
    /// A conflict was found on this file.
    Conflict { file_name: String },
    /// Windows: a delete or rename of a backed-up folder was refused; these are the words (they carry a path:
    /// show them, never log them).
    BackupRefusal { message: String },
}

/// The program around the engine. The engine holds one `Arc<dyn EngineHost>` for its lifetime.
///
/// Threading: every method is called from the engine's own runtime (a tokio worker) or from an OS callback
/// thread. None may block on the UI thread; [`ChannelHost`] shows the intended shape (queue, then wake).
pub trait EngineHost: Send + Sync + 'static {
    /// The engine has something to tell the UI.
    fn event(&self, event: EngineEvent);

    /// Show a notice outside the window. An error is logged by the engine and changes nothing else.
    fn notify(&self, notice: Notice) -> Result<(), String>;

    /// Whether a new conflict opens its own window on this platform (the shell's policy).
    fn conflict_opens_window(&self) -> bool {
        false
    }

    /// Ask the person about a conflict in a window of its own (only when [`Self::conflict_opens_window`]).
    fn open_conflict(&self, _conflict: &ConflictNotice) -> Result<(), String> {
        Ok(())
    }

    /// The transfer board the shell's status views read, shared with the engine. `None`: the engine keeps its own.
    fn transfer_board(&self) -> Option<Arc<TransferBoard>> {
        None
    }

    /// The wall clock. The engine's own tick stamps still read the system clock directly; this is for the code
    /// that moves behind the seam next (the session's expiry checks).
    fn now(&self) -> std::time::SystemTime {
        std::time::SystemTime::now()
    }
}

/// What a [`ChannelHost`] queues for the UI thread.
#[derive(Debug, Clone, PartialEq)]
pub enum HostMessage {
    Event(EngineEvent),
    Notice(Notice),
    /// The engine asks for the conflict window.
    OpenConflict(ConflictNotice),
}

/// A host for a UI that has its own thread: every call queues a [`HostMessage`] and calls the wake hook, and
/// returns at once. The UI thread drains the receiver when woken (or on its own frame tick).
pub struct ChannelHost {
    queue: mpsc::Sender<HostMessage>,
    wake: Box<dyn Fn() + Send + Sync>,
    conflict_opens_window: bool,
    transfers: Arc<TransferBoard>,
}

impl ChannelHost {
    /// `wake` runs on the engine's thread after each message is queued: post a wake-up to the UI loop there,
    /// do nothing else.
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> (Arc<Self>, mpsc::Receiver<HostMessage>) {
        let (queue, messages) = mpsc::channel();
        let host = Self {
            queue,
            wake: Box::new(wake),
            conflict_opens_window: false,
            transfers: TransferBoard::new(),
        };
        (Arc::new(host), messages)
    }

    /// The board the engine reports byte progress on: the shell reads its rows for a transfers view.
    pub fn transfers(&self) -> Arc<TransferBoard> {
        self.transfers.clone()
    }

    fn send(&self, message: HostMessage) {
        // A closed receiver means the UI is gone; the engine is being stopped by the same shutdown.
        if self.queue.send(message).is_ok() {
            (self.wake)();
        }
    }
}

impl EngineHost for ChannelHost {
    fn event(&self, event: EngineEvent) {
        self.send(HostMessage::Event(event));
    }

    fn notify(&self, notice: Notice) -> Result<(), String> {
        self.send(HostMessage::Notice(notice));
        Ok(())
    }

    fn conflict_opens_window(&self) -> bool {
        self.conflict_opens_window
    }

    fn open_conflict(&self, conflict: &ConflictNotice) -> Result<(), String> {
        self.send(HostMessage::OpenConflict(conflict.clone()));
        Ok(())
    }

    fn transfer_board(&self) -> Option<Arc<TransferBoard>> {
        Some(self.transfers.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Each typed body serializes to the JSON its Tauri emit site built by hand before the seam (the literal on the
    /// right is that site's `json!`), compared as text: same keys, same order, same values.
    #[test]
    fn every_typed_event_has_the_name_and_the_bytes_its_tauri_emit_site_sent() {
        let text = |value: &serde_json::Value| serde_json::to_string(value).unwrap();
        let cases: Vec<(EngineEvent, &str, serde_json::Value)> = vec![
            (
                EngineEvent::PauseChanged(PauseChanged {
                    state: PauseState::Paused,
                }),
                "engine-status",
                json!({ "state": "paused" }),
            ),
            (
                EngineEvent::PauseChanged(PauseChanged {
                    state: PauseState::Idle,
                }),
                "engine-status",
                json!({ "state": "idle" }),
            ),
            (
                EngineEvent::Conflict(ConflictNotice {
                    file_id: "f1".into(),
                    file_name: "a.txt".into(),
                    is_text: true,
                }),
                "engine-conflict",
                json!({ "file_id": "f1", "file_name": "a.txt", "is_text": true }),
            ),
            (
                EngineEvent::ConflictAutoResolved(ConflictAutoResolved {
                    file_id: "f1".into(),
                    file_name: "docs/a.txt".into(),
                    conflict_copy_name: "a (conflict).txt".into(),
                }),
                "conflict-auto-resolved",
                json!({ "file_id": "f1", "file_name": "docs/a.txt", "conflict_copy_name": "a (conflict).txt" }),
            ),
            (
                EngineEvent::ConflictResolved(ConflictResolved::Choice {
                    file_id: "f1".into(),
                    choice: "keep_local".into(),
                }),
                "conflict-resolved",
                json!({ "file_id": "f1", "choice": "keep_local" }),
            ),
            (
                EngineEvent::ConflictResolved(ConflictResolved::Operation { op_id: "op-7".into() }),
                "conflict-resolved",
                json!({ "op_id": "op-7" }),
            ),
            (
                EngineEvent::Drive(DriveEvent::Changed { folder_id: None }),
                "drive://event",
                json!({ "kind": "changed", "folderId": null }),
            ),
            (
                EngineEvent::Drive(DriveEvent::Removed { id: "s1".into() }),
                "drive://event",
                json!({ "kind": "removed", "id": "s1" }),
            ),
            (EngineEvent::KeptFolderChanged, "kept-folder-changed", json!(())),
            (
                EngineEvent::SignOutFlowClosed { window: "main".into() },
                "sign-out-flow-closed",
                json!(()),
            ),
            (
                EngineEvent::SignOutSendProgress {
                    window: "main".into(),
                    progress: SignOutSendProgress { changes_left: 3 },
                },
                "sign-out-send-progress",
                json!({ "changesLeft": 3 }),
            ),
            (
                EngineEvent::UpdateAvailable(UpdateAvailable {
                    version: "0.9.0".into(),
                    channel: "stable".into(),
                    body: String::new(),
                    release_notes_url: "https://example.invalid/v0.9.0".into(),
                }),
                "update-available",
                json!({ "version": "0.9.0", "channel": "stable", "body": "", "release_notes_url": "https://example.invalid/v0.9.0" }),
            ),
            (
                EngineEvent::VersionReview(VersionReview {
                    file_id: "f1".into(),
                    version_id: "v2".into(),
                    kind: VersionReviewKind::Restore,
                    operation_id: "op-1".into(),
                }),
                "version-center-review",
                json!({ "file_id": "f1", "version_id": "v2", "kind": "restore", "operation_id": "op-1" }),
            ),
            (
                EngineEvent::FilesAdded(FilesAdded { count: 2 }),
                "menu:files-added",
                json!({ "count": 2 }),
            ),
            (
                EngineEvent::FolderCreated(FolderCreated {
                    path: "/tmp/x/New folder".into(),
                }),
                "menu:folder-created",
                json!({ "path": "/tmp/x/New folder" }),
            ),
        ];
        assert_eq!(cases.len(), 15);
        for (event, name, before) in &cases {
            assert_eq!(event.name(), *name, "{event:?}");
            assert_eq!(text(&event.payload()), text(before), "{name}");
        }
    }

    /// `engine-status`: the typed event is the old `with_startup_guard(payload.to_json())`, byte for byte, for a
    /// plain state, an error and a state with activity and a reason.
    #[test]
    fn the_typed_status_event_is_the_old_status_json_with_the_startup_guard() {
        use crate::engine_status::{Activity, Reason, State, plain_status};
        let guard = StartupGuardProgress {
            running: true,
            done: 2,
            total: 5,
        };
        let before = |status: &StatusPayload| {
            let mut payload = status.to_json();
            payload
                .as_object_mut()
                .unwrap()
                .insert("startup_guard".to_string(), json!(guard));
            serde_json::to_string(&payload).unwrap()
        };
        let mut busy = plain_status("syncing", Some("/tmp/root".into()), None);
        busy.activity = Some(Activity {
            files_remaining: 3,
            bytes_total: 900,
            bytes_done: 100,
        });
        busy.last_tick_ok_at = Some(1_700_000_000);
        busy.reason = Some(Reason {
            code: "offline",
            detail: Some("no route".into()),
        });
        let cases = [
            plain_status("running", None, None),
            plain_status("error", Some("/tmp/root".into()), Some("lock held")),
            busy,
        ];
        for status in cases {
            let event = EngineEvent::Status(EngineStatus {
                status: status.clone(),
                startup_guard: guard,
            });
            assert_eq!(event.name(), "engine-status");
            assert_eq!(serde_json::to_string(&event.payload()).unwrap(), before(&status));
            assert_eq!(status.state == State::Error, status.error.is_some());
        }
        let keys: Vec<String> = EngineEvent::Status(EngineStatus {
            status: plain_status("idle", None, None),
            startup_guard: guard,
        })
        .payload()
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
        assert_eq!(keys.len(), 10, "nine status keys and startup_guard: {keys:?}");
    }

    #[test]
    fn a_channel_host_queues_in_order_and_wakes_once_per_message() {
        let wakes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = wakes.clone();
        let (host, messages) = ChannelHost::new(move || {
            counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        host.event(EngineEvent::KeptFolderChanged);
        host.notify(Notice::SyncComplete).unwrap();
        host.event(EngineEvent::FilesAdded(FilesAdded { count: 1 }));
        let got: Vec<HostMessage> = messages.try_iter().collect();
        assert_eq!(
            got,
            vec![
                HostMessage::Event(EngineEvent::KeptFolderChanged),
                HostMessage::Notice(Notice::SyncComplete),
                HostMessage::Event(EngineEvent::FilesAdded(FilesAdded { count: 1 })),
            ]
        );
        assert_eq!(wakes.load(std::sync::atomic::Ordering::SeqCst), 3);
        drop(messages);
        host.event(EngineEvent::KeptFolderChanged);
        assert_eq!(
            wakes.load(std::sync::atomic::Ordering::SeqCst),
            3,
            "no wake once the UI is gone"
        );
    }
}
