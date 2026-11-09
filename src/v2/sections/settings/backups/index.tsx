// Drive › Backups on Windows (DW-0737, DW-1411, DW-0738, DW-1410, DW-0739, DW-0742, DW-0741).
// Every value is read (P1c): this PC's folders from get/set_known_folder_backup, the status card from the engine's
// backup view (`drive_status`) and its problems list (`drive_sync_problems`), the other devices and the services from
// the Backups folder's own listing (DriveSource). What no command reads says so; the board's numbers stay in its frames.
import { useCallback, useEffect, useRef, useState } from 'react'
import { Btn, Dialog, Group, Page, Row, Switch, mono } from '../shell/kit'
import { useKnownFolders } from '../shell/data'
import { useSettings } from '../shell/context'
import { useDriveSource, type DriveItem } from '@beebeeb/shared/drive-ui'
import { useDriveExtras, type BackupView, type SyncProblem } from '../../../source/extras'
import { actionFailure } from '../../files/model'
import { newestOnly, throttled } from '../../../throttle'
import { board } from '../../../board'

export type BackupState = 'idle' | 'running'

/** The service folders in Backups (a Beebeeb service manages them, DW-0425); the rest are devices. */
const SERVICES = new Set(['Mail Backup'])

/** "09:30" today, else "8 Nov": when the newest backup pass finished. */
export function lastBackupText(iso: string, now = new Date()): string {
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return ''
  const sameDay = d.toDateString() === now.toDateString()
  return sameDay ? d.toTimeString().slice(0, 5) : d.toLocaleDateString('en-GB', { day: 'numeric', month: 'short' })
}

/** A running pass: folder count from this read, or done/total when the read carried them. No size and no clock. */
function runningLine(view: BackupView): string | null {
  const extra = view as BackupView & { done?: unknown; total?: unknown }
  if (typeof extra.done === 'number' && typeof extra.total === 'number'
    && Number.isFinite(extra.done) && Number.isFinite(extra.total) && extra.done >= 0 && extra.total > 0) {
    return `${extra.done.toLocaleString('en')} of ${extra.total.toLocaleString('en')}`
  }
  const n = view.foldersOn
  if (typeof n === 'number' && Number.isFinite(n) && n > 0) return `${n.toLocaleString('en')} ${n === 1 ? 'folder' : 'folders'}`
  return null
}

/** The card's title and line, from the engine's backup view and how many backup items it stopped trying. */
export function cardWords(view: BackupView | null, failed: number, now = new Date()): { title: string; line: string | null } {
  if (!view) return { title: 'Not reported yet', line: null }
  if (view.state === 'getting-ready') return { title: 'Backing up', line: runningLine(view) }
  const title = view.state === 'off' ? 'Backups are off'
    : view.state === 'stopped' ? 'Backups stopped'
      : failed > 0 ? 'Backed up, except a few items' : 'All backed up'
  const line = view.lastBackupAt ? `Last backup ${lastBackupText(view.lastBackupAt, now)}` : view.state === 'off' ? null : 'No backup has finished yet'
  return { title, line }
}

/** The engine's backup view and its backup problems, read on open and with each sync change. */
function useBackupStatus() {
  const extras = useDriveExtras()
  const source = useDriveSource()
  const [view, setView] = useState<BackupView | null>(null)
  const [failed, setFailed] = useState(0)
  const readRef = useRef<() => void>(() => {})
  useEffect(() => {
    let live = true
    const read = () => {
      extras.status().then((s) => { if (live) setView(s.backup) }, () => { if (live) setView(null) })
      extras.syncProblems().then((list) => { if (live) setFailed(list.filter((p) => p.backup && p.retryable).length) }, () => { if (live) setFailed(0) })
    }
    readRef.current = read
    read()
    const off = source.subscribe((e) => { if (e.kind === 'sync') read() })
    return () => { live = false; off() }
  }, [extras, source])
  return { view, failed, retry: () => readRef.current() }
}

/** The Backups folder's children (DriveSource): each device's backup, and the services. This PC's own is left out. */
function useBackupFolders(thisPc: string | undefined) {
  const source = useDriveSource()
  const [rows, setRows] = useState<DriveItem[] | null>(null)
  const [failed, setFailed] = useState(false)
  useEffect(() => {
    let live = true
    source.list(null)
      .then((root) => {
        // The protected folder is a row of the root listing, as the Backups section finds it (FolderView).
        const backups = root.items.find((i) => i.isFolder && i.name === 'Backups')
        return backups ? source.list(backups.id).then((l) => l.items.filter((i) => i.isFolder)) : []
      })
      .then((items) => { if (live) setRows(items) }, () => { if (live) setFailed(true) })
    return () => { live = false }
  }, [source])
  const all = rows ?? []
  return {
    loaded: rows !== null,
    failed,
    devices: all.filter((i) => !SERVICES.has(i.name) && i.name !== thisPc),
    services: all.filter((i) => SERVICES.has(i.name)),
  }
}

const itemsText = (n: number | null | undefined) => (n == null ? undefined : `${n.toLocaleString('en')} ${n === 1 ? 'item' : 'items'}`)

function StatusCard({ state, view, failed, onBackUpNow, onRetry }: { state: BackupState; view: BackupView | null; failed: number; onBackUpNow: () => void; onRetry: () => void }) {
  const { go } = useSettings()
  // A frame's running pass still says only what this read reported.
  const reported = state === 'running' && view && view.state !== 'off' && view.state !== 'stopped'
    ? { ...view, state: 'getting-ready' as const }
    : view
  const words = cardWords(reported, failed)
  if (!reported) {
    return (
      <div className="flex items-center gap-3 rounded-[12px] border border-line bg-paper px-4 py-3" data-testid="backup-status">
        <div className="flex-1">
          <div className="text-[14px] font-medium text-ink">{words.title}</div>
        </div>
        <Btn onClick={onRetry}>Try again</Btn>
      </div>
    )
  }
  return (
    <div className="flex items-center gap-3 rounded-[12px] border border-line bg-paper px-4 py-3" data-testid="backup-status">
      <div className="flex-1">
        <div className="text-[14px] font-medium text-ink">{words.title}</div>
        {words.line ? <div className="text-[12px] text-ink-3"><span className={mono}>{words.line}</span> from this PC</div> : null}
        {failed > 0 ? (
          <a href="?section=settings&page=backup-failures" onClick={(e) => { e.preventDefault(); go('backup-failures') }} className="text-[12px] text-ink-2 underline">
            Couldn't back up {failed.toLocaleString('en')} {failed === 1 ? 'item' : 'items'}
          </a>
        ) : null}
      </div>
      <Btn onClick={onBackUpNow}>Back up now</Btn>
    </div>
  )
}

export function Backups({ state = 'idle' }: { state?: BackupState } = {}) {
  const { go, open } = useSettings()
  const { folders, toggle, error } = useKnownFolders()
  const { view, failed, retry } = useBackupStatus()
  const backupFolders = useBackupFolders(view?.deviceFolder)
  // Blocked: no command starts a backup pass now (the engine backs up on its own schedule). Said, never pretended.
  const [refused, setRefused] = useState<string | null>(null)
  return (
    <Page title="Backups">
      <StatusCard state={state} view={view} failed={failed} onBackUpNow={() => setRefused(actionFailure('start a backup now', 'unsupported').text)} onRetry={retry} />
      {refused ? <p role="alert" className="-mt-3 px-1 text-[12px] text-red-text">{refused}</p> : null}
      <Group title="This PC" note="OneDrive's folder backup may keep Desktop, Documents and Pictures in the OneDrive folder. Beebeeb backs up the folders in your user folder, not OneDrive's copies; add OneDrive's folder below to back those up too.">
        {folders.map((f) => (
          <Row key={f.key} label={f.display_name}
            sub={f.enabled ? (f.last ? `Last backup ${f.last}` : 'Waiting for the first backup') : undefined}
            value={f.size ?? itemsText(f.item_count)} mono
            right={<>
              {f.custom ? <Btn variant="ghost" onClick={() => open(`stop:${f.display_name}`)} aria-label={`Remove ${f.display_name}`}>Remove</Btn> : null}
              <Switch label={`Back up ${f.display_name}`} on={f.enabled} onChange={(v) => toggle(f.key, v)} />
            </>} />
        ))}
        {error ? <p role="alert" className="px-4 py-2 text-[12px] text-red-text">{actionFailure('read this PC’s backup folders', 'failed').text}</p> : null}
        <Row label="Add a folder…" onClick={() => open('picker')} />
      </Group>
      <Group title="Other devices">
        {backupFolders.devices.map((d) => (
          <Row key={d.id} label={d.name} sub={itemsText(d.childCount)} onClick={() => go('device', { props: { device: { id: d.id, name: d.name, items: d.childCount ?? null, changedAt: d.modifiedAt ?? null } } })} />
        ))}
        {backupFolders.loaded && !backupFolders.devices.length ? <div className="px-4 py-3 text-[13px] text-ink-3">No other device backs up to this account yet.</div> : null}
        {backupFolders.failed ? <p role="alert" className="px-4 py-2 text-[12px] text-red-text">{actionFailure('read the other devices’ backups', 'failed').text}</p> : null}
      </Group>
      <Group title="Beebeeb services">
        {/* Mail Backup's own page (DW-0740) is a proposal the board leaves out: the row says where it is and goes nowhere. */}
        {backupFolders.services.map((s) => <Row key={s.id} label={s.name} sub="In Backups, managed by Beebeeb" />)}
        {backupFolders.loaded && !backupFolders.services.length ? <div className="px-4 py-3 text-[13px] text-ink-3">No Beebeeb service backs up here yet.</div> : null}
      </Group>
    </Page>
  )
}

/**
 * DW-0741: the backup items Beebeeb stopped trying, from the engine's problems list (`drive_sync_problems`, the backup
 * ones), each with Try again (`drive_sync_retry`); exclusions are listed apart, without it. The list is the engine's
 * truth: it is read again after each Try again and on each sync change, so an item leaves once it is queued again and
 * one that fails again comes back with Try again. What the retry answered is one line, as Sync history says it.
 * `problems`: a frame's list.
 */
export function BackupFailures({ problems }: { problems?: SyncProblem[] } = {}) {
  const { go } = useSettings()
  const extras = useDriveExtras()
  const source = useDriveSource()
  const [list, setList] = useState<SyncProblem[] | null>(problems ?? null)
  // The ids whose Try again is on its way (until the list has been read again).
  const [trying, setTrying] = useState<ReadonlySet<string>>(new Set())
  const [note, setNote] = useState<{ text: string; failed: boolean } | null>(null)
  // Each read is the newest until a later one starts; an older answer never replaces a newer list (#274 review).
  const newestRead = useRef(newestOnly())
  const load = useCallback(async () => {
    if (problems) return
    const newest = newestRead.current()
    try {
      const all = await extras.syncProblems()
      if (newest()) setList(all.filter((p) => p.backup))
    } catch {
      if (newest()) setNote({ text: actionFailure('load the list', 'failed').text, failed: true })
    }
  }, [extras, problems])
  useEffect(() => {
    void load()
    // At most one read a second: the engine sends a `sync` summary on every change while it works.
    const soon = throttled(() => void load())
    const stop = source.subscribe((e) => { if (e.kind === 'sync') soon() })
    return () => { soon.cancel(); stop() }
  }, [source, load])
  const retry = async (ids: string[]) => {
    setNote(null)
    setTrying((t) => new Set([...t, ...ids]))
    try {
      const { retried } = await extras.retry(ids)
      setNote({ text: retried > 0 ? 'Trying again.' : 'Beebeeb is already trying again.', failed: false })
    } catch {
      setNote({ text: actionFailure('try again', 'failed').text, failed: true })
    }
    await load()
    setTrying((t) => new Set([...t].filter((id) => !ids.includes(id))))
  }
  const failures = (list ?? []).filter((p) => p.retryable)
  const excluded = (list ?? []).filter((p) => !p.retryable)
  const waiting = failures.filter((f) => !trying.has(f.id))
  return (
    <Page title="Couldn't back up"
      back={<a href="?section=settings&page=backups" onClick={(e) => { e.preventDefault(); go('backups') }} className="mb-2 inline-block text-[13px] text-ink-2 hover:underline">Back</a>}
      action={waiting.length ? <Btn onClick={() => void retry(waiting.map((f) => f.id))}>Try all again</Btn> : undefined}>
      <p className="text-[13px] text-ink-2">{list && !failures.length ? 'Everything is backed up.' : 'Beebeeb stopped trying these after several tries. Everything else is backed up.'}</p>
      {note ? <p role={note.failed ? 'alert' : 'status'} className={`text-[12px] ${note.failed ? 'text-red-text' : 'text-ink-3'}`}>{note.text}</p> : null}
      {failures.length ? (
        <Group>
          {failures.map((f) => (
            <Row key={f.id} label={f.name} sub={`${f.folder}: ${f.reason}`}
              right={trying.has(f.id) ? <Btn disabled>Trying again…</Btn> : <Btn onClick={() => void retry([f.id])}>Try again</Btn>} />
          ))}
        </Group>
      ) : null}
      {excluded.length ? (
        <Group title="Not backed up, by design">
          {excluded.map((f) => <Row key={f.id} label={f.name} sub={`${f.folder}: ${f.reason}`} />)}
        </Group>
      ) : null}
    </Page>
  )
}

/** A device's backup as Backups lists it: the folder, what it holds, when it last changed. */
export interface DeviceBackup { id: string; name: string; items: number | null; changedAt: string | null }

/** The board's device page, for a frame (no command reads a device's photo count, size or sign-in state yet). */
interface BoardDevice { name: string; backedUp: string; last: string; status: string }

const changedText = (iso: string | null) => (iso ? new Date(iso).toLocaleDateString('en-GB', { day: 'numeric', month: 'short', year: 'numeric' }) : undefined)

export function DevicePane({ device }: { device?: DeviceBackup } = {}) {
  const { go, open } = useSettings()
  const drawn = device ? undefined : board<BoardDevice>('devicePage')
  const back = <a href="?section=settings&page=backups" onClick={(e) => { e.preventDefault(); go('backups') }} className="mb-2 inline-block text-[13px] text-ink-2 hover:underline">Back</a>
  if (!device && !drawn) {
    return (
      <Page title="A device's backup" back={back}>
        <p className="text-[13px] text-ink-2">Choose a device under Other devices in Backups.</p>
      </Page>
    )
  }
  const name = device?.name ?? drawn!.name
  return (
    <Page title={name} back={back}>
      <Group>
        {drawn ? (
          <>
            <Row label="Backed up" value={drawn.backedUp} mono />
            <Row label="Last backup" value={drawn.last} mono />
            <Row label="Status" value={drawn.status} />
          </>
        ) : (
          <>
            <Row label="In the backup" value={itemsText(device!.items)} mono />
            <Row label="Last change" value={changedText(device!.changedAt)} mono />
          </>
        )}
      </Group>
      <Group title="See it">
        <Row label="Open in Photos" href="?section=photos" />
        <Row label="Open in Files" href={device ? `?section=backups&id=${encodeURIComponent(device.id)}` : '?section=backups'} />
      </Group>
      <Group title="Remove" note="A signed-in device's own backup keeps running.">
        <Row label={<span className="text-[color:var(--color-red-text)]">Delete this backup</span>} onClick={() => open(`delete-backup:${name}`)} />
      </Group>
    </Page>
  )
}

export function StopBackupDialog({ arg }: { arg?: string }) {
  const { close } = useSettings()
  const name = arg || 'this folder'
  return (
    <Dialog labelId="stop-h" title={`Stop backing up ${name}?`} onCancel={close}
      actions={<><Btn onClick={close}>Cancel</Btn><Btn onClick={close}>Stop and move the backup to Trash</Btn><Btn variant="amber" onClick={close}>Stop and keep the backup</Btn></>}>
      <p>Keep the backup and it stays in Backups, read-only. Move it to Trash and it is removed after 30 days.</p>
      <p className="mt-2">Nothing on this PC is touched.</p>
    </Dialog>
  )
}

export function DeleteBackupDialog({ arg }: { arg?: string }) {
  const { close } = useSettings()
  const [code, setCode] = useState('')
  const [refused, setRefused] = useState<string | null>(null)
  const drawn = board<BoardDevice>('devicePage')
  const ok = /^\d{6}$/.test(code)
  // Blocked: no command deletes another device's backup with a second-factor code yet. The dialog says so and keeps
  // everything; it never closes as if the backup had gone.
  const remove = () => setRefused(actionFailure('delete the backup', 'unsupported').text + ' Nothing was deleted.')
  const what = drawn && !arg ? `${drawn.backedUp} from ${drawn.name}` : `Everything in ${arg || 'this device'}’s backup`
  return (
    <Dialog labelId="del-h" title="Delete this backup?" onCancel={close}
      actions={<><Btn onClick={close}>Cancel</Btn><Btn variant="danger" disabled={!ok} onClick={remove}>Delete backup</Btn></>}>
      {refused ? <p role="alert" className="mb-2 text-[12px] text-red-text">{refused}</p> : null}
      <p>{what} {drawn && !arg ? 'move' : 'moves'} to Trash. Your other devices get an alert.</p>
      <label className="mt-3 block text-[12px] text-ink-3" htmlFor="code">Authenticator code</label>
      <input id="code" inputMode="numeric" maxLength={6} value={code} onChange={(e) => setCode(e.target.value.replace(/\D/g, ''))}
        className={`mt-1 h-8 w-full rounded-md border border-line bg-paper px-2 text-[14px] text-ink ${mono}`} />
    </Dialog>
  )
}

/**
 * DW-0738: Windows draws the folder picker; the board's list is a frame's. No command adds a folder beyond the known
 * folders yet (only get/set_known_folder_backup exist), so Back up says so and adds nothing: never a row that is not
 * backed up.
 */
export function FolderPicker() {
  const { close } = useSettings()
  const dirs = board<string[]>('pickerDirs') ?? []
  const [sel, setSel] = useState(dirs[0] ?? '')
  const [refused, setRefused] = useState<string | null>(null)
  return (
    <Dialog labelId="pick-h" title="Choose a folder to back up" onCancel={close}
      actions={<><Btn onClick={close}>Cancel</Btn><Btn variant="amber" onClick={() => setRefused(`${actionFailure('back up another folder', 'unsupported').text} Nothing was added.`)}>Back up</Btn></>}>
      {refused ? <p role="alert" className="mb-2 text-[12px] text-red-text">{refused}</p> : null}
      {dirs.length ? (
        <ul role="listbox" aria-label="Folders" className="max-h-40 overflow-auto rounded-md border border-line">
          {dirs.map((d) => (
            <li key={d} role="option" aria-selected={sel === d}>
              <button type="button" onClick={() => setSel(d)} className={`w-full px-3 py-1.5 text-left text-[13px] ${sel === d ? 'bg-paper-3 text-ink' : 'text-ink-2 hover:bg-paper-2'}`}>{d}</button>
            </li>
          ))}
        </ul>
      ) : <p>Beebeeb backs up the folders listed under This PC. Choosing another folder isn’t available in this build yet.</p>}
      <p className="mt-2 text-[12px] text-ink-3">Beebeeb's own folder in File Explorer can't be chosen, nor a folder inside it.</p>
    </Dialog>
  )
}
