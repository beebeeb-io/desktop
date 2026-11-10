// The one gate every notification goes through: the persisted switches from Settings › Notifications (slice 13) plus
// the OS's own switch. It answers send or suppress with a reason code, so a caller never has to re-derive the rules and
// a test can name each suppression. The toast itself is the Rust hop (`src-tauri/src/notifications.rs`); this file
// proves the decision, not delivery.
import { describe, expect, test } from 'bun:test'
import {
  shouldNotify,
  type NotificationConfig,
  type NotificationEvent,
} from '../src/v2/sections/settings/shell/notificationsGate'

const EVENTS: NotificationEvent[] = [
  'shared_with_you',
  'file_updated',
  'storage_warning',
  'sync_stopped',
  'needs_decision',
  'sync_finished',
  'local_space',
  'backup_finished',
  'backup_attention',
]

const ALL_ON: NotificationConfig = {
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

describe('notifications gate', () => {
  test('Allow off suppresses every event with allow_off', () => {
    for (const event of EVENTS) {
      expect(shouldNotify(event, { ...ALL_ON, notify_allow: false }, true)).toEqual({ send: false, reason: 'allow_off' })
    }
  })

  test('the OS switch off suppresses every event with os_off', () => {
    for (const event of EVENTS) {
      expect(shouldNotify(event, ALL_ON, false)).toEqual({ send: false, reason: 'os_off' })
    }
  })

  test('each event has its own switch, and off suppresses only that event', () => {
    for (const event of EVENTS) {
      const config = { ...ALL_ON, [SWITCH[event]]: false }
      expect(shouldNotify(event, config, true)).toEqual({ send: false, reason: 'preference_off' })
      for (const other of EVENTS) {
        if (other !== event) expect(shouldNotify(other, config, true)).toEqual({ send: true })
      }
    }
  })

  test('all on sends every event', () => {
    for (const event of EVENTS) expect(shouldNotify(event, ALL_ON, true)).toEqual({ send: true })
  })

  test('the OS switch is checked before Allow', () => {
    expect(shouldNotify('sync_stopped', { ...ALL_ON, notify_allow: false }, false)).toEqual({ send: false, reason: 'os_off' })
  })
})
