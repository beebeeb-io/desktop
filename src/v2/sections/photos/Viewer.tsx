// The photo viewer (DW-0605, DW-0605b, DW-0606, DW-0608, DW-0610, DW-0611): the photo large, the gallery strip, the
// actions. The preview is the source's large thumbnail; the original goes through DriveSource.open (decrypted in Rust).
import { useEffect, useRef, useState, type ReactNode } from 'react'
import { Icon } from '@beebeeb/shared/design'
import { useDriveSource } from '@beebeeb/shared/drive-ui'
import { BTN, FOCUS, IconButton } from '../files/ui'
import { fmtExt, idOf, mb1, isVideo, takenLabel, videoReadable, type PhotoCell } from './model'
import { useDriveExtras } from '../../source/extras'
import { READ_CAP_BYTES } from '../../source/desktop'
import { noRoomWords } from '../open/States'
import { useThumb } from './PhotosView'
import { keyBelongsElsewhere } from '../open/model'

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
  /** Frame hook: draw Windows' player controls for reference (DW-0611); live, the video element draws its own. */
  referencePlayer?: boolean
  onBack?: () => void
  onStep?: (delta: -1 | 1) => void
  onJump?: (i: number) => void
  onMode?: (m: 'preview' | 'original') => void
  onMore?: (id: 'save' | 'move' | 'rename' | 'duplicate') => void
  onShare?: () => void
  onOffline?: () => void
  /** DW-0608: "On this PC" offers this; the original stops being kept here. */
  onRemoveLocal?: () => void
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
  const [offer, setOffer] = useState(false)

  useEffect(() => {
    const on = (e: KeyboardEvent) => {
      if (e.key === 'Escape') p.onBack?.()
      // The video's own controls take the arrows (seeking) while they have the focus; a dialog over the photo keeps its keys.
      else if (keyBelongsElsewhere(e)) return
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
        {video && !p.referencePlayer
          ? <VideoPlayer id={idOf(cell)} item={cell.entry.item} poster={big} name={cell.entry.item.name} onOpen={p.onOpen} />
          : big ? <img src={big} alt={cell.entry.item.name} draggable={false} className="max-h-full max-w-full object-contain" /> : <div className="h-2/3 w-2/3 animate-pulse rounded-md bg-paper-3" />}
        {p.progress && original && (
          <div className="absolute inset-x-0 top-0 h-1 bg-ink/10" role="progressbar" aria-label="Downloading the original" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(pct)}>
            <div className="h-full bg-ink-3" style={{ width: `${pct}%` }} />
          </div>
        )}
        {video && p.referencePlayer && (
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
        <div className="relative">
          <Action icon="pin" label="Offline" onClick={p.kept ? () => setOffer((o) => !o) : p.onOffline}>{p.kept ? 'On this PC' : 'Offline'}</Action>
          {p.kept && offer && (
            <div role="menu" aria-label="On this PC" className="absolute bottom-10 left-0 z-30 w-56 rounded-lg border border-line bg-paper p-1 shadow-xl">
              <button type="button" role="menuitem" onClick={() => { setOffer(false); p.onRemoveLocal?.() }} className={`flex h-8 w-full items-center rounded-md px-3 text-left text-[13px] text-ink hover:bg-paper-2 ${FOCUS}`}>Remove from this PC</button>
            </div>
          )}
        </div>
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

/**
 * DW-0611: the video in the window, decrypted on this PC (DriveSource.readBytes), with WebView2's own player controls.
 * Space plays and pauses. A video the window cannot read (too large, a codec WebView2 lacks) says so and opens in
 * Windows' player instead.
 */
function VideoPlayer({ id, item, poster, name, onOpen }: { id: string; item: { size: number | null; local?: string | null }; poster: string | null; name: string; onOpen?: () => void }) {
  const source = useDriveSource()
  const extras = useDriveExtras()
  const [url, setUrl] = useState<string | null>(null)
  const [failed, setFailed] = useState<null | 'cant-play' | 'too-large' | { needed: number; free: number }>(null)
  const ref = useRef<HTMLVideoElement>(null)
  // Read when the video opens: a change to its local state while it plays (the read itself hydrates it) never reads it
  // again or frees what is playing (self-review).
  const at = useRef(item)
  at.current = item
  useEffect(() => {
    const { size, local } = at.current
    let live = true
    let made: string | null = null
    setUrl(null); setFailed(null)
    ;(async () => {
      // Decided before a byte is read: over the read cap, or without the room to decrypt a video not on this PC, the
      // window would hydrate part of it and fail (#273 finding 7). Free space is read only for a video not on this PC.
      const free = local === 'online-only' ? await extras.status().then((s) => s.freeSpaceBytes, () => undefined) : undefined
      const can = videoReadable({ size, local }, free, READ_CAP_BYTES)
      if (!live) return
      if (can === 'too-large') return setFailed('too-large')
      if (can === 'no-room') return setFailed({ needed: size ?? 0, free: free ?? 0 })
      const blob = await source.readBytes(id).then((r) => (r.bytes.byteLength > 0 ? new Blob([r.bytes as BlobPart]) : null))
      // The bytes are copied into the Blob and not kept: only the Blob holds the video while it plays.
      if (!live) return
      if (!blob) return setFailed('cant-play')
      made = URL.createObjectURL(blob)
      setUrl(made)
    })().catch(() => { if (live) setFailed('cant-play') })
    return () => { live = false; if (made) URL.revokeObjectURL(made) }
  }, [source, extras, id])
  useEffect(() => {
    const on = (e: KeyboardEvent) => {
      const v = ref.current
      if (e.key !== ' ' || !v || keyBelongsElsewhere(e)) return
      e.preventDefault()
      if (v.paused) void v.play().catch(() => {})
      else v.pause()
    }
    window.addEventListener('keydown', on)
    return () => window.removeEventListener('keydown', on)
  }, [])
  // A person's own video from their phone: there is no caption file to load with it.
  // eslint-disable-next-line jsx-a11y/media-has-caption
  if (url) return <video ref={ref} src={url} poster={poster ?? undefined} controls onError={() => setFailed('cant-play')} aria-label={name} className="max-h-full max-w-full" data-video />
  return (
    <div className="flex flex-col items-center gap-3">
      {poster ? <img src={poster} alt={name} draggable={false} className="max-h-[60vh] max-w-full object-contain" /> : <div className="h-64 w-96 animate-pulse rounded-md bg-paper-3" />}
      {failed === null
        ? <p role="status" className="m-0 text-[13px] text-ink-3">Decrypting on this PC…</p>
        : typeof failed === 'object'
          ? <p role="status" data-video-state="no-room" className="m-0 text-[13px] text-ink-2">{noRoomWords(failed.needed, failed.free)}</p>
          : <p role="status" data-video-state={failed} className="m-0 text-[13px] text-ink-2">{failed === 'too-large' ? 'This video is too large to play in the window.' : 'This video can’t play in the window.'} <button type="button" onClick={onOpen} className="text-ink underline">Open it in Windows’ player</button></p>}
    </div>
  )
}
