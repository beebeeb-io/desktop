// Drive › Sync and File Explorer with what the desktop does today: Keep on this PC, repair (DW-0746c/d) and the
// metered-connection wait (DW-0746e). B12's pane (drive/sync.tsx) stays as it is; this one replaces it in the list.
import { useState, useSyncExternalStore } from 'react'
import { Btn, Dialog, Group, Page, Row, Switch, fmtBytes } from '../shell/kit'
import { FIXTURE_LOCAL, useConfig } from '../shell/data'
import { useSettings } from '../shell/context'
import { ErrorBanner } from './ui'

type Repair = { phase: 'idle' | 'running' | 'failed' | 'done'; kept?: number }
let repair: Repair = { phase: 'idle' }
const subs = new Set<() => void>()
const setRepair = (r: Repair) => { repair = r; subs.forEach((f) => f()) }
const useRepair = () => useSyncExternalStore((f) => { subs.add(f); return () => { subs.delete(f) } }, () => repair)

export function SyncToday({ metered, repair: forced }: { metered?: boolean; repair?: Repair['phase'] } = {}) {
  const { open, go } = useSettings()
  const { config, update } = useConfig()
  const live = useRepair()
  const rep: Repair = forced ? { phase: forced } : live
  const [login, setLogin] = useState(true)
  const [nav, setNav] = useState(true)
  const [anyway, setAnyway] = useState(false)
  const waiting = !!metered && !anyway
  return (
    <Page title="Sync and File Explorer">
      {rep.phase === 'failed' ? (
        <ErrorBanner title="Couldn't repair Beebeeb in File Explorer." action={<Btn onClick={() => open('repair')}>Repair…</Btn>}>Nothing was changed that you need to undo. Try again.</ErrorBanner>
      ) : null}
      <Group title="File Explorer">
        <Row label="Beebeeb in File Explorer" sub="In File Explorer's navigation pane" right={<Switch label="Beebeeb in File Explorer" on={nav} onChange={setNav} />} />
        <Row label="Show in File Explorer" onClick={() => undefined} />
        <Row label="Show sync status on files" right={<Switch label="Show sync status on files" on={!!config.sync_overlays} onChange={(sync_overlays) => update({ sync_overlays })} />} />
        <Row label="Repair Beebeeb in File Explorer" sub={rep.phase === 'running' ? 'Repairing…' : rep.phase === 'done' && rep.kept ? `${rep.kept} changes waiting to upload were kept.` : undefined}
          right={<Btn onClick={() => open('repair')} disabled={rep.phase === 'running'}>Repair…</Btn>} />
      </Group>
      <Group title="Sync" note={metered ? <>Sync on metered connections is off in <a href="?section=settings&sub=bandwidth" onClick={(e) => { e.preventDefault(); go('bandwidth') }} className="underline">Bandwidth</a>, and sync resumes by itself on a connection that isn't metered.</> : undefined}>
        <Row label="Status" value={waiting ? 'Waiting · this PC is on a metered connection · 7 changes wait' : config.pause_sync ? 'Paused' : 'Up to date · last change 2 min ago'}
          right={waiting ? <Btn onClick={() => setAnyway(true)}>Sync anyway</Btn> : <Btn onClick={() => update({ pause_sync: !config.pause_sync })}>{config.pause_sync ? 'Resume syncing' : 'Pause syncing'}</Btn>} />
        <Row label="Open Beebeeb when you log in" sub="Off: the b and sync only run while Beebeeb is open." right={<Switch label="Open Beebeeb when you log in" on={login} onChange={setLogin} />} />
      </Group>
      <Group title="Space on this PC">
        <Row label="Downloaded files" value={fmtBytes(FIXTURE_LOCAL.downloaded)} mono />
        <Row label="Limit" value="50 GB" mono onClick={() => open('limit-downloads')} />
        <Row label="Keep on this PC" sub="Folders you choose stay on this PC." right={<Btn onClick={() => open('keep')}>Choose folders…</Btn>} />
        <Row label="Available offline" value={`${FIXTURE_LOCAL.offlineItems} items · ${fmtBytes(FIXTURE_LOCAL.offline)}`} mono href="?section=available-offline" />
        <Row label="Free up space…" onClick={() => open('free-up')} />
      </Group>
      <Group>
        <Row label={<span className="text-[color:var(--color-red-text)]">Turn off Beebeeb in File Explorer…</span>} onClick={() => open('explorer-off')} />
      </Group>
    </Page>
  )
}

export function RepairAlert() {
  const { close } = useSettings()
  const run = () => {
    close()
    setRepair({ phase: 'running' })
    setTimeout(() => setRepair({ phase: 'done', kept: 2 }), 1200)
  }
  return (
    <Dialog labelId="rep-h" title="Repair Beebeeb in File Explorer?" onCancel={close}
      actions={<><Btn onClick={close}>Cancel</Btn><Btn variant="amber" onClick={run}>Repair</Btn></>}>
      Beebeeb removes its File Explorer location and turns off Open Beebeeb when you log in, then adds itself back to File Explorer. Files waiting to upload are kept.
    </Dialog>
  )
}
