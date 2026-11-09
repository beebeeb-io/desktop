// Search (DW-0801 recent, DW-0802 results, DW-0802b while the list loads, DW-0803 no match). Names only, in the copy of
// the list on this PC. A view: the host passes the query, the hits and the recent searches.
import { Fragment, type ReactNode } from 'react'
import { Icon } from '@beebeeb/shared/design'
import { FileIcon, getFileType, type DriveItem } from '@beebeeb/shared/drive-ui'
import { FOCUS } from '../files/ui'
import { fmtCount, fmtSize, fmtWhen, LOCAL_LABEL } from '../files/model'
import { BannerSlot } from '../../chrome/Banners'

export interface SearchHit { item: DriveItem; where: string }
export const PILLS = ['All', 'Folders', 'Photos', 'Documents'] as const
export type Pill = (typeof PILLS)[number]

const PHOTO = /\.(jpe?g|png|gif|webp|heic|heif|avif|dng|tiff?|mp4|mov|m4v|webm)$/i
const DOC = /\.(pdf|docx?|xlsx?|pptx?|odt|ods|odp|txt|md|rtf|csv|pages|numbers|key)$/i
export function byPill(hits: SearchHit[], pill: Pill): SearchHit[] {
  if (pill === 'Folders') return hits.filter((h) => h.item.isFolder)
  if (pill === 'Photos') return hits.filter((h) => !h.item.isFolder && (PHOTO.test(h.item.name) || h.item.mime?.startsWith('image/') || h.item.mime?.startsWith('video/')))
  if (pill === 'Documents') return hits.filter((h) => !h.item.isFolder && DOC.test(h.item.name))
  return hits
}

// The same fold as the on-device index (NFKD, drop combining marks, then Unicode lowercase). A JS-only
// toLowerCase would bold a different span, including for İ and ﬁ.
const COMBINING = /[\u0300-\u036F\u1AB0-\u1AFF\u1DC0-\u1DFF\u20D0-\u20FF\uFE20-\uFE2F]/u
const stripMarks = (value: string) => {
  let out = ''
  for (const ch of value) if (!COMBINING.test(ch)) out += ch
  return out
}
const foldName = (value: string) => stripMarks(value.normalize('NFKD')).toLowerCase()

function foldIndexed(name: string): { text: string; from: number[] } {
  const pieces: { ch: string; at: number }[] = []
  for (let i = 0; i < name.length;) {
    const raw = String.fromCodePoint(name.codePointAt(i)!)
    for (const ch of stripMarks(raw.normalize('NFKD'))) pieces.push({ ch, at: i })
    i += raw.length
  }
  const text = pieces.map((piece) => piece.ch).join('').toLowerCase()
  const from: number[] = []
  for (const piece of pieces) for (let k = 0; k < piece.ch.toLowerCase().length; k++) from.push(piece.at)
  return from.length === text.length ? { text, from } : { text, from: [] }
}

/** The name with every match of the query in bold. The query is text, never HTML. */
export function Highlight({ name, query }: { name: string; query: string }): ReactNode {
  const needle = foldName(query.trim()).trim()
  if (!needle) return name
  const { text, from } = foldIndexed(name)
  if (from.length !== text.length) return name
  const ranges: Array<[number, number]> = []
  for (let at = text.indexOf(needle), searchFrom = 0; at !== -1; at = text.indexOf(needle, searchFrom)) {
    const start = from[at]!
    const last = from[at + needle.length - 1]!
    ranges.push([start, last + ((name.codePointAt(last) ?? 0) > 0xffff ? 2 : 1)])
    searchFrom = at + needle.length
  }
  if (ranges.length === 0) return name
  const out: ReactNode[] = []
  let cursor = 0
  ranges.forEach(([start, end], n) => {
    if (end <= cursor) return
    const fromAt = Math.max(start, cursor)
    if (fromAt > cursor) out.push(<Fragment key={`t${n}`}>{name.slice(cursor, fromAt)}</Fragment>)
    out.push(<b key={`b${n}`} className="font-semibold text-ink">{name.slice(fromAt, end)}</b>)
    cursor = end
  })
  if (cursor < name.length) out.push(<Fragment key="end">{name.slice(cursor)}</Fragment>)
  return out
}

const NOTE = 'Searches file and folder names on this PC, even offline. It doesn’t look inside files.'
const COLS = 'grid-cols-[minmax(0,2.4fr)_minmax(120px,1fr)_88px_minmax(0,1.3fr)_120px]'

export interface SearchViewProps {
  query: string
  hits: SearchHit[]
  pill?: Pill
  recent?: string[]
  /** The first sign-in is still listing files: results may be incomplete. */
  loading?: { done: number; total: number } | null
  now?: Date
  menuOn?: string
  /** The selected result (a click, or ↓ ↑ from the field and between rows): drawn neutral grey, as in Files. */
  selected?: string | null
  onSelect?: (id: string) => void
  onQuery?: (q: string) => void
  onPill?: (p: Pill) => void
  onOpen?: (item: DriveItem) => void
  onContext?: (item: DriveItem, x: number, y: number) => void
  onClearRecent?: () => void
  onEnd?: () => void
  /** The last search failed. Rows already shown stay; Try again runs the same query. */
  error?: string | null
  onRetry?: () => void
}

export function SearchView(p: SearchViewProps) {
  const { query } = p
  const pill = p.pill ?? 'All'
  const shown = byPill(p.hits, pill)
  const typed = query.trim().length > 0
  const focusFirst = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Escape') p.onEnd?.()
    if (e.key === 'ArrowDown') { e.preventDefault(); document.querySelector<HTMLElement>('[data-result]')?.focus() }
  }
  const moveIn = (e: React.KeyboardEvent<HTMLElement>, item: DriveItem) => {
    const el = e.currentTarget
    if (e.key === 'ArrowDown') { e.preventDefault(); (el.nextElementSibling as HTMLElement | null)?.focus() }
    else if (e.key === 'ArrowUp') { e.preventDefault(); ((el.previousElementSibling as HTMLElement | null)?.dataset.result ? el.previousElementSibling as HTMLElement : document.querySelector<HTMLElement>('[data-search-field]'))?.focus() }
    else if (e.key === 'Enter') p.onOpen?.(item)
    else if (e.key === 'Escape') p.onEnd?.()
  }
  return (
    <section aria-label="Search" data-search={typed ? (shown.length ? 'results' : 'none') : 'recent'} className="flex min-h-0 min-w-0 flex-1 flex-col">
      <div role="toolbar" aria-label="Search" className="flex h-12 flex-none items-center gap-2 border-b border-line px-4">
        <div className="flex h-8 min-w-0 flex-1 items-center gap-2 rounded-md border border-ink-3 bg-paper px-2.5 text-ink-3">
          <Icon name="search" size={14} />
          <input data-search-field ref={(el) => el?.focus({ preventScroll: true })} aria-label="Search names" placeholder="Search" value={query}
            onChange={(e) => p.onQuery?.(e.target.value)} onKeyDown={focusFirst} spellCheck={false}
            className="h-full min-w-0 flex-1 bg-transparent text-[13px] text-ink outline-none placeholder:text-ink-3" />
          <span className="font-mono text-[11px]">Ctrl+F</span>
          <button type="button" aria-label="Close search (Esc)" title="Close search (Esc)" onClick={p.onEnd} className={`grid h-6 w-6 place-items-center rounded text-ink-3 hover:bg-paper-3 ${FOCUS}`}><Icon name="x" size={12} /></button>
        </div>
      </div>
      {/* The window's banner sits under the toolbar on every view (DW-0101 to DW-0106). */}
      <BannerSlot />

      {!typed ? (
        <div className="min-h-0 flex-1 overflow-auto px-4 py-3">
          <div className="mb-1 flex items-center justify-between">
            <h2 className="m-0 text-[12px] font-semibold text-ink-2">Recent</h2>
            {(p.recent?.length ?? 0) > 0 && <button type="button" onClick={p.onClearRecent} className={`rounded px-1.5 text-[12px] text-ink-2 hover:bg-paper-2 ${FOCUS}`}>Clear</button>}
          </div>
          <ul aria-label="Recent searches" data-recent={p.recent?.length ?? 0} className="m-0 list-none p-0">
            {(p.recent ?? []).map((r) => (
              <li key={r}><button type="button" onClick={() => p.onQuery?.(r)} className={`flex h-9 w-full items-center gap-2.5 rounded-md px-2 text-left text-[13px] text-ink hover:bg-paper-2 ${FOCUS}`}><span className="text-ink-3"><Icon name="clock" size={14} /></span>{r}</button></li>
            ))}
          </ul>
          <p className="m-0 mt-4 text-[12px] leading-4 text-ink-3">{NOTE}</p>
        </div>
      ) : (
        <>
          <div role="group" aria-label="Narrow by type" className="flex flex-none gap-2 px-4 py-2">
            {PILLS.map((x) => {
              const count = byPill(p.hits, x).length
              return (
                <button key={x} type="button" data-pill={x} data-count={count} aria-pressed={pill === x} onClick={() => p.onPill?.(x)} className={`h-7 rounded-full border px-3 text-[12px] ${pill === x ? 'border-ink-3 bg-paper-3 text-ink' : 'border-line text-ink-2 hover:bg-paper-2'} ${FOCUS}`}>{x} <span className="font-mono">{count}</span></button>
              )
            })}
          </div>
          {p.loading && (
            <p role="status" data-search-loading className="m-0 mx-4 mb-2 flex items-center gap-2 rounded-md border border-line bg-paper-2 px-3 py-2 text-[12px] leading-4 text-ink-2">
              <Icon name="info" size={13} />
              <span>The index is still building: <span className="font-mono">{p.loading.done.toLocaleString('en-US')}</span> of <span className="font-mono">{p.loading.total.toLocaleString('en-US')}</span>.</span>
            </p>
          )}
          {p.error && (
            <div role="alert" data-search-error className="mx-4 mb-2 flex items-center justify-between gap-3 rounded-md border border-line bg-paper-2 px-3 py-2 text-[12px] leading-4 text-ink-2">
              <span>{p.error}</span>
              <button type="button" onClick={p.onRetry} className={`h-7 shrink-0 rounded-md border border-line bg-paper px-2.5 text-[12px] text-ink hover:bg-paper-3 ${FOCUS}`}>Try again</button>
            </div>
          )}
          {shown.length === 0 && !p.error ? (
            <div data-empty="search" className="flex flex-1 flex-col items-center justify-center gap-2 px-8 text-center">
              <div className="mb-1 grid h-16 w-16 place-items-center rounded-2xl border border-line bg-paper-2 text-ink-3"><Icon name="search" size={26} /></div>
              <h2 className="m-0 text-[15px] font-semibold text-ink">No files match</h2>
              <p className="m-0 max-w-[400px] text-[13px] leading-5 text-ink-2">Check the spelling, or try fewer letters. Search only looks at names, not inside files.</p>
            </div>
          ) : (
            <div role="grid" aria-label="Results" data-results={shown.length} className="flex min-h-0 flex-1 flex-col">
              <div role="row" className={`grid ${COLS} h-8 flex-none items-center gap-3 border-y border-line px-4 text-[12px] text-ink-3`}>
                {['Name', 'Modified', 'Size', 'Where', 'PC'].map((c) => <span key={c} role="columnheader">{c}</span>)}
              </div>
              <div className="min-h-0 flex-1 overflow-auto">
                {shown.map(({ item, where }) => (
                  <div key={item.id} role="row" tabIndex={-1} data-result={item.id} aria-selected={p.menuOn === item.id || p.selected === item.id}
                    aria-label={where ? `${item.name}, ${where}` : item.name}
                    onClick={() => p.onSelect?.(item.id)} onFocus={() => p.onSelect?.(item.id)}
                    onDoubleClick={() => p.onOpen?.(item)} onKeyDown={(e) => moveIn(e, item)} onContextMenu={(e) => { e.preventDefault(); p.onContext?.(item, e.clientX, e.clientY) }}
                    className={`grid ${COLS} h-10 cursor-default items-center gap-3 px-4 text-[13px] ${p.menuOn === item.id || p.selected === item.id ? 'bg-paper-3' : 'hover:bg-paper-2'} focus-visible:outline-none`}>
                    <span role="gridcell" className="flex min-w-0 items-center gap-2.5"><FileIcon type={getFileType(item.name, item.isFolder)} size={22} /><span className="truncate text-ink-2" title={item.name}><Highlight name={item.name} query={query} /></span></span>
                    <span role="gridcell" className="font-mono text-[12px] text-ink-3">{fmtWhen(item.modifiedAt, p.now)}</span>
                    <span role="gridcell" className="font-mono text-[12px] text-ink-3">{item.isFolder ? fmtCount(item.childCount) : fmtSize(item.size)}</span>
                    <span role="gridcell" className="truncate text-[12px] text-ink-2" title={where}>{where}</span>
                    <span role="gridcell" className="truncate text-[12px] text-ink-3">{item.local ? LOCAL_LABEL[item.local] : ''}</span>
                  </div>
                ))}
              </div>
            </div>
          )}
          <p className="m-0 flex-none border-t border-line bg-paper-2 px-4 py-2 text-[12px] leading-4 text-ink-3">{NOTE} Trash and files not yet listed on this PC aren’t searched.</p>
        </>
      )}
    </section>
  )
}
