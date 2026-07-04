import { describe, expect, test } from 'bun:test'
import {
  ACCOUNT_NOTIFICATION_PREF_META,
  DESKTOP_NOTIFICATION_PREF_META,
} from '../src/windows/notificationPreferenceMeta'

describe('notificationPreferenceMeta', () => {
  test('keeps account notification preferences aligned with server preference keys', () => {
    expect(ACCOUNT_NOTIFICATION_PREF_META.map((item) => item.key)).toEqual([
      'new_device_login',
      'share_received',
      'file_updated',
      'storage_warning',
      'backup_complete',
    ])
  })

  test('exposes only desktop OS notifications that are actually fired locally', () => {
    expect(DESKTOP_NOTIFICATION_PREF_META).toEqual([
      {
        key: 'notify_conflicts',
        label: 'Conflict alerts',
        hint: 'When local and remote edits diverge and need your choice.',
      },
      {
        key: 'notify_sync_complete',
        label: 'Sync complete',
        hint: 'When this device finishes syncing and is all caught up.',
      },
      {
        key: 'notify_quota_warnings',
        label: 'Local cache warnings',
        hint: 'When this device is close to its local cache limit.',
      },
    ])
  })
})
