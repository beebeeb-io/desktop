// The one gate every notification goes through. It is pure: the persisted switches from Settings › Notifications
// (slice 13) and the OS's own switch go in, send or suppress with a reason code comes out. The Rust hop
// (`src-tauri/src/notifications.rs`) mirrors this table and re-checks the security rules (signed in, no file name while
// the vault is locked); this side is what the window uses to decide whether to ask for a toast at all.
//
// The event names are the pane's rows, in the pane's order. `file_updated` is the pane's "File updates" row, which is
// stored as `notify_conflicts` (the key the desktop has used since Task 9); `sync_finished` is stored as
// `notify_sync_complete`. The mapping is the one the pane already writes, not a new one.

export type NotificationEvent =
  | 'shared_with_you'
  | 'file_updated'
  | 'storage_warning'
  | 'sync_stopped'
  | 'needs_decision'
  | 'sync_finished'
  | 'local_space'
  | 'backup_finished'
  | 'backup_attention'

export interface NotificationConfig {
  notify_allow: boolean
  notify_shared: boolean
  notify_conflicts: boolean
  notify_quota_warnings: boolean
  notify_sync_stopped: boolean
  notify_needs_decision: boolean
  notify_sync_complete: boolean
  notify_local_space: boolean
  notify_backup_finished: boolean
  notify_backup_attention: boolean
}

/** Why a notification was not sent. Stable codes, so a caller can log or test them without matching copy. */
export type SuppressReason = 'os_off' | 'allow_off' | 'preference_off'

export type NotificationDecision = { send: true } | { send: false; reason: SuppressReason }

/** The switch each event reads. One row per event, so a new event cannot silently reuse another's switch. */
const SWITCH: Record<NotificationEvent, keyof NotificationConfig> = {
  shared_with_you: 'notify_shared',
  file_updated: 'notify_conflicts',
  storage_warning: 'notify_quota_warnings',
  sync_stopped: 'notify_sync_stopped',
  needs_decision: 'notify_needs_decision',
  sync_finished: 'notify_sync_complete',
  local_space: 'notify_local_space',
  backup_finished: 'notify_backup_finished',
  backup_attention: 'notify_backup_attention',
}

/**
 * Send or suppress one event. The OS switch wins first (Beebeeb cannot turn Windows' own switch back on), then Allow,
 * then the event's own switch. A missing key is the caller's problem: the config type is complete, and Rust fills the
 * defaults for an older file.
 */
export function shouldNotify(
  event: NotificationEvent,
  config: NotificationConfig,
  osAllowed: boolean,
): NotificationDecision {
  if (!osAllowed) return { send: false, reason: 'os_off' }
  if (!config.notify_allow) return { send: false, reason: 'allow_off' }
  if (!config[SWITCH[event]]) return { send: false, reason: 'preference_off' }
  return { send: true }
}
