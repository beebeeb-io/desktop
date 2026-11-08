// Notifications (DW-0750, DW-0750b): which alerts Beebeeb sends on this PC.
import { useState } from 'react'
import { Btn, Group, Page, Row, Switch } from './kit'
import { useConfig } from './data'

export function Notifications({ offInWindows }: { offInWindows?: boolean } = {}) {
  const { config, update } = useConfig()
  const off = !!offInWindows
  const [local, setLocal] = useState<Record<string, boolean>>({
    shared: true, stopped: true, decision: true, finished: false, space: true, bdone: true, battn: true, allow: true,
  })
  const flip = (k: string) => (v: boolean) => setLocal((s) => ({ ...s, [k]: v }))
  const sw = (_k: string, label: string, on: boolean, change: (v: boolean) => void) =>
    <Switch label={label} on={on} onChange={change} disabled={off} />
  return (
    <Page title="Notifications">
      <Group note={off ? undefined : 'No marketing, ever.'}>
        {off ? (
          <Row label="Notifications are off" sub="Turned off for Beebeeb in Windows Settings"
            right={<Btn onClick={() => window.open('ms-settings:notifications', '_self')}>Open Windows Settings</Btn>} />
        ) : (
          <Row label="Allow notifications" sub="Also controlled in Windows Settings." right={<Switch label="Allow notifications" on={local.allow} onChange={flip('allow')} />} />
        )}
        <Row label="Recent notifications" value="2 new" href="?section=settings&sub=notifications&recent=1" />
      </Group>
      <Group title="Security">
        <Row label="New sign-ins and security changes" sub={off ? "Off in Windows: nothing arrives until you turn notifications on there." : undefined}
          value={off ? 'Off in Windows' : 'Always on'} />
      </Group>
      <Group title="Your account" note="Alerts never show file names while Beebeeb is locked.">
        <Row label="Shared with you" dim={off} right={sw('shared', 'Shared with you', local.shared, flip('shared'))} />
        <Row label="File updates" dim={off} right={sw('upd', 'File updates', config.notify_conflicts, (v) => update({ notify_conflicts: v }))} />
        <Row label="Storage warnings" dim={off} right={sw('quota', 'Storage warnings', config.notify_quota_warnings, (v) => update({ notify_quota_warnings: v }))} />
      </Group>
      <Group title="Sync on this PC">
        <Row label="Sync stopped" dim={off} right={sw('stopped', 'Sync stopped', local.stopped, flip('stopped'))} />
        <Row label="Files need a decision" dim={off} right={sw('decision', 'Files need a decision', local.decision, flip('decision'))} />
        <Row label="Sync finished" sub="Off by default." dim={off} right={sw('fin', 'Sync finished', config.notify_sync_complete, (v) => update({ notify_sync_complete: v }))} />
        <Row label="Space on this PC" sub="Downloaded files near their limit." dim={off} right={sw('space', 'Space on this PC', local.space, flip('space'))} />
      </Group>
      <Group title="Backups on this PC">
        <Row label="Backup finished" dim={off} right={sw('bdone', 'Backup finished', local.bdone, flip('bdone'))} />
        <Row label="Backup needs attention" dim={off} right={sw('battn', 'Backup needs attention', local.battn, flip('battn'))} />
      </Group>
    </Page>
  )
}
