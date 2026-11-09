// Drive › Sync and File Explorer with what the desktop does today: Keep on this PC, repair (DW-0746c/d) and the
// metered-connection wait (DW-0746e). B12's pane (drive/sync.tsx) stays as it is; this one replaces it in the list.
import { useEffect, useState } from 'react'
import { command, type CommandResult } from '../../../../desktopApi'
import { actionFailure } from '../../files/model'
import { Btn, Dialog, Group, Page, Row, Switch, fmtBytes } from '../shell/kit'
import { FIXTURE_LOCAL, useConfig } from '../shell/data'
import { useSettings } from '../shell/context'
import { ErrorBanner } from './ui'

// No command repairs the File Explorer registration yet (Repair… says so, DW-0746c): the live row is idle, and a frame
// names the phase it draws.
type Repair = { phase: 'idle' | 'running' | 'failed' | 'done'; kept?: number }

export function SyncToday({ metered, repair: forced }: { metered?: boolean; repair?: Repair['phase'] } = {}) {
  const { open, go } = useSettings()
  const { config, update } = useConfig()
  const rep: Repair = { phase: forced ?? 'idle' }
  const [login, setLogin] = useState(true)
  const [nav, setNav] = useState(true)
  const [refused, setRefused] = useState<string | null>(null)
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
  return (
    <Page title="Sync and File Explorer">
      {rep.phase === 'failed' ? (
        <ErrorBanner title="Couldn't repair Beebeeb in File Explorer." action={<Btn onClick={() => open('repair')}>Repair…</Btn>}>Nothing was changed that you need to undo. Try again.</ErrorBanner>
      ) : null}
      <Group title="File Explorer">
        <Row label="Beebeeb in File Explorer" sub="In File Explorer's navigation pane" right={<Switch label="Beebeeb in File Explorer" on={nav} onChange={setNav} />} />
        <Row label="Show in File Explorer" onClick={() => void showInExplorer()} />
        <Row label="Show sync status on files" right={<Switch label="Show sync status on files" on={!!config.sync_overlays} onChange={(sync_overlays) => update({ sync_overlays })} />} />
        <Row label="Repair Beebeeb in File Explorer" sub={rep.phase === 'running' ? 'Repairing…' : rep.phase === 'done' && rep.kept ? `${rep.kept} changes waiting to upload were kept.` : undefined}
          right={<Btn onClick={() => open('repair')} disabled={rep.phase === 'running'}>Repair…</Btn>} />
      </Group>
      <Group title="Sync" note={metered ? <>Sync on metered connections is off in <a href="?section=settings&page=bandwidth" onClick={(e) => { e.preventDefault(); go('bandwidth') }} className="underline">Bandwidth</a>, and sync resumes by itself on a connection that isn't metered.</> : undefined}>
        <Row label="Status" value={waiting ? 'Waiting · this PC is on a metered connection · 7 changes wait' : config.pause_sync ? 'Paused' : 'Up to date · last change 2 min ago'}
          right={waiting ? <Btn onClick={() => setRefused(actionFailure('sync on this connection', 'unsupported').text)}>Sync anyway</Btn> : <Btn onClick={() => update({ pause_sync: !config.pause_sync })}>{config.pause_sync ? 'Resume syncing' : 'Pause syncing'}</Btn>} />
        <Row label="Open Beebeeb when you log in" sub="Off: the b and sync only run while Beebeeb is open." right={<Switch label="Open Beebeeb when you log in" on={login} onChange={flipLogin} />} />
        {refused ? <p role="alert" className="px-4 pb-2 text-[12px] text-red-text">{refused}</p> : null}
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
