// Drive › Sync and File Explorer with what the desktop does today: Keep on this PC, repair (DW-0746c/d) and the
// metered-connection wait (DW-0746e). Every value is read (P1c): Beebeeb's place in File Explorer from
// `windows_shell_integration_state`, the status from the engine's summary, this PC's copies from
// `desktop_storage_summary`, the limit from the desktop config. Never the board's example numbers.
import { useEffect, useState } from 'react'
import { command, type FinderInstallState } from '../../../../desktopApi'
import { actionFailure } from '../../files/model'
import { Btn, Dialog, Group, Page, Row, Switch, fmtBytes } from '../shell/kit'
import { useConfig, useLocalSpace } from '../shell/data'
import { ofLimit } from '../drive/storage'
import { useSettings } from '../shell/context'
import { ErrorBanner } from './ui'
import { unsentLine, unsentView, type SyncRun } from '../drive/sync'

const SHELL_UNREAD = 'Beebeeb couldn\u2019t read whether it is in File Explorer.'

export function statusValue(state: SyncRun | null | ''): string | undefined {
  if (state === 'running') return 'Running'
  if (state === 'paused') return 'Paused'
  if (state === 'stopped') return 'Stopped'
  if (state === 'refused') return 'Refused'
  return undefined
}

/** Install, Repair, or Off. Installed wins. An error is Repair. Unread is neither. */
export function explorerOffer(state: FinderInstallState | null): 'install' | 'repair' | 'off' | null {
  if (!state) return null
  if (state.installed === true || state.status === 'installed') return 'off'
  if (state.status === 'error' || (typeof state.last_error === 'string' && state.last_error.length > 0)) return 'repair'
  return 'install'
}

type EngineRead = { engine: string; refusal: boolean } | 'failed' | null

function shownRun(pause: boolean | null, status: EngineRead): SyncRun | null {
  if (pause === true) return 'paused'
  if (status === null) return null
  if (status === 'failed' || status.refusal) return 'refused'
  const engine = status.engine
  if (engine === 'error' || engine === 'refused') return 'refused'
  if (engine === 'running' || engine === 'idle' || engine === 'syncing') return 'running'
  if (engine === 'paused') return 'paused'
  return 'stopped'
}

export function SyncToday({ metered, repair }: { metered?: boolean; repair?: 'idle' | 'running' | 'failed' | 'done' } = {}) {
  const { open, go } = useSettings()
  const { config, update } = useConfig()
  const local = useLocalSpace()
  const [login, setLogin] = useState(false)
  const [pause, setPause] = useState<boolean | null>(null)
  const [pauseBusy, setPauseBusy] = useState(false)
  const [pauseError, setPauseError] = useState<string | null>(null)
  const [refused, setRefused] = useState<string | null>(null)
  const [shell, setShell] = useState<FinderInstallState | null>(null)
  const [shellRead, setShellRead] = useState<'loading' | 'ready' | 'failed'>('loading')
  const [shellBusy, setShellBusy] = useState(false)
  const [placeRefused, setPlaceRefused] = useState<string | null>(null)
  const [status, setStatus] = useState<EngineRead>(null)
  const [unsent, setUnsent] = useState<{ state: SyncRun; total: number | null } | null>(null)

  useEffect(() => {
    let live = true
    void command<{ pause_sync?: boolean }>('get_desktop_config').then((r) => {
      if (live && r.ok) setPause(r.value?.pause_sync === true)
    })
    void command<boolean>('autostart_enabled').then((r) => {
      if (live && r.ok) setLogin(r.value === true)
    })
    void command<FinderInstallState>('windows_shell_integration_state').then((r) => {
      if (!live) return
      if (r.ok && r.value && typeof r.value === 'object') { setShell(r.value); setShellRead('ready') }
      else setShellRead('failed')
    })
    void command<{ engine?: unknown; engine_refusal?: unknown }>('sync_status').then((r) => {
      if (!live) return
      if (!r.ok || !r.value || typeof r.value !== 'object') { setStatus('failed'); return }
      const engine = typeof r.value.engine === 'string' ? r.value.engine : ''
      setStatus({ engine, refusal: r.value.engine_refusal != null })
    })
    void command<unknown>('unsent_changes', { offset: 0, limit: 1 }).then((r) => {
      if (live) setUnsent(unsentView(r.ok ? r.value : null, r.ok))
    })
    return () => { live = false }
  }, [])

  const flipLogin = () => {
    setRefused(null)
    void command<boolean>('toggle_autostart').then((r) => {
      if (r.ok) setLogin(r.value === true)
      else setRefused(actionFailure('change Open Beebeeb when you log in', r.unsupported ? 'unsupported' : 'failed').text)
    })
  }

  // The switch stays put until this write answers. A refusal is the action's own sentence.
  const flipPause = () => {
    if (pause === null || pauseBusy) return
    const next = !pause
    setPauseBusy(true)
    setPauseError(null)
    void command('set_desktop_config', { config: { pause_sync: next } }).then((r) => {
      setPauseBusy(false)
      if (r.ok) setPause(next)
      else setPauseError(actionFailure(next ? 'pause sync' : 'resume sync', r.unsupported ? 'unsupported' : 'failed').text)
    })
  }

  const offer = shellRead === 'ready' ? explorerOffer(shell) : null
  const installOrRepair = (which: 'install' | 'repair') => {
    if (!shell || shellBusy) return
    setShellBusy(true)
    setPlaceRefused(null)
    void command<FinderInstallState>('install_windows_shell_integration', { path: shell.path ?? null }).then((r) => {
      setShellBusy(false)
      if (!r.ok) {
        const action = which === 'repair' ? 'repair Beebeeb in File Explorer' : 'add Beebeeb to File Explorer'
        setPlaceRefused(actionFailure(action, r.unsupported ? 'unsupported' : 'failed').text)
        return
      }
      if (r.value && typeof r.value === 'object') setShell(r.value)
    })
  }
  const showInExplorer = () => {
    setRefused(null)
    void command<void>('open_finder_location', { path: shell?.path ?? null }).then((shown) => {
      if (!shown.ok) setRefused(actionFailure('open File Explorer', shown.unsupported ? 'unsupported' : 'failed').text)
    })
  }

  const word = statusValue(shownRun(pause, status) ?? '')
  const line = unsent ? unsentLine(unsent.state, unsent.total) : null
  const linked = !!unsent && unsent.state !== 'refused' && unsent.total !== null && unsent.total > 0
  const here = local.state === 'ready' ? local.value : null
  const limit = config.local_cache_limit_bytes ?? 0
  const folder = shell && typeof shell.path === 'string' && shell.path.length > 0 ? shell.path : 'Not reported'
  const offerButton = offer === 'install'
    ? <Btn variant="amber" onClick={() => installOrRepair('install')} disabled={shellBusy}>Install</Btn>
    : offer === 'repair'
      ? <Btn variant="amber" onClick={() => installOrRepair('repair')} disabled={shellBusy}>Repair</Btn>
      : offer === 'off'
        ? <Btn variant="danger" onClick={() => open('explorer-off')}>Off</Btn>
        : null

  return (
    <Page title="Sync and File Explorer">
      {repair === 'failed' ? (
        <ErrorBanner title="Couldn't repair Beebeeb in File Explorer." action={<Btn onClick={() => open('repair')}>Repair…</Btn>}>Nothing was changed that you need to undo. Try again.</ErrorBanner>
      ) : null}
      <Group title="File Explorer">
        <Row label="Beebeeb in File Explorer" sub={shellRead === 'failed' ? SHELL_UNREAD : "In File Explorer's navigation pane"}
          value={shellRead === 'ready' ? <span data-testid="explorer-on">{offer === 'off' ? 'On' : 'Off'}</span> : undefined}
          right={offerButton} />
        {placeRefused ? <p role="alert" className="px-4 pb-2 text-[12px] text-red-text">{placeRefused}</p> : null}
        <Row label="Sync folder" value={shellRead === 'ready' ? <span data-testid="sync-folder">{folder}</span> : undefined} />
        <Row label="Show in File Explorer" onClick={showInExplorer} />
        <Row label="Show sync status on files" right={<Switch label="Show sync status on files" on={config.sync_overlays === true} onChange={(sync_overlays) => update({ sync_overlays })} />} />
      </Group>
      <Group title="Sync" note={metered ? <>Sync on metered connections is off in <a href="?section=settings&page=bandwidth" onClick={(e) => { e.preventDefault(); go('bandwidth') }} className="underline">Bandwidth</a>, and sync resumes by itself on a connection that isn't metered.</> : undefined}>
        <Row label="Status" value={word ? <span data-testid="sync-status">{word}</span> : undefined}
          right={<Switch label="Pause sync" on={pause === true} onChange={flipPause} disabled={pause === null || pauseBusy} />} />
        {line ? (
          <Row label={linked ? 'Sync history' : 'Changes on this PC'} href={linked ? '?section=sync-history' : undefined}
            value={<span data-testid="unsent-line">{line}</span>} />
        ) : null}
        <Row label="Open Beebeeb when you log in" sub="Off: the b and sync only run while Beebeeb is open."
          right={<Switch label="Open Beebeeb when you log in" on={login} onChange={flipLogin} />} />
        {pauseError ? <p role="alert" className="px-4 pb-2 text-[12px] text-red-text">{pauseError}</p> : null}
        {refused ? <p role="alert" className="px-4 pb-2 text-[12px] text-red-text">{refused}</p> : null}
      </Group>
      <Group title="Space on this PC">
        <Row label="Downloaded files" value={here && here.cache_bytes > 0 ? ofLimit(here.cache_bytes, limit) : undefined} mono />
        <Row label="Limit" value={limit > 0 ? fmtBytes(limit) : 'No limit'} mono onClick={() => open('limit-downloads')} />
        <Row label="Keep on this PC" sub="Folders you choose stay on this PC." right={<Btn onClick={() => open('keep')}>Choose folders…</Btn>} />
        <Row label="Available offline" value={here && here.pinned_bytes > 0 ? fmtBytes(here.pinned_bytes) : undefined} mono href="?section=available-offline" />
        <Row label="Free up space…" onClick={() => open('free-up')} />
        {local.state === 'failed' ? <p role="alert" className="px-4 pb-2 text-[12px] text-red-text">{actionFailure('read this PC\u2019s storage', 'failed').text}</p> : null}
      </Group>
    </Page>
  )
}

/** No command removes the File Explorer location and adds it back. Repair says so and changes nothing. */
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
