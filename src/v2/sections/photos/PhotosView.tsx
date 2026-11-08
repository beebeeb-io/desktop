// Photos (DW-0601, DW-0602, DW-0603, DW-0116): the grid by day, the source pills, the selection bar. Presentational:
// the section (index.tsx) and the frames (frames/b06.tsx) hand it state; it reads thumbnails through DriveSource only.
import { useEffect, useState, type ReactNode } from 'react'
import { Icon } from '@beebeeb/shared/design'
import { PhotoTileSkeleton } from '@beebeeb/shared/design'
import { useDriveSource } from '@beebeeb/shared/drive-ui'
import { BTN, BTN_AMBER, FOCUS } from '../files/ui'
import { ACROSS, ACROSS_LABEL, dayHeading, fmtMB, fmtN, groupByDay, idOf, isVideo, type Across, type PhotoCell } from './model'

/** The decrypted thumbnail URL for a photo, from the source; null until it arrives (or when there is none). */
export function useThumb(id: string, size: 'small' | 'large'): string | null {
  const source = useDriveSource()
  const [url, setUrl] = useState<string | null>(null)
  useEffect(() => {
    let live = true
    source.thumbnail(id, size).then((u) => { if (live) setUrl(u) }, () => { if (live) setUrl(null) })
    return () => { live = false }
  }, [source, id, size])
  return url
}

export interface BackupLine { name: string; status: string }
export interface Totals { photos: number; videos: number; bytes: number }

export interface PhotosViewProps {
  cells: PhotoCell[]
  now: Date
  across: Across
  /** Device names for the pills. Empty: the source cannot say, so no pills (a gap, never invented). */
  devices: string[]
  device: string | null
  totals: Totals | null
  backups: BackupLine[]
  selecting: boolean
  selected: ReadonlySet<string>
  loading?: boolean
  /** Frame hook: draw the Photos across menu open. */
  acrossOpen?: boolean
  canUpload?: boolean
  /** A place name per day ("2026-10-12": "Lisbon"), when the source knows one. */
  places?: Record<string, string>
  onDevice?: (d: string | null) => void
  onOpen?: (id: string) => void
  onToggle?: (id: string, on?: boolean) => void
  onAcross?: (n: Across) => void
  onSources?: () => void
  onSelect?: () => void
  onSelectAll?: () => void
  onDone?: () => void
  onUpload?: () => void
  onShare?: () => void
  onOffline?: () => void
  onMove?: () => void
  onTrash?: () => void
}

const GRID: Record<Across, string> = {
  3: 'grid-cols-3', 4: 'grid-cols-4', 5: 'grid-cols-5', 7: 'grid-cols-7',
}

function Tile({ cell, selected, selecting, onOpen, onToggle }: { cell: PhotoCell; selected: boolean; selecting: boolean; onOpen?: (id: string) => void; onToggle?: (id: string, on?: boolean) => void }) {
  const id = idOf(cell)
  const url = useThumb(id, 'small')
  const name = cell.entry.item.name
  return (
    <button
      type="button" data-tile={id} data-selected={selected || undefined} aria-label={name} aria-pressed={selecting ? selected : undefined}
      onClick={(e) => { if (selecting || e.ctrlKey || e.metaKey) onToggle?.(id); else onOpen?.(id) }}
      onDoubleClick={() => { if (!selecting) onOpen?.(id) }}
      onKeyDown={(e) => { if (e.key === ' ' && !selecting) { e.preventDefault(); onOpen?.(id) } }}
      onPointerEnter={(e) => { if (e.buttons === 1) onToggle?.(id, true) }}
      className={`relative aspect-square w-full select-none overflow-hidden rounded-md bg-paper-2 ${selected ? 'outline outline-2 outline-offset-[-2px] outline-ink' : ''} ${FOCUS}`}
    >
      {url ? <img src={url} alt="" draggable={false} className="h-full w-full object-cover" /> : <PhotoTileSkeleton />}
      {isVideo(cell) && (
        <span className="absolute bottom-1 left-1 flex items-center gap-1 rounded bg-ink/70 px-1.5 py-0.5 font-mono text-[11px] text-paper">
          <Icon name="play" size={10} />{cell.meta?.duration}
        </span>
      )}
      {cell.meta?.uploading && (
        <span title="Uploading" className="absolute right-1 top-1 grid h-5 w-5 place-items-center rounded-full bg-ink/70 text-paper"><Icon name="arrow-up" size={11} /></span>
      )}
      {selected && <span className="absolute left-1 top-1 grid h-5 w-5 place-items-center rounded-full bg-ink text-paper"><Icon name="check" size={12} /></span>}
    </button>
  )
}

function Pill({ label, on, onClick }: { label: string; on: boolean; onClick?: () => void }) {
  return (
    <button type="button" aria-pressed={on} data-pill={label} onClick={onClick}
      className={`h-7 flex-none whitespace-nowrap rounded-full border px-3 text-[12px] ${on ? 'border-ink bg-ink text-paper' : 'border-line text-ink-2 hover:bg-paper-2'} ${FOCUS}`}>
      {label}
    </button>
  )
}

export function Toolbar({ children, subtitle }: { children?: ReactNode; subtitle?: string }) {
  return (
    <div className="flex h-12 flex-none items-center gap-3 border-b border-line px-4">
      <h1 className="m-0 flex-none text-[15px] font-semibold text-ink">Photos</h1>
      {subtitle && <span className="min-w-0 truncate font-mono text-[12px] text-ink-3" data-totals>{subtitle}</span>}
      <span className="flex-1" />
      {children}
    </div>
  )
}

export function EmptyPhotos({ onSources }: { onSources?: () => void }) {
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-3 px-8 text-center" data-empty="photos">
      <div className="grid h-14 w-14 place-items-center rounded-2xl border border-line bg-paper-2 text-ink-3"><Icon name="image" size={26} /></div>
      <h2 className="m-0 text-[15px] font-semibold text-ink">No photos yet</h2>
      <p className="m-0 max-w-sm text-[13px] leading-5 text-ink-3">Photos backed up from your iPhone or iPad appear here. Turn on photo backup in Beebeeb on the iPhone or iPad.</p>
      <button type="button" className={BTN} onClick={onSources}>Photo sources</button>
    </div>
  )
}

export function PhotosView(p: PhotosViewProps) {
  const [menu, setMenu] = useState(p.acrossOpen ?? false)
  const visible = p.device ? p.cells.filter((c) => c.meta?.device === p.device) : p.cells
  const days = groupByDay(visible)
  const picked = visible.filter((c) => p.selected.has(idOf(c)))
  const pickedBytes = picked.reduce((n, c) => n + (c.entry.item.size ?? 0), 0)
  const totals = p.totals
  const subtitle = totals ? `${fmtN(totals.photos)} photos · ${fmtN(totals.videos)} videos · ${fmtMB(totals.bytes)}` : undefined
  const backing = p.device ? p.backups.filter((b) => b.name === p.device) : p.backups.filter((b) => b.status.includes('backing up'))
  const hasPills = p.devices.length > 0

  return (
    <div className="flex min-h-0 flex-1 flex-col" data-section="photos">
      <Toolbar subtitle={subtitle}>
        <div className="relative">
          <button type="button" className={BTN} aria-haspopup="menu" aria-expanded={menu} data-anchor="photos-across" onClick={() => setMenu((m) => !m)}>
            Photos across
          </button>
          {menu && (
            <div role="menu" aria-label="Photos across" className="absolute right-0 top-9 z-30 w-[264px] rounded-lg border border-line bg-paper p-1 shadow-xl" data-menu="Photos across">
              {ACROSS.map((n) => (
                <button key={n} type="button" role="menuitemradio" aria-checked={n === p.across} onClick={() => { p.onAcross?.(n); setMenu(false) }}
                  className={`flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-[13px] text-ink hover:bg-paper-2 ${FOCUS}`}>
                  <span className="grid w-4 place-items-center">{n === p.across && <Icon name="check" size={13} />}</span>
                  <span className="flex-1">{ACROSS_LABEL[n]}</span>
                  {n === 3 && <span className="font-mono text-[11px] text-ink-3">Ctrl+Plus</span>}
                  {n === 7 && <span className="font-mono text-[11px] text-ink-3">Ctrl+−</span>}
                </button>
              ))}
            </div>
          )}
        </div>
        <button type="button" className={BTN} onClick={p.onSources}>Photo sources</button>
        {p.canUpload && <button type="button" className={BTN_AMBER} onClick={p.onUpload}>Upload</button>}
        <button type="button" className={BTN} aria-pressed={p.selecting} onClick={p.onSelect}>Select</button>
      </Toolbar>

      {p.selecting && (
        <div role="toolbar" aria-label="Selection" data-selection-bar className="flex h-11 flex-none items-center gap-2 border-b border-line bg-paper-2 px-4 text-[13px]">
          <span className="mr-2 font-medium text-ink">{picked.length} {picked.length === 1 ? 'photo' : 'photos'} <span className="font-mono text-[12px] text-ink-3">· {fmtMB(pickedBytes)}</span></span>
          <button type="button" className={BTN} onClick={p.onSelectAll}>Select all</button>
          <button type="button" className={BTN} onClick={p.onShare} disabled={picked.length === 0}>Share…</button>
          <button type="button" className={BTN} onClick={p.onOffline} disabled={picked.length === 0}>Make available offline</button>
          <button type="button" className={BTN} onClick={p.onMove} disabled={picked.length === 0}>Move…</button>
          <button type="button" className={BTN} onClick={p.onTrash} disabled={picked.length === 0}>Move to Trash</button>
          <span className="flex-1" />
          <button type="button" className={BTN} onClick={p.onDone}>Done</button>
        </div>
      )}

      {hasPills && (
        <div className="flex flex-none flex-col gap-1.5 border-b border-line px-4 py-2">
          <div className="flex items-center gap-2" role="group" aria-label="Photo sources">
            <Pill label="All" on={p.device === null} onClick={() => p.onDevice?.(null)} />
            {p.devices.map((d) => <Pill key={d} label={d} on={p.device === d} onClick={() => p.onDevice?.(d)} />)}
          </div>
          <p className="m-0 text-[12px] text-ink-3" data-backup-status>
            {backing.length > 0
              ? backing.map((b) => `${b.name} ${b.status}`).join(' · ')
              : 'Every device is backed up.'}
          </p>
        </div>
      )}

      {p.loading ? (
        <div className="grid flex-1 content-start gap-1 overflow-hidden p-4" style={{ gridTemplateColumns: `repeat(${p.across}, minmax(0, 1fr))` }} data-loading="photos">
          {Array.from({ length: p.across * 3 }, (_, i) => <PhotoTileSkeleton key={i} />)}
        </div>
      ) : days.length === 0 ? (
        <EmptyPhotos onSources={p.onSources} />
      ) : (
        <div className="min-h-0 flex-1 overflow-y-auto px-4 pb-6 pt-3" data-photo-grid>
          {days.map((d) => (
            <section key={d.key} data-day={d.key} className="mb-5">
              <h2 className="m-0 mb-2 text-[13px] font-medium text-ink">{dayHeading(d.key, p.now, p.places?.[d.key])}</h2>
              <div className={`grid gap-1 ${GRID[p.across]}`}>
                {d.cells.map((c) => (
                  <Tile key={idOf(c)} cell={c} selected={p.selected.has(idOf(c))} selecting={p.selecting} onOpen={p.onOpen} onToggle={p.onToggle} />
                ))}
              </div>
            </section>
          ))}
        </div>
      )}
    </div>
  )
}
