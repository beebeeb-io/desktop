// Photos' dialogs: Photo sources (DW-0604), Share (DW-0607), Details (DW-0609).
import { useState } from 'react'
import { BTN, BTN_AMBER, FOCUS, Modal } from '../files/ui'
import { fmtN, mb1, takenLabel, type PhotoCell } from './model'

export interface SourceChoice { name: string; photos: number; on: boolean }
export interface FolderChoice { name: string; items: number; note?: string; on: boolean }

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
