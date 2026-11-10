// The ten viewers' bodies (DW-1501..1515). Each is a pure view over what it is handed, so a frame and the real
// section render the same component. Decrypted bytes may reach the view for display; keys, tokens and URLs never.
import { useMemo, useState, type ReactNode } from 'react'
import Markdown from 'react-markdown'
import { Icon } from '@beebeeb/shared/design'
import { openExternal } from '../../external'
import { actionFailure } from '../files/model'
import { BTN, FOCUS, IconButton } from '../files/ui'
import { HighlightedLine } from './Highlight'
import { fmtBytes, fmtClock, isNumeric, parseTable } from './model'

// ---- Photos: a RAW shows the camera's own preview, and says so (DW-1501) ----
export function RawViewer({ src, strip, label = 'Camera preview · Sony ARW', size = 'RAW · 61.4 MB', onPrev, onNext, onPick, onOpenElsewhere }: {
  src: string | null; strip: { id: string; src: string | null; current?: boolean }[]; label?: string; size?: string
  onPrev?: () => void; onNext?: () => void; onPick?: (id: string) => void; onOpenElsewhere?: () => void
}) {
  return (
    <div data-viewer="raw" className="relative flex min-h-0 flex-1 flex-col bg-viewer">
      <div className="relative flex min-h-0 flex-1 items-center justify-center">
        <div className="absolute left-4 top-4 flex gap-2">
          <span data-pill className="rounded-full bg-black/55 px-3 py-1 text-[12px] text-white">{label}</span>
          <span className="rounded-full bg-black/55 px-3 py-1 font-mono text-[12px] text-white">{size}</span>
        </div>
        <button type="button" aria-label="Previous photo" onClick={onPrev} className={`absolute left-3 grid h-9 w-9 place-items-center rounded-full bg-black/45 text-white ${FOCUS}`}><span className="inline-flex rotate-180"><Icon name="chevron-right" size={16} /></span></button>
        {src ? <img src={src} alt={label} className="max-h-full max-w-full object-contain" /> : <div className="h-64 w-96 rounded bg-viewer-placeholder" />}
        <button type="button" aria-label="Next photo" onClick={onNext} className={`absolute right-3 grid h-9 w-9 place-items-center rounded-full bg-black/45 text-white ${FOCUS}`}><Icon name="chevron-right" size={16} /></button>
        <button type="button" onClick={onOpenElsewhere} className={`absolute bottom-3 right-3 rounded-md bg-black/55 px-3 py-1.5 text-[12px] text-white ${FOCUS}`}>Open in another app…</button>
      </div>
      <div data-strip className="flex h-20 flex-none items-center justify-center gap-2 bg-black/40 px-4">
        {strip.map((s) => (
          <button key={s.id} type="button" data-strip-item aria-label={s.current ? 'This photo' : 'Show this photo'} aria-current={s.current || undefined} onClick={() => onPick?.(s.id)} className={`h-14 w-20 flex-none overflow-hidden rounded ${s.current ? 'outline outline-2 outline-white' : 'opacity-70'} ${FOCUS}`}>
            {s.src ? <img src={s.src} alt="" className="h-full w-full object-cover" /> : <div className="h-full w-full bg-viewer-placeholder" />}
          </button>
        ))}
      </div>
    </div>
  )
}

/** A plain picture for the other image types: the decrypted bytes as an object URL. */
export function ImageViewer({ src, name }: { src: string; name: string }) {
  return <div data-viewer="image" className="flex min-h-0 flex-1 items-center justify-center bg-paper-2"><img src={src} alt={name} className="max-h-full max-w-full object-contain" /></div>
}

// ---- Audio: the folder is the queue (DW-1503) ----
export interface Track { id: string; title: string; artist: string; length: number }

export function AudioPlayer({ tracks, index, playing, elapsed, onToggle, onPrev, onNext, onPick, onSeek, artwork, fileName }: {
  tracks: Track[]; index: number; playing: boolean; elapsed: number; onToggle?: () => void; onPrev?: () => void; onNext?: () => void; onPick?: (i: number) => void; onSeek?: (s: number) => void; artwork?: string | null; fileName?: string
}) {
  const t = tracks[index]
  const title = t.title || fileName || ''
  return (
    <div data-viewer="audio" className="flex min-h-0 flex-1 gap-8 overflow-auto p-8">
      <div className="flex w-80 flex-none flex-col items-center gap-4">
        <div className="grid h-72 w-72 place-items-center overflow-hidden rounded-xl border border-line bg-paper-2 text-ink-3">
          {artwork ? <img src={artwork} alt="" className="h-full w-full object-cover" /> : <Icon name="file-audio" size={64} />}
        </div>
        <div className="text-center"><div data-title className="text-[16px] font-semibold text-ink">{title}</div>{t.artist && <div className="text-[13px] text-ink-3">{t.artist}</div>}</div>
        <div className="flex w-full items-center gap-2 font-mono text-[11px] text-ink-3">
          <span>{fmtClock(elapsed)}</span>
          <input type="range" aria-label="Position" min={0} max={t.length} value={elapsed} onChange={(e) => onSeek?.(Number(e.target.value))} className="h-1 flex-1 accent-ink-2" />
          <span>{`−${fmtClock(t.length - elapsed)}`}</span>
        </div>
        <div className="flex items-center gap-3">
          <IconButton icon="chevron-right" label="Previous (Ctrl+Left)" flip onClick={onPrev} />
          <button type="button" aria-label={playing ? 'Pause (Space)' : 'Play (Space)'} onClick={onToggle} className={`grid h-11 w-11 place-items-center rounded-full border border-line bg-paper-2 text-ink ${FOCUS}`}><Icon name={playing ? 'pause' : 'play'} size={18} /></button>
          <IconButton icon="chevron-right" label="Next (Ctrl+Right)" onClick={onNext} />
        </div>
        <p className="m-0 text-center text-[12px] leading-5 text-ink-3">Decrypted on this PC · keeps playing when you leave this view · the media keys work</p>
      </div>
      <div className="min-w-0 flex-1">
        <h2 className="m-0 mb-2 text-[13px] font-semibold text-ink">Up next</h2>
        <ul className="m-0 list-none p-0">
          {tracks.map((k, i) => (
            <li key={k.id}>
              <button type="button" data-track aria-current={i === index} onClick={() => onPick?.(i)} className={`flex h-10 w-full items-center gap-3 rounded-md px-3 text-left text-[13px] hover:bg-paper-2 ${i === index ? 'bg-paper-3 font-medium' : ''} ${FOCUS}`}>
                <span className="w-5 font-mono text-[11px] text-ink-3">{i + 1}</span>
                <span className="min-w-0 flex-1 truncate text-ink">{k.title || fileName}</span>
                <span className="font-mono text-[12px] text-ink-3">{fmtClock(k.length)}</span>
              </button>
            </li>
          ))}
        </ul>
      </div>
    </div>
  )
}

/** Windows draws Quick Settings (DW-1504): system greys, not ours. Beebeeb only supplies the media session. */
export function QuickSettingsMedia({ title, artist, tracks }: { title: string; artist: string; tracks?: number }) {
  void tracks
  const tile = 'h-12 flex-1 rounded-lg bg-win-dark-tile'
  return (
    <div data-quick-settings className="absolute bottom-12 right-3 z-30 w-[360px] rounded-xl border border-win-dark-line bg-win-dark-surface p-4 font-sans text-white shadow-2xl">
      <div className="mb-3 flex items-center gap-3 rounded-lg bg-win-dark-tile p-3">
        <div className="grid h-16 w-16 flex-none place-items-center rounded bg-win-dark-art"><Icon name="file-audio" size={26} /></div>
        <div className="min-w-0 flex-1">
          <div className="text-[11px] text-white/60">Beebeeb</div>
          <a href="#open" data-media-title className="block truncate text-[13px] font-semibold text-white no-underline">{title}</a>
          <div className="truncate text-[12px] text-white/70">{artist}</div>
          <div className="mt-1 flex items-center gap-4 text-white"><span className="inline-flex rotate-180"><Icon name="chevron-right" size={14} /></span><Icon name="pause" size={16} /><Icon name="chevron-right" size={14} /></div>
        </div>
      </div>
      <div className="flex gap-2"><div className={tile} /><div className={tile} /><div className={tile} /></div>
      <div className="mt-2 flex gap-2"><div className={tile} /><div className={tile} /><div className={tile} /></div>
    </div>
  )
}

// ---- PDF: pages on grey, thumbnails beside them (DW-1505, DW-1506) ----
export function PdfViewer({ pages, page, zoom, onPage, query, current, noText }: { pages: string[][]; page: number; zoom: number; onPage?: (i: number) => void; query?: string; current?: number; noText?: boolean }) {
  const scrollTo = (i: number) => onPage?.(i)
  void noText
  let seen = -1
  const mark = (text: string) => {
    const q = query?.trim()
    if (!q) return text
    const parts = text.split(new RegExp(`(${q.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')})`, 'gi'))
    return parts.map((p, i) => {
      if (p.toLowerCase() !== q.toLowerCase()) return p
      seen += 1
      return <mark key={i} data-match data-current={seen === current} className={seen === current ? 'rounded-sm bg-ink-3/60 text-ink' : 'rounded-sm bg-ink-4/30 text-ink'}>{p}</mark>
    })
  }
  return (
    <div data-viewer="pdf" className="flex min-h-0 flex-1 bg-paper-3">
      <ol aria-label="Pages" className="m-0 flex w-32 flex-none list-none flex-col items-center gap-3 overflow-auto border-r border-line bg-paper-2 p-3">
        {pages.map((_, i) => (
          <li key={i}>
            <button type="button" data-thumb aria-current={i === page} aria-label={`Page ${i + 1}`} onClick={() => scrollTo(i)} className={`flex w-20 flex-col items-center gap-1 ${FOCUS}`}>
              <span className={`block h-24 w-20 rounded-sm bg-paper ${i === page ? 'outline outline-2 outline-ink' : 'outline outline-1 outline-line'}`} />
              <span className="font-mono text-[11px] text-ink-3">{i + 1}</span>
            </button>
          </li>
        ))}
      </ol>
      <div className="flex min-w-0 flex-1 justify-center overflow-auto p-6">
        <article data-pdf-page aria-label={`Page ${page + 1}`} style={{ width: 460 * zoom, minHeight: 600 * zoom, fontSize: 13 * zoom }} className="flex-none bg-paper p-10 leading-6 text-ink shadow-lg">
          {pages[page].map((line, i) => (
            <p key={i} className={`m-0 mb-3 ${line.length < 28 && !line.includes('.') ? 'text-[1.25em] font-semibold' : ''}`}>{mark(line)}</p>
          ))}
        </article>
      </div>
    </div>
  )
}

export function pdfMatches(pages: string[][], q: string): { page: number }[] {
  const needle = q.trim().toLowerCase()
  if (!needle) return []
  return pages.flatMap((lines, page) => lines.flatMap((l) => Array.from(l.toLowerCase().matchAll(new RegExp(needle.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), 'g'))).map(() => ({ page }))))
}

export function PdfZoom({ zoom, onZoom }: { zoom: number; onZoom: (z: number) => void }) {
  return (
    <span className="flex items-center gap-1" role="group" aria-label="Zoom">
      <button type="button" aria-label="Zoom out (Ctrl+−)" onClick={() => onZoom(Math.max(0.5, +(zoom - 0.25).toFixed(2)))} className={`${BTN} h-8 w-8 px-0`}>{'−'}</button>
      <span className="w-12 text-center font-mono text-[12px] text-ink-2">{Math.round(zoom * 100)}%</span>
      <button type="button" aria-label="Zoom in (Ctrl+Plus)" onClick={() => onZoom(Math.min(3, +(zoom + 0.25).toFixed(2)))} className={`${BTN} h-8 w-8 px-0`}>+</button>
    </span>
  )
}

// ---- Text and code (DW-1512) ----
export function CodeViewer({ text, wrap, limit }: { text: string; wrap: boolean; limit?: number }) {
  const lines = useMemo(() => { const l = text.replace(/\n$/, '').split('\n'); return limit ? l.slice(0, limit) : l }, [text, limit])
  return (
    <div data-viewer="code" className="min-h-0 flex-1 overflow-auto bg-paper">
      <pre aria-label="Code" className="m-0 py-3 font-mono text-[13px] leading-6 text-ink">
        {lines.map((line, i) => (
          <div key={i} data-line className="flex">
            <span aria-hidden="true" className="w-12 flex-none select-none pr-3 text-right text-ink-4">{i + 1}</span>
            <code className={`min-w-0 flex-1 ${wrap ? 'whitespace-pre-wrap break-words' : 'whitespace-pre'}`}><HighlightedLine line={line} /></code>
          </div>
        ))}
      </pre>
    </div>
  )
}

// ---- Markdown (DW-1513) ----
export function MarkdownViewer({ text, mode, onOpenFile }: { text: string; mode: 'rendered' | 'source'; onOpenFile?: (name: string) => void }) {
  const [refusal, setRefusal] = useState<string | null>(null)
  if (mode === 'source') return <CodeViewer text={text} wrap />
  const openLink = (href: string) => {
    void openExternal(href).then((r) => {
      if (!r.ok) setRefusal(actionFailure('open the link', r.unsupported ? 'unsupported' : 'failed').text)
    })
  }
  return (
    <div data-viewer="markdown" className="min-h-0 flex-1 overflow-auto bg-paper">
      {refusal ? <p role="alert" className="mx-auto mt-4 max-w-[680px] px-10 text-[12px] text-red-text">{refusal}</p> : null}
      <article className="mx-auto max-w-[680px] px-10 py-8 text-[14px] leading-7 text-ink">
        <Markdown
          // Raw HTML in a note is shown as text, never run: no rehype-raw, and html nodes render as plain text.
          skipHtml={false}
          components={{
            h1: ({ children }) => <h1 className="mb-3 mt-0 text-[24px] font-bold tracking-tight text-ink">{children}</h1>,
            h2: ({ children }) => <h2 className="mb-2 mt-7 text-[17px] font-semibold text-ink">{children}</h2>,
            h3: ({ children }) => <h3 className="mb-2 mt-5 text-[14px] font-semibold text-ink">{children}</h3>,
            p: ({ children }) => <p className="m-0 mb-3">{children}</p>,
            ul: ({ children }) => <ul className="m-0 mb-3 list-disc pl-6">{children}</ul>,
            ol: ({ children }) => <ol className="m-0 mb-3 list-decimal pl-6">{children}</ol>,
            code: ({ children }) => <code className="rounded bg-paper-3 px-1 py-0.5 font-mono text-[12.5px]">{children}</code>,
            a: ({ href, children }) => {
              if (href?.startsWith('beebeeb://file/')) {
                const name = decodeURIComponent(href.slice('beebeeb://file/'.length))
                return <a href="#file" data-drive-link className="text-ink underline" onClick={(e) => { e.preventDefault(); onOpenFile?.(name) }}>{children}</a>
              }
              // An http(s) address opens in the OS, never in this window (external.ts). Any other scheme is shown as
              // plain text: a note cannot make the window navigate somewhere it does not handle.
              if (href && /^https?:\/\//i.test(href)) {
                return <a href={href} className="text-ink underline" onClick={(e) => { e.preventDefault(); openLink(href) }}>{children}</a>
              }
              return <span>{children}</span>
            },
          }}
          urlTransform={(url) => url}
        >
          {text}
        </Markdown>
      </article>
    </div>
  )
}

// ---- CSV and TSV as a table (DW-1514) ----
export function CsvViewer({ text }: { text: string }) {
  const table = useMemo(() => parseTable(text), [text])
  const [sel, setSel] = useState<string | null>(null)
  const numeric = table.header.map((_, c) => table.rows.length > 0 && table.rows.every((r) => isNumeric(r[c] ?? '')))
  return (
    <div data-viewer="csv" className="flex min-h-0 flex-1 flex-col bg-paper">
      <div data-csv-note className="flex-none border-b border-line px-4 py-2 font-mono text-[12px] text-ink-3">{table.sep.label} · {table.header.length} columns · {table.rows.length} rows</div>
      <div className="min-h-0 flex-1 overflow-auto">
        <table className="w-full border-collapse text-[13px]">
          <thead className="sticky top-0 bg-paper-2">
            <tr>{table.header.map((h, c) => <th key={c} scope="col" className={`border-b border-line px-3 py-2 font-semibold text-ink ${numeric[c] ? 'text-right' : 'text-left'}`}>{h}</th>)}</tr>
          </thead>
          <tbody>
            {table.rows.map((r, ri) => (
              <tr key={ri} data-row>
                {table.header.map((_, c) => {
                  const key = `${ri}:${c}`
                  return <td key={c} tabIndex={0} aria-selected={sel === key} onClick={() => setSel(key)} onFocus={() => setSel(key)} className={`h-9 cursor-default border-b border-line px-3 text-ink ${numeric[c] ? 'text-right font-mono tabular-nums' : ''} ${sel === key ? 'outline outline-2 -outline-offset-2 outline-ink-2' : ''}`}>{r[c] ?? ''}</td>
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  )
}

// ---- Archive: what is inside, never unpacked (DW-1515) ----
export function ArchiveViewer({ rows, note }: { rows: { path: string; size: number | null; depth: number }[]; note: string }) {
  return (
    <div data-viewer="archive" className="flex min-h-0 flex-1 flex-col bg-paper">
      <div role="note" data-archive-note className="flex flex-none items-center gap-2 border-b border-line bg-paper-2 px-4 py-2 text-[13px] text-ink-2"><Icon name="info" size={14} />{note}</div>
      <ul className="m-0 min-h-0 flex-1 list-none overflow-auto p-0">
        {rows.map((r, i) => (
          <li key={i} data-row className="flex h-9 items-center gap-2 border-b border-line pr-4 text-[13px]" style={{ paddingLeft: 16 + r.depth * 20 }}>
            <span className="text-ink-3"><Icon name={r.size === null ? 'folder' : 'file'} size={14} /></span>
            <span className="min-w-0 flex-1 truncate text-ink">{r.path}</span>
            <span className="font-mono text-[12px] text-ink-3">{r.size === null ? '' : fmtBytes(r.size)}</span>
          </li>
        ))}
      </ul>
    </div>
  )
}

export function Centered({ children }: { children: ReactNode }) {
  return <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-3 bg-paper p-8 text-center">{children}</div>
}
