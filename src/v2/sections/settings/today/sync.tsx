// Drive › Sync and File Explorer with what the desktop does today: Keep on this PC, repair (DW-0746c/d) and the
// metered-connection wait (DW-0746e). Every value is read (P1c): Beebeeb's place in File Explorer from
// `windows_shell_integration_state`, the status from the engine's summary, this PC's copies from
// `desktop_storage_summary`, the limit from the desktop config. Never the board's example numbers.
import { useCallback, useEffect, useState } from 'react'
import { useDriveSource, type SyncSummary } from '@beebeeb/shared/drive-ui'
import { command, type CommandResult, type FinderInstallState } from '../../../../desktopApi'
import { actionFailure } from '../../files/model'
import { PHASE } from '../../../chrome/StatusLine'
import { Btn, Dialog, Group, Page, Row, Switch, fmtBytes } from '../shell/kit'
import { useConfig, useLocalSpace } from '../shell/data'
import { ofLimit } from '../drive/storage'
import { useSettings } from '../shell/context'
import { ErrorBanner } from './ui'

// No command repairs the File Explorer registration yet (Repair… says so, DW-0746c): the live row is idle, and a frame
// names the phase it draws.
type Repair = { phase: 'idle' | 'running' | 'failed' | 'done'; kept?: number }

/** The Status row: the engine's phase in the status line's words, with the changes that wait; Paused when paused. */
export function statusValue(sync: SyncSummary | null, paused: boolean, metered: boolean): string | undefined {
  const wait = sync && sync.pending > 0 ? ` · ${sync.pending.toLocaleString('en')} ${sync.pending === 1 ? 'change waits' : 'changes wait'}` : ''
  if (metered) return `Waiting · this PC is on a metered connection${wait}`
  if (paused) return 'Paused'
  if (!sync) return undefined
  return `${PHASE[sync.phase]}${sync.phase === 'syncing' || sync.phase === 'paused' || sync.phase === 'offline' ? wait : ''}`
}

/** The engine's summary, read on open and kept current by its `sync` events. */
function useSyncSummary(): SyncSummary | null {
  const source = useDriveSource()
  const [sync, setSync] = useState<SyncSummary | null>(null)
  useEffect(() => {
    let live = true
    source.sync().then((s) => { if (live) setSync(s) }, () => { if (live) setSync(null) })
    const off = source.subscribe((e) => { if (e.kind === 'sync' && live) setSync(e.sync) })
    return () => { live = false; off() }
  }, [source])
  return sync
}

/**
 * "Beebeeb in File Explorer" (DW-0746#1): on while the sync folder is registered with Windows. Turning it on registers
 * the folder the state names (`install_windows_shell_integration`); no command takes Beebeeb out of File Explorer yet,
 * so turning it off says so and changes nothing (#274 review, sync.tsx:42).
 */
function useExplorerPlace() {
  const [state, setState] = useState<FinderInstallState | null>(null)
  const [read, setRead] = useState<'loading' | 'ready' | 'failed'>('loading')
  const [busy, setBusy] = useState(false)
  useEffect(() => {
    let live = true
    void command<FinderInstallState>('windows_shell_integration_state').then((r) => {
      if (!live) return
      if (r.ok) { setState(r.value); setRead('ready') } else setRead('failed')
    })
    return () => { live = false }
  }, [])
  const flip = useCallback(async (on: boolean): Promise<string | null> => {
    if (!on) return `${actionFailure('take Beebeeb out of File Explorer', 'unsupported').text} Nothing was changed.`
    if (!state?.path) return 'Beebeeb doesn’t know this PC’s Beebeeb folder yet. Try again in a moment.'
    setBusy(true)
    const r = await command<FinderInstallState>('install_windows_shell_integration', { path: state.path })
    setBusy(false)
    if (!r.ok) return actionFailure('add Beebeeb to File Explorer', r.unsupported ? 'unsupported' : 'failed').text
    setState(r.value)
    return null
  }, [state])
  return { installed: !!state?.installed, read, busy, flip }
}

export function SyncToday({ metered, repair: forced }: { metered?: boolean; repair?: Repair['phase'] } = {}) {
  const { open, go } = useSettings()
  const { config, update } = useConfig()
  const sync = useSyncSummary()
  const local = useLocalSpace()
  const place = useExplorerPlace()
  const rep: Repair = { phase: forced ?? 'idle' }
  const [login, setLogin] = useState(true)
  const [refused, setRefused] = useState<string | null>(null)
  const [placeRefused, setPlaceRefused] = useState<string | null>(null)
  const waiting = !!metered
  const say = (what: string) => (r: CommandResult<unknown>) => setRefused(r.ok ? null : actionFailure(what, r.unsupported ? 'unsupported' : 'failed').text)
  // The sync folder in File Explorer (DW-1180). A refusal is the one line, never the host's words.
  const showInExplorer = async () => {
    setRefused(null)
    const shown = await command<void>('open_finder_location', { path: null })
    if (!shown.ok) setRefused(actionFailure('open File Explorer', shown.unsupported ? 'unsupported' : 'failed').text)
  }
  // Open Beebeeb when you log in is the login item (`autostart_enabled`, `toggle_autostart`): read, then flipped.
  useEffect(() => { void command<boolean>('autostart_enabled').then((r) => { if (r.ok) setLogin(r.value) }) }, [])
  const flipLogin = () => {
    setRefused(null)
    void command<boolean>('toggle_autostart').then((r) => { if (r.ok) setLogin(r.value); say('change Open Beebeeb when you log in')(r) })
  }
  const flipPlace = (on: boolean) => { setPlaceRefused(null); void place.flip(on).then(setPlaceRefused) }
  const here = local.state === 'ready' ? local.value : null
  const limit = config.local_cache_limit_bytes ?? 0
  return (
    <Page title="Sync and File Explorer">
      {rep.phase === 'failed' ? (
        <ErrorBanner title="Couldn't repair Beebeeb in File Explorer." action={<Btn onClick={() => open('repair')}>Repair…</Btn>}>Nothing was changed that you need to undo. Try again.</ErrorBanner>
      ) : null}
      <Group title="File Explorer">
        <Row label="Beebeeb in File Explorer" sub={place.read === 'failed' ? 'Beebeeb couldn’t read whether it is in File Explorer.' : "In File Explorer's navigation pane"}
          right={<Switch label="Beebeeb in File Explorer" on={place.installed} onChange={flipPlace} disabled={place.read !== 'ready' || place.busy} />} />
        {placeRefused ? <p role="alert" className="px-4 pb-2 text-[12px] text-red-text">{placeRefused}</p> : null}
        <Row label="Show in File Explorer" onClick={() => void showInExplorer()} />
        <Row label="Show sync status on files" right={<Switch label="Show sync status on files" on={!!config.sync_overlays} onChange={(sync_overlays) => update({ sync_overlays })} />} />
        <Row label="Repair Beebeeb in File Explorer" sub={rep.phase === 'running' ? 'Repairing…' : rep.phase === 'done' && rep.kept ? `${rep.kept} changes waiting to upload were kept.` : undefined}
          right={<Btn onClick={() => open('repair')} disabled={rep.phase === 'running'}>Repair…</Btn>} />
      </Group>
      <Group title="Sync" note={metered ? <>Sync on metered connections is off in <a href="?section=settings&page=bandwidth" onClick={(e) => { e.preventDefault(); go('bandwidth') }} className="underline">Bandwidth</a>, and sync resumes by itself on a connection that isn't metered.</> : undefined}>
        <Row label="Status" value={statusValue(sync, !!config.pause_sync, waiting)}
          right={waiting ? <Btn onClick={() => setRefused(actionFailure('sync on this connection', 'unsupported').text)}>Sync anyway</Btn> : <Btn onClick={() => update({ pause_sync: !config.pause_sync })}>{config.pause_sync ? 'Resume syncing' : 'Pause syncing'}</Btn>} />
        <Row label="Open Beebeeb when you log in" sub="Off: the b and sync only run while Beebeeb is open." right={<Switch label="Open Beebeeb when you log in" on={login} onChange={flipLogin} />} />
        {refused ? <p role="alert" className="px-4 pb-2 text-[12px] text-red-text">{refused}</p> : null}
      </Group>
      <Group title="Space on this PC">
        <Row label="Downloaded files" value={here ? ofLimit(here.cache_bytes, limit) : undefined} mono />
        <Row label="Limit" value={limit > 0 ? fmtBytes(limit) : 'No limit'} mono onClick={() => open('limit-downloads')} />
        <Row label="Keep on this PC" sub="Folders you choose stay on this PC." right={<Btn onClick={() => open('keep')}>Choose folders…</Btn>} />
        <Row label="Available offline" value={here ? `${sync ? `${sync.availableOffline.toLocaleString('en')} items · ` : ''}${fmtBytes(here.pinned_bytes)}` : undefined} mono href="?section=available-offline" />
        <Row label="Free up space…" onClick={() => open('free-up')} />
        {local.state === 'failed' ? <p role="alert" className="px-4 pb-2 text-[12px] text-red-text">{actionFailure('read this PC’s storage', 'failed').text}</p> : null}
      </Group>
      <Group>
        <Row label={<span className="text-[color:var(--color-red-text)]">Turn off Beebeeb in File Explorer…</span>} onClick={() => open('explorer-off')} />
      </Group>
    </Page>
  )
}

/**
 * DW-0746c. Blocked (M3e): no command removes Beebeeb's File Explorer location and adds it back, which is what Repair
 * says it does (`install_windows_shell_integration` only adds). The alert says so and changes nothing: never a
 * pretend repair that "kept 2 changes".
 */
export function RepairAlert() {
  const { close } = useSettings()
  const [refused, setRefused] = useState<string | null>(null)
  const run = () => setRefused(`${actionFailure('repair Beebeeb in File Explorer', 'unsupported').text} Nothing was changed.`)
  return (
    <Dialog labelId="rep-h" title="Repair Beebeeb in File Explorer?" onCancel={close}
      actions={<><Btn onClick={close}>Cancel</Btn><Btn variant="amber" onClick={run}>Repair</Btn></>}>
      {refused ? <p role="alert" className="mb-2 text-[12px] text-red-text">{refused}</p> : null}
      Beebeeb removes its File Explorer location and turns off Open Beebeeb when you log in, then adds itself back to File Explorer. Files waiting to upload are kept.
    </Dialog>
  )
}
