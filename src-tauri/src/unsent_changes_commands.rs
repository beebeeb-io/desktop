//! Task 2409: the "Not sent" list on the Activity page, and its Try again.
//!
//! `unsent_changes` serves [`state_db::StateDb::unsent_changes`] (the rows the "Not sent" count counts, one each) with
//! what the engine knows next to it: whether sync is paused, the engine's last state (`running` before its first sync
//! check finished), and the progress of a change being sent right now. It carries paths, so unlike
//! `sync_activity_summary` it answers only while the vault is unlocked; the window reads it only while the list is on
//! screen. `retry_unsent_change` is Try again on a change that failed (Guus, 13:39): its attempts and next-attempt time
//! are reset, and the engine is asked for a queue step at once.
//!
//! Task 2414: `not_downloaded` serves [`state_db::StateDb::not_downloaded`] the same way, for the Activity page's "Not
//! downloaded" section: the downloads a pin queued that are paused or out of attempts.

use super::*;

/// One call serves at most this many changes (`UNSENT_MAX_PER_CALL` in `src/windows/unsentChanges.ts`): the window
/// reads the rows it shows (100, and 100 more per "Show more") in one call, so in one read transaction, up to this.
const MAX_PAGE: u32 = 500;

/// A change being sent right now: bytes done of total, from the transfer board.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub(super) struct Sending {
    done: u64,
    total: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(super) struct UnsentChangeView {
    #[serde(flatten)]
    change: state_db::UnsentChange,
    sending: Option<Sending>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(super) struct UnsentChangesView {
    total: i64,
    items: Vec<UnsentChangeView>,
    /// The engine's last state from `engine-status` (`running` until its first sync check finished), or "".
    engine_state: String,
    /// The live pause flag.
    paused: bool,
}

/// The view of one page: an upload on the board (by its file id, not failed) carries its progress. The board knows a
/// file, not a change, so only the change being sent carries it: the first waiting upload (new file or edit) of that
/// file on the page, as a file's changes go in order. A change of it that gave up or is held keeps its own reason.
fn view_of(
    page: state_db::UnsentPage,
    transfers: &[(String, crate::transfer_progress::Transfer)],
    engine_state: String,
    paused: bool,
) -> UnsentChangesView {
    let mut shown = std::collections::HashSet::new();
    let items = page
        .items
        .into_iter()
        .map(|change| {
            let sendable = change.group == "waiting" && matches!(change.kind, "new_file" | "edit");
            let sending = change
                .file_id
                .as_deref()
                .filter(|_| sendable)
                .and_then(|id| {
                    transfers.iter().find_map(|(file_id, t)| {
                        (file_id == id && t.direction == crate::transfer_progress::Direction::Up && !t.failed)
                            .then_some(Sending {
                                done: t.done,
                                total: t.total,
                            })
                    })
                })
                .filter(|_| shown.insert(change.file_id.clone()));
            UnsentChangeView { change, sending }
        })
        .collect();
    UnsentChangesView {
        total: page.total,
        items,
        engine_state,
        paused,
    }
}

/// The unlocked-session gate the other path-carrying reads use (`desktop_recent_files`).
fn require_unlocked(state: &AppState) -> Result<Arc<account::AccountRuntime>, String> {
    let acct = state.active_account()?;
    let unlocked = acct
        .session
        .lock()
        .map_err(|_| "session mutex poisoned".to_string())?
        .is_some();
    if !unlocked {
        return Err("vault is locked".to_string());
    }
    Ok(acct)
}

#[tauri::command]
pub(super) async fn unsent_changes(
    state: State<'_, AppState>,
    runtime: State<'_, popover_data::PopoverRuntime>,
    offset: Option<u32>,
    limit: Option<u32>,
) -> Result<UnsentChangesView, String> {
    session_command!(async {
        let acct = require_unlocked(&state)?;
        let paused = acct.sync_paused.load(Ordering::Relaxed);
        let engine_state = runtime.status.lock().map(|view| view.state.clone()).unwrap_or_default();
        let transfers = runtime.transfers.active();
        let (offset, limit) = (offset.unwrap_or(0), limit.unwrap_or(100).clamp(1, MAX_PAGE));
        let page = tokio::task::spawn_blocking(move || -> Result<state_db::UnsentPage, String> {
            // As the summary: no readable config, no sync root or no state DB yet is nothing waiting.
            let config = DesktopConfig::load().ok();
            let Some(db) = config.as_ref().map(state_db_for_config).transpose()?.flatten() else {
                return Ok(state_db::UnsentPage {
                    total: 0,
                    items: Vec::new(),
                });
            };
            db.unsent_changes(offset, limit, crate::local_state_purge::now_unix_seconds())
                .map_err(|e| format!("read the changes not sent: {e}"))
        })
        .await
        .map_err(|e| format!("changes not sent task failed: {e}"))??;
        Ok(view_of(page, &transfers, engine_state, paused))
    })
}

/// What Try again answers: `queued` (sent at the next queue step), or a refusal code the window words.
pub(super) fn retry_answer(outcome: state_db::RetryOutcome) -> Result<&'static str, &'static str> {
    match outcome {
        state_db::RetryOutcome::Retried | state_db::RetryOutcome::AlreadyQueued => Ok("queued"),
        state_db::RetryOutcome::Unknown => Err("unknown"),
        state_db::RetryOutcome::Held => Err("held"),
        state_db::RetryOutcome::Parked => Err("parked"),
        state_db::RetryOutcome::Superseded => Err("superseded"),
        state_db::RetryOutcome::CopyGone => Err("copy_gone"),
    }
}

/// Task 2414: the downloads a pin queued that will not finish by themselves, one row each, behind "Not downloaded N".
/// Paths, so the same gate as `unsent_changes`; the window reads it only while its section is on screen.
#[tauri::command]
pub(super) async fn not_downloaded(
    state: State<'_, AppState>,
    runtime: State<'_, popover_data::PopoverRuntime>,
    offset: Option<u32>,
    limit: Option<u32>,
) -> Result<UnsentChangesView, String> {
    session_command!(async {
        let acct = require_unlocked(&state)?;
        let paused = acct.sync_paused.load(Ordering::Relaxed);
        let engine_state = runtime.status.lock().map(|view| view.state.clone()).unwrap_or_default();
        let (offset, limit) = (offset.unwrap_or(0), limit.unwrap_or(100).clamp(1, MAX_PAGE));
        let page = tokio::task::spawn_blocking(move || -> Result<state_db::UnsentPage, String> {
            let config = DesktopConfig::load().ok();
            let Some(db) = config.as_ref().map(state_db_for_config).transpose()?.flatten() else {
                return Ok(state_db::UnsentPage {
                    total: 0,
                    items: Vec::new(),
                });
            };
            db.not_downloaded(offset, limit)
                .map_err(|e| format!("read the files not downloaded: {e}"))
        })
        .await
        .map_err(|e| format!("files not downloaded task failed: {e}"))??;
        // No upload progress: a download is never "being sent" (`view_of` gives it none).
        Ok(view_of(page, &[], engine_state, paused))
    })
}

#[tauri::command]
pub(super) async fn retry_unsent_change(state: State<'_, AppState>, id: String) -> Result<String, String> {
    session_command!(async {
        let acct = require_unlocked(&state)?;
        let outcome = tokio::task::spawn_blocking(move || -> Result<state_db::RetryOutcome, String> {
            let config = DesktopConfig::load().ok();
            let Some(db) = config.as_ref().map(state_db_for_config).transpose()?.flatten() else {
                return Ok(state_db::RetryOutcome::Unknown);
            };
            db.retry_unsent_change(&id, crate::local_state_purge::now_unix_seconds())
                .map_err(|e| format!("try the change again: {e}"))
        })
        .await
        .map_err(|e| format!("try again task failed: {e}"))??;
        if outcome == state_db::RetryOutcome::Retried {
            // A running engine sends it now; one that is not running sends it when it starts.
            if let Some(link) = acct
                .engine
                .lock()
                .await
                .as_ref()
                .filter(|engine| engine.is_running())
                .map(|e| e.send_phase())
            {
                link.nudge();
            }
        }
        retry_answer(outcome).map(str::to_string).map_err(str::to_string)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfer_progress::{Direction, Transfer};

    fn change(id: &str, file_id: Option<&str>) -> state_db::UnsentChange {
        state_db::UnsentChange {
            id: id.into(),
            file_id: file_id.map(str::to_string),
            kind: "new_file",
            path: format!("docs/{id}.txt"),
            backup: false,
            group: "waiting".into(),
            queued_at: 1,
            retry_at: None,
            parked: false,
            retryable: false,
            blocked: None,
        }
    }

    /// The upload on the board shows its progress; a download of the same file, a failed upload and another file's
    /// upload do not.
    #[test]
    fn a_change_being_sent_carries_its_progress_and_nothing_else_does() {
        let page = state_db::UnsentPage {
            total: 4,
            items: vec![
                change("a", Some("fa")),
                change("b", Some("fb")),
                change("c", Some("fc")),
                change("d", None),
            ],
        };
        let up = |done| Transfer {
            direction: Direction::Up,
            done,
            total: 100,
            ..Transfer::default()
        };
        let transfers = vec![
            ("fa".to_string(), up(34)),
            (
                "fb".to_string(),
                Transfer {
                    direction: Direction::Down,
                    done: 5,
                    total: 10,
                    ..Transfer::default()
                },
            ),
            ("fc".to_string(), Transfer { failed: true, ..up(50) }),
        ];
        let view = view_of(page, &transfers, "running".into(), true);
        let sending: Vec<_> = view.items.iter().map(|i| i.sending).collect();
        assert_eq!(sending, [Some(Sending { done: 34, total: 100 }), None, None, None]);
        assert_eq!(
            (view.total, view.engine_state.as_str(), view.paused),
            (4, "running", true)
        );
        let json = serde_json::to_value(&view).unwrap();
        assert_eq!(
            json["items"][0]["path"], "docs/a.txt",
            "the change's fields are flattened in"
        );
        assert_eq!(json["items"][0]["sending"]["done"], 34);
    }

    /// The board knows a file, not a change: of a file's changes only the one being sent carries the progress. That is
    /// the first waiting upload of it (a file's changes go in order); a change of it that gave up keeps its reason and
    /// its Try again, and a rename or a later edit of it waits (2409 self-review).
    #[test]
    fn only_the_change_being_sent_carries_its_files_progress() {
        let gave_up = state_db::UnsentChange {
            group: "gave_up".into(),
            kind: "edit",
            retryable: true,
            ..change("old-edit", Some("fa"))
        };
        let rename = state_db::UnsentChange {
            kind: "rename",
            ..change("rename", Some("fa"))
        };
        let page = state_db::UnsentPage {
            total: 4,
            items: vec![
                gave_up,
                change("sending", Some("fa")),
                rename,
                state_db::UnsentChange {
                    kind: "edit",
                    ..change("later-edit", Some("fa"))
                },
            ],
        };
        let transfers = vec![(
            "fa".to_string(),
            Transfer {
                direction: Direction::Up,
                done: 40,
                total: 100,
                ..Transfer::default()
            },
        )];
        let view = view_of(page, &transfers, "idle".into(), false);
        let sending: Vec<_> = view
            .items
            .iter()
            .map(|i| (i.change.id.as_str(), i.sending.is_some()))
            .collect();
        assert_eq!(
            sending,
            [
                ("old-edit", false),
                ("sending", true),
                ("rename", false),
                ("later-edit", false)
            ]
        );
    }

    /// The answer is the JSON the frontend validates: `src-tauri/tests/fixtures/unsent-changes.page.json`.
    #[test]
    fn the_view_is_the_json_the_window_parses() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/unsent-changes.page.json")).unwrap();
        let item = |id: &str, file: &str, kind: &'static str, path: &str, group: &str, at: i64, retryable: bool| {
            state_db::UnsentChange {
                id: id.into(),
                file_id: Some(file.into()),
                kind,
                path: path.into(),
                backup: false,
                group: group.into(),
                queued_at: at,
                retry_at: None,
                parked: false,
                retryable,
                blocked: None,
            }
        };
        let page = state_db::UnsentPage {
            total: 3,
            items: vec![
                item(
                    "op-delete",
                    "file-plan",
                    "delete",
                    "Projects/2026/old plan.docx",
                    "gave_up",
                    1_793_000_000,
                    true,
                ),
                item(
                    "op-edit",
                    "file-budget",
                    "edit",
                    "Finance/budget.xlsx",
                    "quota",
                    1_793_000_100,
                    false,
                ),
                item(
                    "op-new",
                    "file-photo",
                    "new_file",
                    "Camera Roll/IMG_0412.jpg",
                    "waiting",
                    1_793_000_200,
                    false,
                ),
            ],
        };
        let transfers = vec![(
            "file-photo".to_string(),
            Transfer {
                direction: Direction::Up,
                done: 34,
                total: 100,
                ..Transfer::default()
            },
        )];
        assert_eq!(
            serde_json::to_value(view_of(page, &transfers, "running".into(), false)).unwrap(),
            fixture
        );
    }

    /// A refusal is an error with a code the window words; queued (done now, or already) is an answer.
    #[test]
    fn try_again_answers_queued_or_a_refusal_code() {
        use state_db::RetryOutcome::*;
        assert_eq!(retry_answer(Retried), Ok("queued"));
        assert_eq!(retry_answer(AlreadyQueued), Ok("queued"));
        assert_eq!(retry_answer(Unknown), Err("unknown"));
        assert_eq!(retry_answer(Held), Err("held"));
        assert_eq!(retry_answer(Parked), Err("parked"));
        assert_eq!(retry_answer(Superseded), Err("superseded"));
        assert_eq!(retry_answer(CopyGone), Err("copy_gone"));
    }

    /// Both commands answer only while the vault is unlocked (they carry paths, or change the queue), and Try again
    /// nudges the engine only when it changed a row.
    #[test]
    fn both_commands_need_an_unlocked_vault_and_try_again_nudges_after_a_change() {
        let source = include_str!("unsent_changes_commands.rs").replace("\r\n", "\n");
        for command in [
            "async fn unsent_changes(",
            "async fn retry_unsent_change(",
            "async fn not_downloaded(",
        ] {
            let body = &source[source.find(command).unwrap()..];
            let body = &body[..body.find("\n}\n").unwrap()];
            let gate = body
                .find("require_unlocked(&state)?")
                .unwrap_or_else(|| panic!("{command} has the gate"));
            assert!(
                gate < body.find("spawn_blocking").unwrap(),
                "{command}: the gate comes before the read"
            );
        }
        let retry = &source[source.find("async fn retry_unsent_change(").unwrap()..];
        let nudge = retry.find("link.nudge()").expect("the nudge");
        let only_after_a_change = retry
            .find("if outcome == state_db::RetryOutcome::Retried {")
            .expect("the condition");
        assert!(only_after_a_change < nudge);
    }
}
