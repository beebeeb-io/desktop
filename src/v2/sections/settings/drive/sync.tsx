// Drive › Sync and File Explorer (DW-0746) and its two confirms (DW-0746b, repair).
import { Btn, Dialog, Group, Page, Row, Switch, fmtBytes } from '../shell/kit'
import { FIXTURE_LOCAL, useConfig } from '../shell/data'
import { useSettings } from '../shell/context'
import { actionFailure } from '../../files/model'
import { useState } from 'react'

export function SyncPane() {
  const { open } = useSettings()
  const { config, update } = useConfig()
  const [login, setLogin] = useState(true)
  const [nav, setNav] = useState(true)
  return (
    <Page title="Sync and File Explorer">
      <Group title="File Explorer">
        <Row label="Beebeeb in File Explorer" sub="In File Explorer's navigation pane" right={<Switch label="Beebeeb in File Explorer" on={nav} onChange={setNav} />} />
        <Row label="Show in File Explorer" onClick={() => undefined} />
        <Row label="Show sync status on files" right={<Switch label="Show sync status on files" on={!!config.sync_overlays} onChange={(sync_overlays) => update({ sync_overlays })} />} />
        <Row label="Repair Beebeeb in File Explorer" right={<Btn onClick={() => open('repair')}>Repair…</Btn>} />
      </Group>
      <Group title="Sync">
        <Row label="Status" value={config.pause_sync ? 'Paused' : 'Up to date · last change 2 min ago'}
          right={<Btn onClick={() => update({ pause_sync: !config.pause_sync })}>{config.pause_sync ? 'Resume syncing' : 'Pause syncing'}</Btn>} />
        <Row label="Open Beebeeb when you log in" sub="Off: the b and sync only run while Beebeeb is open." right={<Switch label="Open Beebeeb when you log in" on={login} onChange={setLogin} />} />
      </Group>
      <Group title="Space on this PC">
        <Row label="Downloaded files" value={fmtBytes(FIXTURE_LOCAL.downloaded)} mono />
        <Row label="Limit" value="50 GB" mono onClick={() => open('limit-downloads')} />
        <Row label="Keep on this PC" right={<Btn onClick={() => undefined}>Choose folders…</Btn>} />
        <Row label="Available offline" value={`${FIXTURE_LOCAL.offlineItems} items · ${fmtBytes(FIXTURE_LOCAL.offline)}`} mono href="?section=available-offline" />
        <Row label="Free up space…" onClick={() => open('free-up')} />
      </Group>
      <Group>
        <Row label={<span className="text-[color:var(--color-red-text)]">Turn off Beebeeb in File Explorer…</span>} onClick={() => open('explorer-off')} />
      </Group>
    </Page>
  )
}

/** DW-0746b. Blocked (M3e): no command takes Beebeeb out of File Explorer; Turn off says so and changes nothing. */
export function ExplorerOffDialog() {
  const { close } = useSettings()
  const [refused, setRefused] = useState<string | null>(null)
  return (
    <Dialog labelId="exp-h" title="Turn off Beebeeb in File Explorer?" onCancel={close}
      actions={<><Btn onClick={close}>Cancel</Btn><Btn variant="danger" onClick={() => setRefused(`${actionFailure('turn off Beebeeb in File Explorer', 'unsupported').text} Nothing was changed.`)}>Turn off</Btn></>}>
      {refused ? <p role="alert" className="mb-2 text-[12px] text-red-text">{refused}</p> : null}
      <p>2 files, 14 MB, made in File Explorer have not uploaded yet and would be lost.</p>
      <p className="mt-2">Files that have uploaded are safe. The window keeps working.</p>
    </Dialog>
  )
}
export function RepairDialog() {
  const { close } = useSettings()
  return (
    <Dialog labelId="rep-h" title="Repair Beebeeb in File Explorer?" onCancel={close}
      actions={<><Btn onClick={close}>Cancel</Btn><Btn variant="amber" onClick={close}>Repair</Btn></>}>
      Beebeeb registers its folder with Windows again. Your files are not touched.
    </Dialog>
  )
}
