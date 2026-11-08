// The Files dialogs in the board's words (DW-0407, DW-0409, DW-0411, DW-0413, DW-0417, DW-0425, DW-0429, DW-1110).
// Views only: the live section and the frames pass the state.
import { useState, type ReactNode } from 'react'
import { Icon } from '@beebeeb/shared/design'
import { FileIcon, getFileType, type DriveItem, type ItemDetails, type VersionEntry } from '@beebeeb/shared/drive-ui'
import { BTN, BTN_AMBER, FOCUS, Modal } from './ui'
import { fmtCount, fmtDay, fmtSize, fmtWhen, kindOf } from './model'

const NOW = new Date('2026-11-08T12:00:00')

function Select({ label, value, options, onChange }: { label: string; value: string; options: string[]; onChange?: (v: string) => void }) {
  return (
    <select aria-label={label} value={value} onChange={(e) => onChange?.(e.target.value)} className={`h-8 rounded-md border border-line bg-paper px-2 text-[13px] text-ink ${FOCUS}`}>
      {options.map((o) => <option key={o}>{o}</option>)}
    </select>
  )
}

function Row({ k, children }: { k: string; children: ReactNode }) {
  return <div className="flex min-h-11 items-center justify-between gap-4 border-b border-line last:border-0"><span className="text-ink">{k}</span>{children}</div>
}

/** DW-0407: Share. Anyone with the link, expiry, a password, Invite someone, Copy link. */
export function ShareDialog({ name, size, count = 1, canView = 'Can view', expires = 'In 30 days', password = false, onCopy, onInvite, onClose }: {
  name: string; size: number | null; count?: number; canView?: string; expires?: string; password?: boolean
  onCopy?: (o: { canView: string; expires: string; password: string | null }) => void; onInvite?: () => void; onClose?: () => void
}) {
  const [view, setView] = useState(canView)
  const [exp, setExp] = useState(expires)
  const [pwOn, setPwOn] = useState(password)
  const [pw, setPw] = useState('')
  return (
    <Modal
      title="Share" onClose={onClose}
      footer={<><button type="button" className={BTN} onClick={onInvite}>Invite someone…</button><span className="flex-1" /><button type="button" className={BTN} onClick={onClose}>Cancel</button><button type="button" className={BTN_AMBER} onClick={() => onCopy?.({ canView: view, expires: exp, password: pwOn ? pw : null })}>Copy link</button></>}
    >
      <div className="mb-2 flex items-center gap-2.5">
        {count > 1 ? <span className="text-ink">{count} items</span> : <><FileIcon type={getFileType(name, false)} size={24} /><span className="min-w-0 truncate text-ink">{name}</span><span className="font-mono text-[12px] text-ink-3">{fmtSize(size)}</span></>}
      </div>
      <Row k="Anyone with the link"><Select label="Anyone with the link" value={view} options={['Can view', 'Can download']} onChange={setView} /></Row>
      <Row k="Link expires"><Select label="Link expires" value={exp} options={['In 1 day', 'In 7 days', 'In 30 days', 'Never']} onChange={setExp} /></Row>
      <Row k="Password">
        <span className="flex items-center gap-2">
          {pwOn && <input type="password" aria-label="Link password" value={pw} onChange={(e) => setPw(e.target.value)} className={`h-8 w-36 rounded-md border border-line bg-paper px-2 text-[13px] ${FOCUS}`} />}
          <span className="text-ink-3">{pwOn ? 'On' : 'Off'}</span>
          <button type="button" role="switch" aria-checked={pwOn} aria-label="Password" onClick={() => setPwOn(!pwOn)} className={`relative h-5 w-9 rounded-full ${pwOn ? 'bg-ink' : 'border border-line bg-paper-3'} ${FOCUS}`}><span className={`absolute top-0.5 h-4 w-4 rounded-full bg-paper shadow ${pwOn ? 'left-[18px]' : 'left-0.5'}`} /></button>
        </span>
      </Row>
      <p className="m-0 mt-2 text-[12px] leading-4 text-ink-3">The link’s key is in the part after #, which never reaches Beebeeb.</p>
    </Modal>
  )
}

export interface FolderRow { id: string; name: string; count: number }

/** DW-0409: Move to. */
export function MoveDialog({ path, folders, selectedId, targetName, onSelect, onEnter, onCrumb, onNewFolder, onMove, onClose }: {
  path: { id: string | null; name: string }[]; folders: FolderRow[]; selectedId: string | null; targetName: string
  onSelect?: (id: string) => void; onEnter?: (id: string) => void; onCrumb?: (id: string | null) => void; onNewFolder?: () => void; onMove?: () => void; onClose?: () => void
}) {
  return (
    <Modal
      title="Move to" onClose={onClose} width={480}
      footer={<><button type="button" className={BTN} onClick={onNewFolder}>New folder</button><span className="flex-1" /><button type="button" className={BTN} onClick={onClose}>Cancel</button><button type="button" className={BTN_AMBER} onClick={onMove}>Move to {targetName}</button></>}
    >
      <nav aria-label="Path" className="mb-2 flex items-center gap-1.5 text-[13px] text-ink-2">
        {path.map((c, i) => (
          <span key={`${c.id}`} className="flex items-center gap-1.5">
            {i > 0 && <Icon name="chevron-right" size={11} />}
            <a href={`?section=files${c.id ? `&id=${c.id}` : ''}`} onClick={(e) => { e.preventDefault(); onCrumb?.(c.id) }} className={`rounded-sm hover:text-ink ${FOCUS}`}>{c.name}</a>
          </span>
        ))}
      </nav>
      <ul role="listbox" aria-label="Folders" className="m-0 max-h-64 list-none overflow-auto rounded-lg border border-line p-0">
        {folders.map((f) => (
          <li key={f.id} role="option" tabIndex={0} aria-selected={selectedId === f.id} data-move-folder={f.id} onClick={() => onSelect?.(f.id)} onKeyDown={(e) => { if (e.key === 'Enter' && e.ctrlKey) onEnter?.(f.id); else if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); onSelect?.(f.id) } }} onDoubleClick={() => onEnter?.(f.id)} className={`flex h-9 cursor-default items-center gap-2.5 px-3 text-[13px] ${selectedId === f.id ? 'bg-paper-3' : 'hover:bg-paper-2'}`}>
            <FileIcon type="folder" size={20} /><span className="flex-1 truncate text-ink">{f.name}</span><span className="font-mono text-[12px] text-ink-3">{fmtCount(f.count)}</span>
          </li>
        ))}
        {folders.length === 0 && <li className="px-3 py-4 text-center text-ink-3">No folders here.</li>}
      </ul>
    </Modal>
  )
}

/** DW-0417: New folder. The same name rules as Rename (DW-0410b). */
export function NewFolderDialog({ parentName, value, error, onChange, onCreate, onClose }: {
  parentName: string; value: string; error?: string | null; onChange?: (v: string) => void; onCreate?: () => void; onClose?: () => void
}) {
  return (
    <Modal
      title="New folder" onClose={onClose} width={420}
      footer={<><span className="flex-1" /><button type="button" className={BTN} onClick={onClose}>Cancel</button><button type="button" className={BTN_AMBER} onClick={onCreate}>Create</button></>}
    >
      <form onSubmit={(e) => { e.preventDefault(); onCreate?.() }}>
        <input
          aria-label="Folder name" aria-invalid={!!error} value={value} onChange={(e) => onChange?.(e.target.value)} onFocus={(e) => e.currentTarget.select()}
          className={`h-9 w-full rounded-md border bg-paper px-3 text-[13px] text-ink outline-none ${error ? 'border-red' : 'border-line focus:border-ink-3'}`}
        />
        {error ? <p role="alert" className="m-0 mt-2 text-[12px] leading-4 text-red">{error}</p> : <p className="m-0 mt-2 text-[12px] leading-4 text-ink-3">Goes into {parentName}. The name is encrypted too.</p>}
      </form>
    </Modal>
  )
}

/** DW-0411: Details, the iPhone's rows. */
export function DetailsDialog({ item, details, onVersions, onClose }: { item: DriveItem; details: ItemDetails | null; onVersions?: () => void; onClose?: () => void }) {
  const facts: [string, ReactNode][] = [
    ['Size', <span key="s" className="font-mono text-[12px]">{item.isFolder ? fmtCount(item.childCount) : fmtSize(item.size)}</span>],
    ['Changed', <span key="c" className="font-mono text-[12px]">{fmtWhen(item.modifiedAt, NOW)}</span>],
    ['Added', <span key="a" className="font-mono text-[12px]">{details?.changedOn ? fmtDay('2026-10-28T12:00:00') : '—'}</span>],
    ['Where', details ? details.path.map((c) => c.name).join(' › ') : 'Files'],
    ['On this PC', item.local === 'downloaded' || item.local === 'available-offline' ? 'Yes' : 'No'],
    ['Encrypted', <span key="e" className="inline-flex items-center gap-1.5 text-amber-deep"><Icon name="lock" size={12} />Yes, end to end</span>],
  ]
  return (
    <Modal
      title={<span className="flex items-center gap-2.5"><FileIcon type={getFileType(item.name, item.isFolder)} size={26} /><span><span className="block break-all">{item.name}</span><span className="block text-[12px] font-normal text-ink-3">{kindOf(item)}</span></span></span>}
      label="Details" onClose={onClose}
      footer={<><span className="flex-1" /><button type="button" className={BTN} onClick={onClose}>Done</button></>}
    >
      <dl className="m-0">
        {facts.map(([k, v]) => <div key={k} className="flex min-h-9 items-center justify-between gap-4 border-b border-line"><dt className="text-ink-3">{k}</dt><dd className="m-0 text-right text-ink">{v}</dd></div>)}
        {!item.isFolder && (
          <div className="flex min-h-9 items-center justify-between gap-4">
            <dt className="text-ink-3">Version history</dt>
            <dd className="m-0"><button type="button" onClick={onVersions} className={`inline-flex items-center gap-1 rounded-sm text-ink ${FOCUS}`}>{details?.versionCount ?? 0} versions <Icon name="chevron-right" size={11} /></button></dd>
          </div>
        )}
      </dl>
    </Modal>
  )
}

/** DW-0413: Version history. */
export function VersionsDialog({ name, versions, onRestore, onOpen, onClose }: { name: string; versions: VersionEntry[]; onRestore?: (id: string) => void; onOpen?: (id: string) => void; onClose?: () => void }) {
  return (
    <Modal
      title={<span>Version history<span className="block text-[12px] font-normal text-ink-3">{name}, {versions.length} versions</span></span>}
      label="Version history" onClose={onClose}
      footer={<><span className="flex-1" /><button type="button" className={BTN} onClick={onClose}>Done</button></>}
    >
      <ul className="m-0 list-none p-0">
        {versions.map((v) => (
          <li key={v.id} data-version={v.id} title={v.deviceName ? `Changed on ${v.deviceName}` : undefined} onDoubleClick={() => onOpen?.(v.id)} className="flex h-11 items-center gap-3 border-b border-line last:border-0">
            <span className="min-w-0 flex-1 text-ink">{fmtWhen(v.createdAt, NOW).replace('Today, ', 'Today, ')}</span>
            <span className="font-mono text-[12px] text-ink-3">{fmtSize(v.size)}</span>
            {v.current ? <span className="w-28 text-right text-[12px] text-ink-3">Current version</span> : <span className="w-28 text-right"><button type="button" className={`${BTN} h-7 px-3`} onClick={() => onRestore?.(v.id)}>Restore</button></span>}
          </li>
        ))}
      </ul>
      <p className="m-0 mt-3 text-[12px] leading-4 text-ink-3">Earlier versions are kept for 30 days and count toward your storage.</p>
    </Modal>
  )
}

/** DW-0425 and DW-1110: why a protected folder can't change. */
export function ProtectedAlert({ name, inside, onOk, onSettings }: { name: string; inside?: boolean; onOk?: () => void; onSettings?: () => void }) {
  return (
    <Modal
      title="Beebeeb manages this folder" label="Beebeeb manages this folder" width={440} onClose={onOk}
      footer={<><span className="flex-1" /><button type="button" className={BTN} onClick={onOk}>OK</button><button type="button" className={BTN} onClick={onSettings}>Open Settings</button></>}
    >
      {inside
        ? `${name} keeps its files here, so they can’t be moved, renamed or deleted. Turn it on or off in Settings.`
        : `${name} keeps its files here, so it can’t be moved, renamed or deleted. You can turn the service on or off in Settings.`}
    </Modal>
  )
}

export interface ConflictSide { label: string; /** As the Keep button names it: "this PC’s", "Anna’s iPad’s". */ keep: string; when: string; size: number; page?: ReactNode }

/** DW-0429: Two versions. Keep both is the default and sits at the right. */
export function ConflictDialog({ index, total, name, mine, theirs, onKeep, onLater }: {
  index: number; total: number; name: string; mine: ConflictSide; theirs: ConflictSide
  onKeep?: (choice: 'mine' | 'theirs' | 'both') => void; onLater?: () => void
}) {
  const card = (s: ConflictSide) => (
    <button type="button" aria-label={s.label} className={`flex flex-1 flex-col gap-2 rounded-lg border border-line bg-paper p-2 text-left ${FOCUS}`}>
      <div className="grid h-36 place-items-center overflow-hidden rounded-md bg-paper-2">{s.page}</div>
      <div className="px-1 pb-1"><div className="text-[13px] font-medium text-ink">{s.label}</div><div className="font-mono text-[11px] text-ink-3">{s.when} · {fmtSize(s.size)}</div></div>
    </button>
  )
  return (
    <Modal
      title={<span>Two versions <span className="ml-1 font-mono text-[12px] font-normal text-ink-3">{index} of {total}</span></span>}
      label="Two versions" width={560} onClose={onLater}
      footer={<><button type="button" className={BTN} onClick={onLater}>Cancel</button><span className="flex-1" /><button type="button" className={BTN} onClick={() => onKeep?.('mine')}>Keep {mine.keep}</button><button type="button" className={BTN} onClick={() => onKeep?.('theirs')}>Keep {theirs.keep}</button><button type="button" ref={(el) => { if (el) setTimeout(() => el.focus(), 50) }} className={`${BTN} border-ink-3`} onClick={() => onKeep?.('both')}>Keep both</button></>}
    >
      <p className="m-0 mb-3 text-ink"><span className="font-medium">{name}</span> changed in two places at once.</p>
      <div className="flex gap-3">{card(mine)}{card(theirs)}</div>
    </Modal>
  )
}

