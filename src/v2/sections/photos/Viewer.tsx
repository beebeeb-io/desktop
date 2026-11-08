// The photo viewer (DW-0605, DW-0605b, DW-0606, DW-0608, DW-0610, DW-0611): the photo large, the gallery strip, the
// actions. The preview is the source's large thumbnail; the original goes through DriveSource.open (decrypted in Rust).
import { useEffect, useState, type ReactNode } from 'react'
import { Icon } from '@beebeeb/shared/design'
import { BTN, FOCUS, IconButton } from '../files/ui'
import { fmtExt, idOf, mb1, isVideo, takenLabel, type PhotoCell } from './model'
import { useThumb } from './PhotosView'

export interface ViewerProps {
  cells: PhotoCell[]
  index: number
  now: Date
  /** 'original' with progress draws the thin bar over the photo (DW-0605b). */
  mode: 'preview' | 'original'
  progress?: { done: number; total: number } | null
  kept?: boolean
  keeping?: boolean
  /** Frame hook: the ⋯ menu drawn open (DW-0606). */
  moreOpen?: boolean
  onBack?: () => void
  onStep?: (delta: -1 | 1) => void
  onJump?: (i: number) => void
  onMode?: (m: 'preview' | 'original') => void
  onMore?: (id: 'save' | 'move' | 'rename' | 'duplicate') => void
  onShare?: () => void
  onOffline?: () => void
  onDetails?: () => void
  onTrash?: () => void
  onOpen?: () => void
}

function Thumb({ cell }: { cell: PhotoCell }) {
  const url = useThumb(idOf(cell), 'small')
  return url ? <img src={url} alt="" draggable={false} className="h-full w-full object-cover" /> : <span className="block h-full w-full animate-pulse bg-paper-3" />
}

function Action({ icon, label, onClick, children }: { icon: 'share' | 'pin' | 'info' | 'trash'; label: string; onClick?: () => void; children?: ReactNode }) {
  return (
    <button type="button" onClick={onClick} data-action={label} className={`inline-flex h-9 items-center gap-2 rounded-md px-3 text-[13px] text-ink hover:bg-paper-2 ${FOCUS}`}>
      <Icon name={icon} size={15} />{children ?? label}
    </button>
  )
}

export function PhotoViewer(p: ViewerProps) {
  const cell = p.cells[p.index]
  const big = useThumb(idOf(cell), 'large')
  const video = isVideo(cell)
  const size = cell.entry.item.size ?? 0
  const previewBytes = cell.meta?.previewBytes
  const [more, setMore] = useState(p.moreOpen ?? false)

  useEffect(() => {
    const on = (e: KeyboardEvent) => {
      if (e.key === 'Escape') p.onBack?.()
      else if (e.key === 'ArrowLeft') p.onStep?.(-1)
      else if (e.key === 'ArrowRight') p.onStep?.(1)
    }
    window.addEventListener('keydown', on)
    return () => window.removeEventListener('keydown', on)
  }, [p])

  const original = p.mode === 'original'
  const pct = p.progress ? Math.min(100, (p.progress.done / p.progress.total) * 100) : 0

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-paper" data-section="photo-viewer">
      <div className="flex h-12 flex-none items-center gap-3 border-b border-line px-3">
        <IconButton icon="chevron-right" flip label="Back to Photos (Esc)" onClick={p.onBack} />
        <h1 className="m-0 min-w-0 flex-1 truncate text-[14px] font-medium text-ink">{takenLabel(cell, p.now)}{video && cell.meta?.duration ? ` · ${cell.meta.duration}` : ''}</h1>
        {!video && (
          <div role="group" aria-label="Quality" className="flex h-8 items-center rounded-md border border-line p-0.5 text-[12px]">
            <button type="button" aria-pressed={!original} onClick={() => p.onMode?.('preview')} className={`h-7 rounded px-2.5 ${!original ? 'bg-paper-3 text-ink' : 'text-ink-3'} ${FOCUS}`}>
              Preview <span className="font-mono text-[11px] text-ink-3">· 4K {fmtExt(cell.entry.item.name)}</span>
            </button>
            <button type="button" aria-pressed={original} onClick={() => p.onMode?.('original')} className={`h-7 rounded px-2.5 ${original ? 'bg-paper-3 text-ink' : 'text-ink-3'} ${FOCUS}`}>
              Original <span className="font-mono text-[11px] text-ink-3">· {p.progress ? `${(p.progress.done / 1e6).toFixed(1)} of ${(p.progress.total / 1e6).toFixed(1)} MB` : `${(size / 1e6).toFixed(1)} MB`}</span>
            </button>
          </div>
        )}
        <div className="relative">
          <IconButton icon="more" label="More actions" pressed={more} onClick={() => setMore((m) => !m)} />
          {more && (
            <div role="menu" aria-label="More actions" data-menu="More actions" className="absolute right-0 top-9 z-30 w-56 rounded-lg border border-line bg-paper p-1 shadow-xl">
              {([['save', 'Save to PC…'], ['move', 'Move…'], ['rename', 'Rename'], ['duplicate', 'Duplicate']] as const).map(([id, label]) => (
                <button key={id} type="button" role="menuitem" onClick={() => { p.onMore?.(id); setMore(false) }} className={`flex h-8 w-full items-center rounded-md px-3 text-left text-[13px] text-ink hover:bg-paper-2 ${FOCUS}`}>{label}</button>
              ))}
            </div>
          )}
        </div>
      </div>

      <div className="relative flex min-h-0 flex-1 items-center justify-center bg-paper-2">
        {big ? <img src={big} alt={cell.entry.item.name} draggable={false} className="max-h-full max-w-full object-contain" /> : <div className="h-2/3 w-2/3 animate-pulse rounded-md bg-paper-3" />}
        {p.progress && original && (
          <div className="absolute inset-x-0 top-0 h-1 bg-ink/10" role="progressbar" aria-label="Downloading the original" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(pct)}>
            <div className="h-full bg-ink-3" style={{ width: `${pct}%` }} />
          </div>
        )}
        {video && (
          <div aria-hidden="true" data-reference="windows-player" className="absolute inset-x-8 bottom-4 flex h-11 items-center gap-3 rounded-lg bg-ink/75 px-3 text-paper">
            <Icon name="play" size={16} /><span className="font-mono text-[11px]">0:00</span>
            <span className="h-1 flex-1 rounded-full bg-paper/30"><span className="block h-full w-1/5 rounded-full bg-paper" /></span>
            <span className="font-mono text-[11px]">{cell.meta?.duration}</span>
          </div>
        )}
        {p.index > 0 && <button type="button" aria-label="Previous photo (←)" onClick={() => p.onStep?.(-1)} className={`absolute left-3 grid h-9 w-9 place-items-center rounded-full bg-paper/90 text-ink shadow ${FOCUS}`}><span className="inline-flex rotate-180"><Icon name="chevron-right" size={16} /></span></button>}
        {p.index < p.cells.length - 1 && <button type="button" aria-label="Next photo (→)" onClick={() => p.onStep?.(1)} className={`absolute right-3 grid h-9 w-9 place-items-center rounded-full bg-paper/90 text-ink shadow ${FOCUS}`}><Icon name="chevron-right" size={16} /></button>}
      </div>

      <div className="flex h-[72px] flex-none items-center gap-1.5 overflow-x-auto border-t border-line px-3" role="listbox" aria-label="Gallery" data-strip>
        {p.cells.map((c, i) => (
          <button key={idOf(c)} type="button" role="option" aria-selected={i === p.index} aria-label={c.entry.item.name} onClick={() => p.onJump?.(i)}
            className={`h-12 w-12 flex-none overflow-hidden rounded ${i === p.index ? 'outline outline-2 outline-offset-1 outline-ink' : 'opacity-80 hover:opacity-100'} ${FOCUS}`}>
            <Thumb cell={c} />
          </button>
        ))}
      </div>

      <div className="flex h-12 flex-none items-center gap-1 border-t border-line px-3">
        <Action icon="share" label="Share" onClick={p.onShare} />
        <Action icon="pin" label="Offline" onClick={p.onOffline}>{p.kept ? 'On this PC' : 'Offline'}</Action>
        <Action icon="info" label="Details" onClick={p.onDetails} />
        <Action icon="trash" label="Trash" onClick={p.onTrash} />
        <span className="flex-1" />
        {p.keeping && <span role="status" className="text-[12px] text-ink-3">Keeping the original on this PC</span>}
        {video && <button type="button" className={BTN} onClick={p.onOpen}>Open</button>}
        <span className="ml-3 font-mono text-[11px] text-ink-3">{mb1(size)}{previewBytes && !video ? ` · preview ${(previewBytes / 1e6).toFixed(1)} MB` : ''}</span>
      </div>
    </div>
  )
}
