// Notifications (DW-0750, DW-0750b): which alerts Beebeeb sends on this PC.
import { useState } from 'react'
import { Btn, Group, Page, Row, Switch } from './kit'
import { useConfig, useRead } from './data'
import type { DesktopConfig, NotificationList } from '../../../../desktopApi'
import { openExternal } from '../../../external'
import { actionFailure } from '../../files/model'

export function Notifications({ offInWindows }: { offInWindows?: boolean } = {}) {
  const { config, update } = useConfig()
  const off = !!offInWindows
  // Not on the TypeScript DesktopConfig: that file is over its line baseline. A patch still sends the key, and Rust
  // merges only that key. A missing key on an older file reads as the default below.
  const extra = {
    notify_allow: true,
    notify_shared: true,
    notify_sync_stopped: true,
    notify_needs_decision: true,
    notify_local_space: true,
    notify_backup_finished: true,
    notify_backup_attention: true,
  } as const
  const pref = (key: keyof typeof extra): boolean => {
    const value = (config as DesktopConfig & Partial<Record<keyof typeof extra, boolean>>)[key]
    return typeof value === 'boolean' ? value : extra[key]
  }
  const [refused, setRefused] = useState<string | null>(null)
  // Windows' own Notifications › Beebeeb (DW-0750b#0): Beebeeb can't turn them back on itself.
  const openWindowsSettings = () => { void openExternal('ms-settings:notifications').then((r) => setRefused(r.ok ? null : actionFailure('open Windows Settings', r.unsupported ? 'unsupported' : 'failed').text)) }
  const notices = useRead<NotificationList>('account_notifications')
  const unread = notices.state === 'ready' ? notices.value.unread_count : 0
  const save = (key: string, value: boolean) => { void update({ [key]: value } as Partial<DesktopConfig>) }
  const sw = (key: string, label: string, on: boolean) => <Switch label={label} on={on} onChange={(v) => save(key, v)} />
  const allow = pref('notify_allow')
  // Allow off, or off in Windows: the groups below are not drawn. Recent stays, so what already arrived can be read.
  const hidden = off || !allow
  return (
    <Page title="Notifications">
      <Group note={off ? undefined : 'No marketing, ever.'}>
        {off ? (
          <Row label="Notifications are off" sub="Turned off for Beebeeb in Windows Settings"
            right={<Btn onClick={openWindowsSettings}>Open Windows Settings</Btn>} />
        ) : (
          <Row label="Allow notifications" sub="Also controlled in Windows Settings." right={sw('notify_allow', 'Allow notifications', allow)} />
        )}
        <Row label="Recent notifications" value={unread > 0 ? `${unread.toLocaleString('en')} new` : undefined} href="?section=settings&page=recent" />
      </Group>
      {refused ? <p role="alert" className="-mt-3 px-1 text-[12px] text-red-text">{refused}</p> : null}
      {hidden ? null : <Group title="Security">
        <Row label="New sign-ins and security changes" value="Always on" />
      </Group>}
      {hidden ? null : <Group title="Your account" note="Alerts never show file names while Beebeeb is locked.">
        <Row label="Shared with you" right={sw('notify_shared', 'Shared with you', pref('notify_shared'))} />
        <Row label="File updates" right={sw('notify_conflicts', 'File updates', config.notify_conflicts)} />
        <Row label="Storage warnings" right={sw('notify_quota_warnings', 'Storage warnings', config.notify_quota_warnings)} />
      </Group>}
      {hidden ? null : <Group title="Sync on this PC">
        <Row label="Sync stopped" right={sw('notify_sync_stopped', 'Sync stopped', pref('notify_sync_stopped'))} />
        <Row label="Files need a decision" right={sw('notify_needs_decision', 'Files need a decision', pref('notify_needs_decision'))} />
        <Row label="Sync finished" sub="Off by default." right={sw('notify_sync_complete', 'Sync finished', config.notify_sync_complete)} />
        <Row label="Space on this PC" sub="Downloaded files near their limit." right={sw('notify_local_space', 'Space on this PC', pref('notify_local_space'))} />
      </Group>}
      {hidden ? null : <Group title="Backups on this PC">
        <Row label="Backup finished" right={sw('notify_backup_finished', 'Backup finished', pref('notify_backup_finished'))} />
        <Row label="Backup needs attention" right={sw('notify_backup_attention', 'Backup needs attention', pref('notify_backup_attention'))} />
      </Group>}
    </Page>
  )
}
