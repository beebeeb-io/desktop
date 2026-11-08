// The Files pane: toolbar, selection bar, list or grid, details panel (DW-0401, DW-0403, DW-0420 to DW-0421, DW-1101 to DW-1102).
// Presentational: the live section and every frame pass the same props, so a frame draws the real component.
import { useEffect, useRef, useState, type MouseEvent, type ReactNode } from 'react'
import { Icon } from '@beebeeb/shared/design'
import { FileIcon, getFileType, useDriveSource, type DriveItem, type ItemDetails, type Listing } from '@beebeeb/shared/drive-ui'
import { IconButton, BTN, BTN_AMBER, FOCUS } from './ui'
import {
  fmtCount, fmtSize, fmtWhen, isProtected, kindOf, LOCAL_LABEL, middleShorten, sharedLabel, stemLength,
  type FolderPrefs, type ViewMode,
} from './model'
import type { SortKey } from '@beebeeb/shared/drive-ui'

export const ROW_H = 36

export interface Renaming { id: string; value: string; error?: string | null }
export interface PaneHandlers {
  onSelect?: (id: string, mods: { ctrl: boolean; shift: boolean }) => void
  onClear?: () => void
  onOpen?: (item: DriveItem) => void
  onContext?: (e: MouseEvent, item: DriveItem | null) => void
  onBack?: () => void
  onForward?: () => void
  onCrumb?: (id: string | null) => void
  onView?: (view: ViewMode) => void
  onSortMenu?: (e: MouseEvent) => void
  onSortColumn?: (key: SortKey) => void
  onNewMenu?: (e: MouseEvent) => void
  onTogglePanel?: () => void
  onFind?: () => void
  onRenameChange?: (value: string) => void
  onRenameCommit?: () => void
  onRenameCancel?: () => void
  onToggleOffline?: (item: DriveItem, on: boolean) => void
  onBulk?: (action: 'share' | 'offline' | 'move' | 'trash' | 'done' | 'select-all') => void
  onShare?: (item: DriveItem) => void
  onVersions?: (item: DriveItem) => void
  onDragRows?: (ids: string[]) => void
  onDropOnFolder?: (folder: DriveItem, e: React.DragEvent) => void
  onSettings?: () => void
}

export interface PaneProps {
  listing: Listing | null
  items: DriveItem[]
  prefs: FolderPrefs
  selection: ReadonlySet<string>
  /** The row the details panel follows. */
  currentId?: string | null
  renaming?: Renaming | null
  /** Offline: online-only files are dimmed and say why (DW-1108). */
  offline?: boolean
  dropTarget?: string | null
  /** A drop refused here: the row does not light up (DW-0481). */
  scrollTop?: number
  details?: ItemDetails | null
  /** `backups`: the columns of DW-0422. */
  columns?: 'files' | 'backups'
  canNew?: boolean
  /** Lives above the list: banner, getting-ready strip. */
  top?: ReactNode
  empty?: ReactNode
  /** The window title row text for Backups/other lists. */
  title?: string
  handlers?: PaneHandlers
  /** Frames: show the sort menu button pressed, etc. */
  newPressed?: boolean
  sortPressed?: boolean
  videoLength?: Record<string, string>
  /** Transfers in progress, by item id (DW-0412, DW-0426). */
  transfers?: Record<string, Transfer>
  /** Backups' "Last backup" and "Status" by item id (DW-0422); a row without one shows "—". */
  backupMeta?: Record<string, BackupMeta>
  /** "Now" for the relative times (frames pin the board's clock; live is the real one). */
  now?: Date
}

export interface BackupMeta { last?: string; status: string; thisPc?: boolean }

export interface Transfer { kind: 'uploading' | 'downloading' | 'waiting'; done: number; total: number }

const noopKey = () => {}
const COLS_FILES = 'grid-cols-[minmax(0,1fr)_128px_76px_84px_124px]'
const COLS_BACKUPS = 'grid-cols-[minmax(0,1fr)_128px_76px_150px_124px]'

function Thumb({ item, className = '', fallback = 36 }: { item: DriveItem; className?: string; fallback?: number }) {
  const source = useDriveSource()
  const [url, setUrl] = useState<string | null>(null)
  useEffect(() => {
    let live = true
    if (!item.hasThumbnail) { setUrl(null); return }
    source.thumbnail(item.id, 'small').then((u) => { if (live) setUrl(u) }, () => { if (live) setUrl(null) })
    return () => { live = false }
  }, [source, item.id, item.hasThumbnail])
  if (url) return <img src={url} alt="" data-thumb className={`object-cover ${className}`} />
  return <div className={`grid place-items-center bg-paper-2 ${className}`}><FileIcon type={getFileType(item.name, item.isFolder)} size={fallback} /></div>
}

function SharedCell({ item }: { item: DriveItem }) {
  const s = item.shared
  if (s.kind === 'people') {
    return (
      <span className="flex items-center" title={sharedLabel(s)}>
        {s.people.slice(0, 3).map((p, i) => (
          <span key={p.name} style={{ marginLeft: i ? -6 : 0 }} className="grid h-5 w-5 place-items-center rounded-full border border-paper bg-paper-3 text-[10px] font-medium text-ink-2">{p.initials}</span>
        ))}
      </span>
    )
  }
  if (s.kind === 'link') return <span className="inline-flex items-center gap-1 text-ink-2"><Icon name="link" size={12} />Link</span>
  return <span className="text-ink-3">—</span>
}

function PcCell({ item }: { item: DriveItem }) {
  const l = item.local
  if (!l) return null
  const icon = l === 'available-offline' ? 'pin' : l === 'downloaded' ? 'check' : l === 'online-only' ? 'cloud' : 'activity'
  return <span className="inline-flex items-center gap-1.5 text-ink-2"><Icon name={icon} size={12} />{LOCAL_LABEL[l]}</span>
}

function NameCell({ item, renaming, h, dim }: { item: DriveItem; renaming?: Renaming | null; h: PaneHandlers; dim?: boolean }) {
  const ref = useRef<HTMLInputElement>(null)
  const active = renaming?.id === item.id
  useEffect(() => {
    if (active && ref.current) { ref.current.focus(); ref.current.setSelectionRange(0, stemLength(item)) }
  }, [active]) // eslint-disable-line react-hooks/exhaustive-deps
  return (
    <span role="gridcell" className={`flex min-w-0 items-center gap-2.5 ${dim ? 'opacity-50' : ''}`}>
      {item.hasThumbnail && !item.isFolder
        ? <Thumb item={item} fallback={22} className="h-[22px] w-[22px] flex-none overflow-hidden rounded" />
        : <FileIcon type={getFileType(item.name, item.isFolder)} size={22} />}
      {active ? (
        <span className="min-w-0 flex-1">
          <input
            ref={ref} value={renaming.value} aria-label="Name" aria-invalid={!!renaming.error} aria-describedby={renaming.error ? 'rename-error' : undefined}
            onChange={(e) => h.onRenameChange?.(e.target.value)}
            onClick={(e) => e.stopPropagation()} onDoubleClick={(e) => e.stopPropagation()}
            onKeyDown={(e) => { e.stopPropagation(); if (e.key === 'Enter') h.onRenameCommit?.(); else if (e.key === 'Escape') h.onRenameCancel?.() }}
            onBlur={() => h.onRenameCommit?.()}
            className={`h-7 w-full rounded-md border bg-paper px-2 text-[13px] text-ink outline-none ${renaming.error ? 'border-red' : 'border-ink-3'}`}
          />
          {renaming.error && <span id="rename-error" role="alert" className="mt-1 block text-[12px] leading-4 text-red">{renaming.error}</span>}
        </span>
      ) : (
        <span className="truncate text-ink" title={item.name}>{middleShorten(item.name)}</span>
      )}
    </span>
  )
}

function Row({ item, p, h, selected, index }: { item: DriveItem; p: PaneProps; h: PaneHandlers; selected: boolean; index: number }) {
  const dim = !!p.offline && !item.isFolder && item.local === 'online-only'
  const meta = p.columns === 'backups' ? p.backupMeta?.[item.id] : undefined
  const now = p.now ?? new Date()
  const cols = p.columns === 'backups' ? COLS_BACKUPS : COLS_FILES
  const drop = p.dropTarget === item.id
  const refused = isProtected(item)
  const tr = p.transfers?.[item.id]
  return (
    <div
      role="row" aria-selected={selected} aria-rowindex={index + 2} data-row={item.id} data-selected={selected || undefined} tabIndex={-1}
      draggable={!isProtected(item) && p.renaming?.id !== item.id}
      onDragStart={(e) => { e.dataTransfer.setData('application/x-bb-ids', JSON.stringify(selected ? [...p.selection] : [item.id])); h.onDragRows?.(selected ? [...p.selection] : [item.id]) }}
      onDragOver={(e) => { if (item.isFolder && !refused) e.preventDefault() }}
      onDrop={(e) => { if (item.isFolder && !refused) { e.preventDefault(); h.onDropOnFolder?.(item, e) } }}
      onClick={(e) => { e.stopPropagation(); h.onSelect?.(item.id, { ctrl: e.ctrlKey || e.metaKey, shift: e.shiftKey }) }}
      onDoubleClick={() => h.onOpen?.(item)}
      onKeyDown={(e) => { if (e.key === 'Enter' && !e.altKey) h.onOpen?.(item) }}
      onContextMenu={(e) => { e.preventDefault(); e.stopPropagation(); h.onContext?.(e, item) }}
      style={{ height: ROW_H }}
      className={`relative grid ${cols} cursor-default items-center gap-3 px-4 text-[13px] ${selected ? 'bg-paper-3' : 'hover:bg-paper-2'} ${drop ? 'outline outline-2 -outline-offset-2 outline-ink-3' : ''}`}
    >
      <NameCell item={item} renaming={p.renaming} h={h} dim={dim} />
      {p.columns === 'backups' ? (
        <span role="gridcell" className="font-mono text-[12px] text-ink-3">{meta?.last ? fmtWhen(meta.last, now) : '—'}</span>
      ) : tr ? (
        <span role="gridcell" data-transfer={tr.kind} className="col-span-2 whitespace-nowrap font-mono text-[12px] text-ink-3">
          {tr.kind === 'waiting' ? 'Waiting' : `${tr.kind === 'uploading' ? 'Uploading' : 'Downloading'}, ${(tr.done / 1e6).toFixed(1)} of ${(tr.total / 1e6).toFixed(1)} MB`}
        </span>
      ) : (
        <span role="gridcell" className={`font-mono text-[12px] ${dim ? 'text-ink-2' : 'text-ink-3'}`}>{dim ? <span className="font-sans">Needs a connection</span> : fmtWhen(item.modifiedAt, now)}</span>
      )}
      {!tr && <span role="gridcell" className={`font-mono text-[12px] text-ink-3 ${dim ? 'opacity-50' : ''}`}>{item.isFolder ? fmtCount(item.childCount) : fmtSize(item.size)}</span>}
      <span role="gridcell" className={`text-[12px] ${dim ? 'opacity-50' : ''}`}>
        {p.columns === 'backups'
          ? <span className="inline-flex items-center gap-1.5 text-ink-2">{meta?.status === 'Managed by Beebeeb' && <Icon name="lock" size={12} />}{meta ? meta.status : '—'}{meta?.thisPc && <span className="font-mono text-[11px] text-ink-3">This PC</span>}</span>
          : <SharedCell item={item} />}
      </span>
      <span role="gridcell" className={`text-[12px] ${dim ? 'opacity-50' : ''}`}><PcCell item={item} /></span>
      {tr && tr.kind !== 'waiting' && (
        <span role="progressbar" aria-label={tr.kind === 'uploading' ? 'Uploading' : 'Downloading'} aria-valuenow={Math.round((tr.done / tr.total) * 100)} aria-valuemin={0} aria-valuemax={100} className="absolute inset-x-4 bottom-0.5 h-[3px] overflow-hidden rounded-full bg-paper-3">
          {/* Upload is the encryption step, so its bar is Amber; a download bar stays neutral (DW-0426, DW-0412). */}
          <span className={`block h-full ${tr.kind === 'uploading' ? 'bg-amber' : 'bg-ink-3'}`} style={{ width: `${(tr.done / tr.total) * 100}%` }} />
        </span>
      )}
    </div>
  )
}

function Header({ p, h }: { p: PaneProps; h: PaneHandlers }) {
  const cols = p.columns === 'backups' ? COLS_BACKUPS : COLS_FILES
  const labels: { label: string; key?: SortKey }[] = p.columns === 'backups'
    ? [{ label: 'Name', key: 'name' }, { label: 'Last backup', key: 'modified' }, { label: 'Size', key: 'size' }, { label: 'Status' }, { label: 'PC' }]
    : [{ label: 'Name', key: 'name' }, { label: 'Modified', key: 'modified' }, { label: 'Size', key: 'size' }, { label: 'Shared' }, { label: 'PC' }]
  return (
    <div role="row" aria-rowindex={1} className={`grid ${cols} flex-none items-center gap-3 border-b border-line bg-paper px-4 text-[12px] text-ink-3`} style={{ height: 32 }}>
      {labels.map((c) => (
        <span key={c.label} role="columnheader" aria-sort={c.key && p.prefs.sort.key === c.key ? (p.prefs.sort.dir === 'asc' ? 'ascending' : 'descending') : undefined}>
          {c.key ? (
            <button type="button" onClick={() => h.onSortColumn?.(c.key!)} className={`inline-flex items-center gap-1 rounded-sm font-medium hover:text-ink ${FOCUS}`}>
              {c.label}
              {p.prefs.sort.key === c.key && <Icon name={p.prefs.sort.dir === 'asc' ? 'arrow-up' : 'chevron-down'} size={11} />}
            </button>
          ) : <span className="font-medium">{c.label}</span>}
        </span>
      ))}
    </div>
  )
}

function Grid({ p, h }: { p: PaneProps; h: PaneHandlers }) {
  return (
    <div role="grid" tabIndex={0} aria-label="Files" data-grid onKeyDown={noopKey} className="grid min-h-0 flex-1 content-start gap-3 overflow-auto p-4 [grid-template-columns:repeat(auto-fill,minmax(148px,1fr))]" onClick={() => h.onClear?.()} onContextMenu={(e) => { e.preventDefault(); h.onContext?.(e, null) }}>
      {p.items.map((item) => {
        const selected = p.selection.has(item.id)
        return (
          <div
            key={item.id} role="row" tabIndex={-1} aria-selected={selected} data-tile={item.id} onKeyDown={(e) => { if (e.key === 'Enter' && !e.altKey) h.onOpen?.(item) }} data-selected={selected || undefined}
            onClick={(e) => { e.stopPropagation(); h.onSelect?.(item.id, { ctrl: e.ctrlKey || e.metaKey, shift: e.shiftKey }) }}
            onDoubleClick={() => h.onOpen?.(item)}
            onContextMenu={(e) => { e.preventDefault(); e.stopPropagation(); h.onContext?.(e, item) }}
            className={`flex cursor-default flex-col gap-2 rounded-xl border p-2 ${selected ? 'border-ink-3 bg-paper-3' : 'border-line bg-paper hover:bg-paper-2'}`}
          >
            <div role="gridcell" className="relative">
              {item.isFolder
                ? <div className="grid aspect-[4/3] place-items-center rounded-lg bg-paper-2"><FileIcon type="folder" size={44} /></div>
                : <Thumb item={item} className="aspect-[4/3] w-full rounded-lg" />}
              {p.videoLength?.[item.id] && <span className="absolute bottom-1.5 right-1.5 rounded bg-ink/75 px-1.5 py-0.5 font-mono text-[11px] leading-4 text-paper">{p.videoLength[item.id]}</span>}
            </div>
            <div role="gridcell" className="min-w-0 px-1 pb-1">
              <div className="line-clamp-2 break-words text-[13px] leading-[18px] text-ink" title={item.name}>{item.name}</div>
              <div className="font-mono text-[11px] leading-4 text-ink-3">{item.isFolder ? fmtCount(item.childCount) : fmtSize(item.size)}</div>
            </div>
          </div>
        )
      })}
    </div>
  )
}

function List({ p, h }: { p: PaneProps; h: PaneHandlers }) {
  const ref = useRef<HTMLDivElement>(null)
  const [top, setTop] = useState(p.scrollTop ?? 0)
  const [height, setHeight] = useState(560)
  useEffect(() => {
    const el = ref.current
    if (!el) return
    if (p.scrollTop) el.scrollTop = p.scrollTop
    setHeight(el.clientHeight || 560)
    const ro = new ResizeObserver(() => setHeight(el.clientHeight || 560))
    ro.observe(el)
    return () => ro.disconnect()
  }, [p.scrollTop])
  // One lazy list, no paging (DW-0420b): only the visible rows are drawn.
  const first = Math.max(0, Math.floor(top / ROW_H) - 6)
  const last = Math.min(p.items.length, Math.ceil((top + height) / ROW_H) + 6)
  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key.length !== 1 || e.ctrlKey || e.altKey || e.metaKey || e.key === ' ') return
    const hit = p.items.findIndex((i) => i.name.toLowerCase().startsWith(e.key.toLowerCase()))
    if (hit >= 0 && ref.current) { ref.current.scrollTop = hit * ROW_H; h.onSelect?.(p.items[hit].id, { ctrl: false, shift: false }) }
  }
  return (
    <div role="grid" aria-label="Files" aria-rowcount={p.items.length + 1} className="flex min-h-0 flex-1 flex-col" data-list-count={p.items.length}>
      <Header p={p} h={h} />
      {/* A scroll container is focusable so type-to-select and the arrow keys reach the list. */}
      {/* eslint-disable-next-line jsx-a11y/no-static-element-interactions, jsx-a11y/no-noninteractive-tabindex */}
      <div ref={ref} tabIndex={0}
        onScroll={(e) => setTop(e.currentTarget.scrollTop)} onKeyDown={onKeyDown}
        onClick={() => h.onClear?.()} onContextMenu={(e) => { e.preventDefault(); h.onContext?.(e, null) }}
        className="relative min-h-0 flex-1 overflow-auto focus-visible:outline-none"
      >
        <div style={{ height: p.items.length * ROW_H, position: 'relative' }}>
          <div style={{ position: 'absolute', top: first * ROW_H, left: 0, right: 0 }}>
            {p.items.slice(first, last).map((item, i) => (
              <Row key={item.id} item={item} p={p} h={h} selected={p.selection.has(item.id)} index={first + i} />
            ))}
          </div>
        </div>
      </div>
    </div>
  )
}

export function Crumbs({ listing, h, section = 'files' }: { listing: Listing | null; h: PaneHandlers; section?: 'files' | 'backups' }) {
  const path = listing?.path ?? [{ id: null, name: 'Files' }]
  return (
    <nav aria-label="Path" className="flex min-w-0 items-center gap-1.5 text-[15px] font-semibold text-ink">
      {path.map((c, i) => (
        <span key={`${c.id}`} className="flex min-w-0 items-center gap-1.5">
          {i > 0 && <Icon name="chevron-right" size={12} />}
          {i < path.length - 1
            ? <a href={`?section=${section}${c.id ? `&id=${c.id}` : ''}`} onClick={(e) => { e.preventDefault(); h.onCrumb?.(c.id) }} className={`rounded-sm font-normal text-ink-2 hover:text-ink ${FOCUS}`}>{c.name}</a>
            : <h1 className="m-0 truncate text-[15px] font-semibold">{c.name}</h1>}
        </span>
      ))}
    </nav>
  )
}

export function Toolbar({ p, h }: { p: PaneProps; h: PaneHandlers }) {
  const canBack = (p.listing?.path.length ?? 1) > 1
  return (
    <div className="flex h-12 flex-none items-center gap-2 border-b border-line px-3">
      <IconButton icon="chevron-right" flip label="Back (Alt+Left)" onClick={h.onBack} disabled={!canBack} />
      <IconButton icon="chevron-right" label="Forward (Alt+Right)" onClick={h.onForward} />
      <div className="ml-1 flex min-w-0 items-baseline gap-3">
        <Crumbs listing={p.listing} h={h} section={p.title === 'Backups' ? 'backups' : 'files'} />
        <span className="flex-none font-mono text-[12px] text-ink-3" data-count>{fmtCount(p.items.length)}</span>
      </div>
      <span className="flex-1" />
      <button type="button" onClick={h.onFind} className={`inline-flex h-8 items-center gap-2 rounded-md border border-line bg-paper px-3 text-[13px] text-ink-3 hover:bg-paper-2 ${FOCUS}`}>
        <Icon name="search" size={14} />Search names <span className="font-mono text-[11px]">Ctrl+F</span>
      </button>
      <div role="group" aria-label="View" className="flex rounded-md border border-line">
        <IconButton icon="menu" label="as List" pressed={p.prefs.view === 'list'} onClick={() => h.onView?.('list')} />
        <IconButton icon="columns" label="as Grid" pressed={p.prefs.view === 'grid'} onClick={() => h.onView?.('grid')} />
      </div>
      <button type="button" data-anchor="sort" aria-haspopup="menu" aria-pressed={p.sortPressed} onClick={h.onSortMenu} className={`inline-flex h-8 items-center gap-1.5 rounded-md px-2.5 text-[13px] text-ink-2 hover:bg-paper-2 ${p.sortPressed ? 'bg-paper-3' : ''} ${FOCUS}`}>
        Sort <Icon name="chevron-down" size={12} />
      </button>
      <IconButton icon="info" label="Details panel (Alt+Shift+P)" pressed={p.prefs.panel} onClick={h.onTogglePanel} />
      <button type="button" data-anchor="new" aria-haspopup="menu" disabled={p.canNew === false} title={p.canNew === false ? 'Nothing can be added here' : 'New'} onClick={h.onNewMenu} className={`${BTN_AMBER} gap-1.5 disabled:opacity-40`}>
        <Icon name="plus" size={14} />New
      </button>
    </div>
  )
}

export function SelectionBar({ p, h }: { p: PaneProps; h: PaneHandlers }) {
  const picked = p.items.filter((i) => p.selection.has(i.id))
  const blocked = picked.some(isProtected)
  return (
    <div role="toolbar" aria-label="Selection" data-selection-bar className="flex h-11 flex-none items-center gap-2 border-b border-line bg-paper-2 px-4 text-[13px]">
      <span className="font-medium text-ink">{picked.length} selected</span>
      <button type="button" onClick={() => h.onBulk?.('select-all')} className={`rounded-md px-2 py-1 text-ink-2 underline-offset-2 hover:bg-paper-3 ${FOCUS}`}>Select all</button>
      <span className="flex-1" />
      <button type="button" className={BTN} onClick={() => h.onBulk?.('share')}>Share…</button>
      <button type="button" className={BTN} onClick={() => h.onBulk?.('offline')}>Make available offline</button>
      <button type="button" className={BTN} disabled={blocked} title={blocked ? 'Backups is protected, so it stays out of a bulk action.' : undefined} onClick={() => h.onBulk?.('move')}>Move…</button>
      <button type="button" className={BTN} disabled={blocked} title={blocked ? 'Backups is protected, so it stays out of a bulk action.' : undefined} onClick={() => h.onBulk?.('trash')}>Move to Trash <span className="ml-2 font-mono text-[11px] text-ink-3">Delete</span></button>
      <button type="button" className={BTN} onClick={() => h.onBulk?.('done')}>Done</button>
    </div>
  )
}

function Switch({ on, label, onChange }: { on: boolean; label: string; onChange?: (on: boolean) => void }) {
  return (
    <button type="button" role="switch" aria-checked={on} aria-label={label} onClick={() => onChange?.(!on)} className={`relative h-5 w-9 flex-none rounded-full ${on ? 'bg-ink' : 'bg-paper-3 border border-line'} ${FOCUS}`}>
      <span className={`absolute top-0.5 h-4 w-4 rounded-full bg-paper shadow ${on ? 'left-[18px]' : 'left-0.5'}`} />
    </button>
  )
}

function Fact({ k, v, mono }: { k: string; v: ReactNode; mono?: boolean }) {
  return (
    <div className="flex items-baseline justify-between gap-3 py-1.5 text-[13px]">
      <dt className="flex-none text-ink-3">{k}</dt>
      <dd className={`m-0 min-w-0 truncate text-right text-ink ${mono ? 'font-mono text-[12px]' : ''}`}>{v}</dd>
    </div>
  )
}

export function EncryptedMark() {
  return <span className="inline-flex items-center gap-1.5 text-[12px] font-medium text-amber-deep"><Icon name="lock" size={12} />End-to-end encrypted</span>
}

export function DetailsPanel({ p, h }: { p: PaneProps; h: PaneHandlers }) {
  const picked = p.items.filter((i) => p.selection.has(i.id))
  const now = new Date('2026-11-08T12:00:00')
  if (picked.length === 0) {
    return <aside aria-label="Details" className="flex w-[320px] flex-none flex-col items-center justify-center border-l border-line bg-paper px-8 text-center text-[13px] text-ink-3">Select a file to see its details.</aside>
  }
  if (picked.length > 1) {
    const total = picked.reduce((n, i) => n + (i.size ?? 0), 0)
    return (
      <aside aria-label="Details" data-details="multi" className="flex w-[320px] flex-none flex-col gap-3 border-l border-line bg-paper p-4">
        <div className="text-[15px] font-semibold text-ink">{picked.length} items</div>
        <div className="font-mono text-[12px] text-ink-3">{picked.length} items · {fmtSize(total)}</div>
        <EncryptedMark />
      </aside>
    )
  }
  const item = picked[0]
  const d = p.details?.item.id === item.id ? p.details : null
  const photo = getFileType(item.name, false) === 'image' && !item.isFolder
  return (
    <aside aria-label="Details" data-details="one" className="flex w-[320px] flex-none flex-col overflow-auto border-l border-line bg-paper">
      <div className="p-4 pb-3">
        {item.isFolder
          ? <div className="grid aspect-[4/3] place-items-center rounded-lg bg-paper-2"><FileIcon type="folder" size={56} /></div>
          : <Thumb item={item} className="aspect-[4/3] w-full rounded-lg" />}
        <h2 className="m-0 mt-3 break-words text-[15px] font-semibold leading-5 text-ink">{item.name}</h2>
        <div className="mt-0.5 text-[12px] text-ink-3">{kindOf(item)} · <span className="font-mono">{item.isFolder ? fmtCount(item.childCount) : fmtSize(item.size)}</span></div>
        <div className="mt-2"><EncryptedMark /></div>
      </div>
      <dl className="m-0 border-t border-line px-4 py-2">
        <Fact k="Modified" v={fmtWhen(item.modifiedAt, now)} mono />
        <Fact k={photo ? 'Taken on' : 'Changed on'} v={d?.changedOn ?? '—'} />
        <Fact k="Where" v={d ? d.path.map((c) => c.name).join(' › ') : 'Files'} />
        <Fact k="Versions" v={d ? (item.isFolder ? '—' : `${d.versionCount}`) : '—'} mono />
        <Fact k="Shared" v={item.shared.kind === 'none' ? 'No' : sharedLabel(item.shared)} />
      </dl>
      {!item.isFolder && (
        <div className="flex items-center justify-between border-t border-line px-4 py-3 text-[13px] text-ink">
          Available offline
          <Switch on={item.local === 'available-offline'} label="Available offline" onChange={(on) => h.onToggleOffline?.(item, on)} />
        </div>
      )}
      <div className="mt-auto flex gap-2 border-t border-line p-4">
        <button type="button" className={`${BTN} flex-1`} onClick={() => h.onShare?.(item)}>Share…</button>
        {!item.isFolder && <button type="button" className={`${BTN} flex-1`} onClick={() => h.onVersions?.(item)}>Versions</button>}
      </div>
    </aside>
  )
}

export function FilesPane({ p }: { p: PaneProps }) {
  const h = p.handlers ?? {}
  const showSelectionBar = p.selection.size > 1
  return (
    <div className="flex min-h-0 min-w-0 flex-1">
      <section aria-label={p.title ?? 'Files'} className="flex min-w-0 flex-1 flex-col">
        <Toolbar p={p} h={h} />
        {showSelectionBar && <SelectionBar p={p} h={h} />}
        {p.top}
        {p.items.length === 0 && p.empty ? p.empty : p.prefs.view === 'grid' ? <Grid p={p} h={h} /> : <List p={p} h={h} />}
      </section>
      {p.prefs.panel && <DetailsPanel p={p} h={h} />}
    </div>
  )
}

