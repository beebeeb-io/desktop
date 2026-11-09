// Photos' dialogs: Photo sources (DW-0604), Share (DW-0607), Details (DW-0609), and the viewer's ⋯ Move… (DW-0409)
// and Rename.
import { useEffect, useState } from 'react'
import { DriveSourceError, useDriveSource, type DriveItem, type Listing } from '@beebeeb/shared/drive-ui'
import { BTN, BTN_AMBER, FOCUS, Modal } from '../files/ui'
import { MoveDialog, type FolderRow } from '../files/Dialogs'
import { nameProblem, nameTaken, takenMessage } from '../files/model'
import { showToast } from '../../chrome/Toast'
import { fmtN, mb1, takenLabel, type PhotoCell } from './model'

export interface SourceChoice { name: string; photos: number; on: boolean }
export interface FolderChoice { id?: string; name: string; items: number; note?: string; on: boolean }

function Switch({ on, label, onChange }: { on: boolean; label: string; onChange: (on: boolean) => void }) {
  return (
    <button type="button" role="switch" aria-checked={on} aria-label={label} onClick={() => onChange(!on)}
      className={`relative h-5 w-9 flex-none rounded-full ${on ? 'bg-ink' : 'bg-paper-3 border border-line'} ${FOCUS}`}>
      <span className={`absolute top-0.5 h-4 w-4 rounded-full bg-paper transition-all ${on ? 'left-[18px]' : 'left-0.5'}`} />
    </button>
  )
}

export function SourcesDialog({ devices, folders, onAddFolder, onCancel, onDone }: {
  devices: SourceChoice[]; folders: FolderChoice[]; onAddFolder?: () => void; onCancel?: () => void; onDone?: (d: SourceChoice[], f: FolderChoice[]) => void
}) {
  const [d, setD] = useState(devices)
  const [f, setF] = useState(folders)
  return (
    <Modal title="Photo sources" width={500} onClose={onCancel}
      footer={<><span className="flex-1" /><button type="button" className={BTN} onClick={onCancel}>Cancel</button><button type="button" className={BTN} onClick={() => onDone?.(d, f)}>Done</button></>}>
      <h3 className="m-0 mb-1 text-[12px] font-medium uppercase tracking-wide text-ink-3">Backups from your devices</h3>
      {d.length === 0 && <p className="m-0 mb-4 text-[13px] text-ink-3">No device has backed up photos yet.</p>}
      <ul className="m-0 mb-4 list-none p-0">
        {d.map((x, i) => (
          <li key={x.name} data-source={x.name} className="flex h-10 items-center gap-3 border-b border-line">
            <span className="min-w-0 flex-1 truncate text-ink">{x.name}</span>
            <span className="font-mono text-[12px] text-ink-3">{fmtN(x.photos)} photos</span>
            <Switch on={x.on} label={x.name} onChange={(on) => setD(d.map((y, j) => (j === i ? { ...y, on } : y)))} />
          </li>
        ))}
      </ul>
      <h3 className="m-0 mb-1 text-[12px] font-medium uppercase tracking-wide text-ink-3">Folders from Files</h3>
      {f.length === 0 && <p className="m-0 mb-2 text-[13px] text-ink-3">No folder from Files is shown yet.</p>}
      <ul className="m-0 mb-2 list-none p-0">
        {f.map((x, i) => (
          <li key={x.name} data-source={x.name} className="flex h-10 items-center gap-3 border-b border-line">
            <span className="min-w-0 flex-1 truncate text-ink">{x.name}</span>
            <span className="font-mono text-[12px] text-ink-3">{x.items} items{x.note ? `, ${x.note}` : ''}</span>
            <Switch on={x.on} label={x.name} onChange={(on) => setF(f.map((y, j) => (j === i ? { ...y, on } : y)))} />
          </li>
        ))}
      </ul>
      <button type="button" className={BTN} onClick={onAddFolder}>Add a folder…</button>
      <p className="m-0 mt-4 text-[12px] leading-5 text-ink-3">This only changes what Photos shows. Nothing is moved or deleted, and backups keep running.</p>
    </Modal>
  )
}

export function ShareDialog({ cell, onCopyLink, onSendCopy, onClose }: { cell: PhotoCell; onCopyLink?: () => void; onSendCopy?: () => void; onClose?: () => void }) {
  const row = (k: string, v: string) => (
    <div className="flex h-9 items-center justify-between border-b border-line text-[13px]"><span className="text-ink-2">{k}</span><span className="text-ink">{v}</span></div>
  )
  return (
    <Modal title="Share" width={460} onClose={onClose}
      footer={<><button type="button" className={BTN} onClick={onSendCopy}>Send a copy…</button><span className="flex-1" /><button type="button" className={BTN_AMBER} onClick={onCopyLink}>Copy link</button></>}>
      <p className="m-0 mb-2 text-ink">{cell.entry.item.name} <span className="font-mono text-[12px] text-ink-3">· {mb1(cell.entry.item.size ?? 0)}</span></p>
      {row('Anyone with the link', 'Can view')}
      {row('Link expires', 'In 30 days')}
      {row('Password', 'Off')}
      <p className="m-0 mt-3 text-[12px] leading-5 text-ink-3">Copy link keeps the photo encrypted: the key stays in the link. Send a copy decrypts it and opens the Windows Share menu, so the copy leaves Beebeeb’s protection.</p>
    </Modal>
  )
}

export function DetailsDialog({ cell, now, onClose }: { cell: PhotoCell; now: Date; onClose?: () => void }) {
  const m = cell.meta
  const rows: [string, string][] = [
    ['Taken', takenLabel({ ...cell, meta: undefined }, now).replace(' ', ', ')],
    ...(m?.device ? [['From', m.device] as [string, string]] : []),
    ...(m?.camera ? [['Camera', m.camera] as [string, string]] : []),
    ['Size', `${m?.dims ? `${m.dims}, ` : ''}${mb1(cell.entry.item.size ?? 0)}`],
    ...(m?.previewBytes ? [['Preview', `4K HEIC, ${(m.previewBytes / 1e6).toFixed(1)} MB`] as [string, string]] : []),
    ...(m?.device ? [['Where', `Backups / ${m.device}`] as [string, string]] : []),
    ['Version history', '1 version'],
  ]
  return (
    <Modal title="Details" width={440} onClose={onClose} footer={<><span className="flex-1" /><button type="button" className={BTN} onClick={onClose}>Done</button></>}>
      <dl className="m-0">
        {rows.map(([k, v]) => (
          <div key={k} className="flex min-h-9 items-center gap-4 border-b border-line py-1.5 text-[13px]"><dt className="w-32 flex-none text-ink-3">{k}</dt><dd className="m-0 text-ink">{v}</dd></div>
        ))}
      </dl>
      <p className="m-0 mt-3 text-[12px] text-ink-3">Camera and size come from the photo’s own data, read on this PC.</p>
    </Modal>
  )
}

/** Why a write failed, in the board's words: never the host's text. */
export function failedWords(e: unknown, what: string): string {
  const code = e instanceof DriveSourceError ? e.code : 'failed'
  if (code === 'unsupported') return `Couldn’t ${what}. This isn’t available in this version of Beebeeb.`
  if (code === 'offline') return `Couldn’t ${what}. Check your connection.`
  if (code === 'read-only') return `Couldn’t ${what}. It’s read-only.`
  return `Couldn’t ${what}. Try again.`
}

/** The viewer's ⋯ › Move… (DW-0606 → DW-0409) and the selection bar's Move… (DW-0602): Files' Move to dialog. */
export function PhotoMove({ items, onDone }: { items: DriveItem[]; onDone: (moved: boolean) => void }) {
  const source = useDriveSource()
  const [at, setAt] = useState<string | null>(null)
  const [target, setTarget] = useState<string | null>(null)
  const [listing, setListing] = useState<Listing | null>(null)
  useEffect(() => { source.list(at).then(setListing, () => setListing(null)) }, [source, at])
  const folders: FolderRow[] = (listing?.items ?? []).filter((i) => i.isFolder && !i.readOnly).map((i) => ({ id: i.id, name: i.name, count: i.childCount ?? 0 }))
  const chosen = target ? folders.find((f) => f.id === target) : undefined
  const name = chosen?.name ?? listing?.folder.name ?? 'Files'
  const move = () => {
    source.move(items.map((i) => i.id), target ?? at).then(
      () => { showToast({ kind: 'done', text: `Moved to ${name}` }); onDone(true) },
      (e: unknown) => { showToast({ kind: 'failed', text: failedWords(e, 'move it') }); onDone(false) },
    )
  }
  return (
    <MoveDialog
      path={listing?.path ?? [{ id: null, name: 'Files' }]} folders={folders} selectedId={target} targetName={name}
      onSelect={setTarget} onEnter={(id) => { setAt(id); setTarget(null) }} onCrumb={(id) => { setAt(id); setTarget(null) }}
      onNewFolder={() => showToast({ kind: 'failed', text: `Make the folder in Files first, then move ${items.length === 1 ? 'the photo' : 'the photos'} there.` })}
      onMove={move} onClose={() => onDone(false)}
    />
  )
}

/** Focus once, when the field mounts (a stable callback, so a re-render does not run it again). */
const focusSoon = (el: HTMLInputElement | null) => { if (el) setTimeout(() => el.focus(), 0) }

/** The viewer's ⋯ › Rename: the same name rules as Files' rename (DW-0410). */
export function RenameDialog({ item, siblings, onDone }: { item: DriveItem; siblings: readonly DriveItem[]; onDone: (renamed: boolean) => void }) {
  const source = useDriveSource()
  const [value, setValue] = useState(item.name)
  const [error, setError] = useState<string | null>(null)
  const save = () => {
    const name = value.trim()
    if (name === item.name) return onDone(false)
    const problem = nameProblem(name) ?? (nameTaken(name, siblings, item.id) ? takenMessage(name, false) : null)
    if (problem) return setError(problem)
    source.rename(item.id, name).then(
      () => onDone(true),
      (e: unknown) => {
        if (e instanceof DriveSourceError && e.code === 'name-taken') setError(takenMessage(name, false))
        else { showToast({ kind: 'failed', text: failedWords(e, 'rename it') }); onDone(false) }
      },
    )
  }
  return (
    <Modal title="Rename" width={420} onClose={() => onDone(false)}
      footer={<><span className="flex-1" /><button type="button" className={BTN} onClick={() => onDone(false)}>Cancel</button><button type="button" className={BTN_AMBER} onClick={save}>Rename</button></>}>
      <form onSubmit={(e) => { e.preventDefault(); save() }}>
        <input
          aria-label="Name" aria-invalid={!!error} value={value} onChange={(e) => { setValue(e.target.value); setError(null) }}
          onFocus={(e) => { const dot = e.currentTarget.value.lastIndexOf('.'); e.currentTarget.setSelectionRange(0, dot > 0 ? dot : e.currentTarget.value.length) }}
          ref={focusSoon}
          className={`h-9 w-full rounded-md border bg-paper px-3 text-[13px] text-ink outline-none ${error ? 'border-red' : 'border-line focus:border-ink-3'}`}
        />
        {error && <p role="alert" className="m-0 mt-2 text-[12px] leading-4 text-red">{error}</p>}
      </form>
    </Modal>
  )
}
