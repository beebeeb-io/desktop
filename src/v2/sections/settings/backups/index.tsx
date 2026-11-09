// Drive › Backups on Windows (DW-0737, DW-1411, DW-0738, DW-1410, DW-0739, DW-0742, DW-0741).
// Folder state comes from the existing engine commands (get/set_known_folder_backup).
import { useCallback, useEffect, useState } from 'react'
import { Btn, Dialog, Group, Meter, Page, Row, Switch, mono } from '../shell/kit'
import { FIXTURE_DEVICES, useKnownFolders } from '../shell/data'
import { useSettings } from '../shell/context'
import { useDriveSource } from '@beebeeb/shared/drive-ui'
import { useDriveExtras, type SyncProblem } from '../../../source/extras'
import { actionFailure } from '../../files/model'

export type BackupState = 'idle' | 'running'

function StatusCard({ state, onBackUpNow }: { state: BackupState; onBackUpNow: () => void }) {
  const { go } = useSettings()
  if (state === 'running') {
    return (
      <div className="rounded-[12px] border border-line bg-paper px-4 py-3" data-testid="backup-status">
        <div className="flex items-baseline justify-between"><span className="text-[14px] font-medium text-ink">Backing up Documents</span>
          <span className={`text-[12px] text-ink-3 ${mono}`}>about 20 min left</span></div>
        <div className="my-2"><Meter label="Backing up Documents" fraction={612 / 1820} amber /></div>
        <div className={`text-[12px] text-ink-3 ${mono}`}>612 of 1,820 files</div>
      </div>
    )
  }
  return (
    <div className="flex items-center gap-3 rounded-[12px] border border-line bg-paper px-4 py-3" data-testid="backup-status">
      <div className="flex-1">
        <div className="text-[14px] font-medium text-ink">All backed up</div>
        <div className={`text-[12px] text-ink-3`}><span className={mono}>Last backup 09:30 · 27.8 GB</span> from this PC</div>
        <a href="?section=settings&page=backup-failures" onClick={(e) => { e.preventDefault(); go('backup-failures') }} className="text-[12px] text-ink-2 underline">Couldn't back up 3 items</a>
      </div>
      <Btn onClick={onBackUpNow}>Back up now</Btn>
    </div>
  )
}

export function Backups({ state = 'idle' }: { state?: BackupState } = {}) {
  const { go, open } = useSettings()
  const { folders, toggle } = useKnownFolders()
  // Blocked: no command starts a backup pass now (the engine backs up on its own schedule). Said, never pretended.
  const [refused, setRefused] = useState<string | null>(null)
  return (
    <Page title="Backups">
      <StatusCard state={state} onBackUpNow={() => setRefused(actionFailure('start a backup now', 'unsupported').text)} />
      {refused ? <p role="alert" className="-mt-3 px-1 text-[12px] text-red-text">{refused}</p> : null}
      <Group title="This PC" note="OneDrive's folder backup may keep Desktop, Documents and Pictures in the OneDrive folder. Beebeeb backs up the folders in your user folder, not OneDrive's copies; add OneDrive's folder below to back those up too.">
        {folders.map((f) => (
          <Row key={f.key} label={f.display_name}
            sub={f.enabled ? (f.last ? `Last backup ${f.last}` : 'Waiting for the first backup') : undefined}
            value={f.size ?? (f.item_count != null ? `${f.item_count.toLocaleString('en')} items` : undefined)} mono
            right={<>
              {f.custom ? <Btn variant="ghost" onClick={() => open(`stop:${f.display_name}`)} aria-label={`Remove ${f.display_name}`}>Remove</Btn> : null}
              <Switch label={`Back up ${f.display_name}`} on={f.enabled} onChange={(v) => toggle(f.key, v)} />
            </>} />
        ))}
        <Row label="Add a folder…" onClick={() => open('picker')} />
      </Group>
      <Group title="Other devices">
        {FIXTURE_DEVICES.map((d) => <Row key={d.id} label={d.name} sub={d.status} onClick={() => go('device')} />)}
      </Group>
      <Group title="Beebeeb services">
        {/* Mail Backup's own page (DW-0740) is a proposal the board leaves out: the row says its state and goes nowhere. */}
        <Row label="Mail Backup" value="On" />
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
  const load = useCallback(async () => {
    if (problems) return
    try {
      setList((await extras.syncProblems()).filter((p) => p.backup))
    } catch {
      setNote({ text: actionFailure('load the list', 'failed').text, failed: true })
    }
  }, [extras, problems])
  useEffect(() => {
    void load()
    return source.subscribe((e) => { if (e.kind === 'sync') void load() })
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

export function DevicePane() {
  const { go, open } = useSettings()
  return (
    <Page title="Anna's iPad"
      back={<a href="?section=settings&page=backups" onClick={(e) => { e.preventDefault(); go('backups') }} className="mb-2 inline-block text-[13px] text-ink-2 hover:underline">Back</a>}>
      <Group>
        <Row label="Backed up" value="4,120 photos · 38 GB" mono />
        <Row label="Last backup" value="19 Oct" mono />
        <Row label="Status" value="Signed in" />
      </Group>
      <Group title="See it">
        <Row label="Open in Photos" href="?section=photos" />
        <Row label="Open in Files" href="?section=backups" />
      </Group>
      <Group title="Remove" note="A signed-in device's own backup keeps running.">
        <Row label={<span className="text-[color:var(--color-red-text)]">Delete this backup</span>} onClick={() => open('delete-backup')} />
      </Group>
    </Page>
  )
}

export function StopBackupDialog({ arg }: { arg?: string }) {
  const { close } = useSettings()
  const name = arg || 'Projects'
  return (
    <Dialog labelId="stop-h" title={`Stop backing up ${name}?`} onCancel={close}
      actions={<><Btn onClick={close}>Cancel</Btn><Btn onClick={close}>Stop and move the backup to Trash</Btn><Btn variant="amber" onClick={close}>Stop and keep the backup</Btn></>}>
      <p>Keep the backup and it stays in Backups, read-only. Move it to Trash and it is removed after 30 days.</p>
      <p className="mt-2">Nothing on this PC is touched.</p>
    </Dialog>
  )
}

export function DeleteBackupDialog() {
  const { close } = useSettings()
  const [code, setCode] = useState('')
  const [refused, setRefused] = useState<string | null>(null)
  const ok = /^\d{6}$/.test(code)
  // Blocked: no command deletes another device's backup with a second-factor code yet. The dialog says so and keeps
  // everything; it never closes as if the backup had gone.
  const remove = () => setRefused(actionFailure('delete the backup', 'unsupported').text + ' Nothing was deleted.')
  return (
    <Dialog labelId="del-h" title="Delete this backup?" onCancel={close}
      actions={<><Btn onClick={close}>Cancel</Btn><Btn variant="danger" disabled={!ok} onClick={remove}>Delete backup</Btn></>}>
      {refused ? <p role="alert" className="mb-2 text-[12px] text-red-text">{refused}</p> : null}
      <p>4,120 photos · 38 GB from Anna's iPad move to Trash. Your other devices get an alert.</p>
      <label className="mt-3 block text-[12px] text-ink-3" htmlFor="code">Authenticator code</label>
      <input id="code" inputMode="numeric" maxLength={6} value={code} onChange={(e) => setCode(e.target.value.replace(/\D/g, ''))}
        className={`mt-1 h-8 w-full rounded-md border border-line bg-paper px-2 text-[14px] text-ink ${mono}`} />
    </Dialog>
  )
}

export function FolderPicker() {
  const { close } = useSettings()
  const { add } = useKnownFolders()
  const [sel, setSel] = useState('Projects')
  const dirs = ['Projects', 'Work', 'Archive', 'Scans']
  return (
    <Dialog labelId="pick-h" title="Choose a folder to back up" onCancel={close}
      actions={<><Btn onClick={close}>Cancel</Btn><Btn variant="amber" onClick={() => { add(sel); close() }}>Back up</Btn></>}>
      <ul role="listbox" aria-label="Folders" className="max-h-40 overflow-auto rounded-md border border-line">
        {dirs.map((d) => (
          <li key={d} role="option" aria-selected={sel === d}>
            <button type="button" onClick={() => setSel(d)} className={`w-full px-3 py-1.5 text-left text-[13px] ${sel === d ? 'bg-paper-3 text-ink' : 'text-ink-2 hover:bg-paper-2'}`}>{d}</button>
          </li>
        ))}
      </ul>
      <p className="mt-2 text-[12px] text-ink-3">Beebeeb's own folder in File Explorer can't be chosen, nor a folder inside it.</p>
    </Dialog>
  )
}
