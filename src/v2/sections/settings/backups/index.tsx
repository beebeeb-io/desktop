// Drive › Backups on Windows (DW-0737, DW-1411, DW-0738, DW-1410, DW-0739, DW-0742, DW-0741).
// Folder state comes from the existing engine commands (get/set_known_folder_backup).
import { useState } from 'react'
import { Btn, Dialog, Group, Meter, Page, Row, Switch, mono } from '../shell/kit'
import { FIXTURE_DEVICES, useKnownFolders } from '../shell/data'
import { useSettings } from '../shell/context'

export type BackupState = 'idle' | 'running'

export const FAILURES = [
  { id: 'a', name: 'Render final.mov', folder: 'Documents', why: 'the file is open in another app' },
  { id: 'b', name: 'Invoice scan.pdf', folder: 'Desktop', why: 'this file is damaged' },
  { id: 'c', name: 'Budget 2025.xlsx', folder: 'Documents', why: 'it kept changing while it was backing up' },
]

function StatusCard({ state }: { state: BackupState }) {
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
        <a href="?section=settings&sub=backup-failures" onClick={(e) => { e.preventDefault(); go('backup-failures') }} className="text-[12px] text-ink-2 underline">Couldn't back up 3 items</a>
      </div>
      <Btn>Back up now</Btn>
    </div>
  )
}

export function Backups({ state = 'idle' }: { state?: BackupState } = {}) {
  const { go, open } = useSettings()
  const { folders, toggle } = useKnownFolders()
  return (
    <Page title="Backups">
      <StatusCard state={state} />
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
        <Row label="Mail Backup" value="On" href="?section=settings&sub=mail-backup" />
      </Group>
    </Page>
  )
}

export function BackupFailures() {
  const { go } = useSettings()
  const [list, setList] = useState(FAILURES)
  return (
    <Page title="Couldn't back up"
      back={<a href="?section=settings&sub=backups" onClick={(e) => { e.preventDefault(); go('backups') }} className="mb-2 inline-block text-[13px] text-ink-2 hover:underline">Back</a>}
      action={list.length ? <Btn onClick={() => setList([])}>Try all again</Btn> : undefined}>
      <p className="text-[13px] text-ink-2">Beebeeb stopped trying these after several tries. Everything else is backed up.</p>
      <Group>
        {list.map((f) => (
          <Row key={f.id} label={f.name} sub={`${f.folder}: ${f.why}`} right={<Btn onClick={() => setList((l) => l.filter((x) => x.id !== f.id))}>Try again</Btn>} />
        ))}
      </Group>
      <Group title="Not backed up, by design">
        <Row label="Site walkthrough.mkv" sub="Desktop: files larger than 4 GB are not backed up from this PC" />
      </Group>
    </Page>
  )
}

export function DevicePane() {
  const { go, open } = useSettings()
  return (
    <Page title="Anna's iPad"
      back={<a href="?section=settings&sub=backups" onClick={(e) => { e.preventDefault(); go('backups') }} className="mb-2 inline-block text-[13px] text-ink-2 hover:underline">Back</a>}>
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
  const ok = /^\d{6}$/.test(code)
  return (
    <Dialog labelId="del-h" title="Delete this backup?" onCancel={close}
      actions={<><Btn onClick={close}>Cancel</Btn><Btn variant="danger" disabled={!ok} onClick={close}>Delete backup</Btn></>}>
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
