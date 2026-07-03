import type {
  DesktopConfig,
  NotificationPreferenceValues,
} from '../desktopApi'

export type AccountNotificationPrefKey = keyof NotificationPreferenceValues
export type DesktopNotificationPrefKey = Extract<keyof DesktopConfig, 'notify_conflicts'>

export interface NotificationPreferenceMeta<Key extends string> {
  key: Key
  label: string
  hint: string
}

export const ACCOUNT_NOTIFICATION_PREF_META: Array<NotificationPreferenceMeta<AccountNotificationPrefKey>> = [
  { key: 'new_device_login', label: 'New device sign-in', hint: 'We cannot see your files, but we know when a new device signs in.' },
  { key: 'share_received', label: 'Share received', hint: 'When someone grants you access to a file or folder.' },
  { key: 'file_updated', label: 'File updated', hint: 'When a file in your vault changes on another device.' },
  { key: 'storage_warning', label: 'Storage warning', hint: 'When you are close to your storage limit.' },
  { key: 'backup_complete', label: 'Backup complete', hint: 'When a scheduled backup finishes.' },
]

export const DESKTOP_NOTIFICATION_PREF_META: Array<NotificationPreferenceMeta<DesktopNotificationPrefKey>> = [
  {
    key: 'notify_conflicts',
    label: 'Conflict alerts',
    hint: 'When local and remote edits diverge and need your choice.',
  },
]
