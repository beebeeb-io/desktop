// Sync history (DW-1301 to DW-1305, DW-0428): the toolbar with the filters and the search, the bar on Problems,
// the days. Presentational; the section feeds it the rows and the handlers.
import { useState } from 'react'
import { Icon } from '@beebeeb/shared/design'
import { BTN, FOCUS, Menu, type MenuEntry } from '../files/ui'
import { applyFilter, FILTERS, groupDays, isProblem, problemCount, timeLabel, type Filter, type HistoryRow } from './model'
import { BannerSlot } from '../../chrome/Banners'

export interface HistoryViewProps {
  rows: HistoryRow[]
  now: Date
  filter: Filter
  query: string
  onFilter?: (f: Filter) => void
  onQuery?: (q: string) => void
  onTry?: (id: string) => void
  onTryAll?: () => void
  onOpen?: (row: HistoryRow) => void
  onMenu?: (id: 'reveal' | 'beebeeb' | 'copy', row: HistoryRow) => void
}

export const MENU: MenuEntry[] = [
  { id: 'reveal', label: 'Show in File Explorer' },
  { id: 'beebeeb', label: 'Show in Beebeeb' },
  { id: 'copy', label: 'Copy name' },
]

function Row({ row, onTry, onOpen, onContext }: { row: HistoryRow; onTry?: (id: string) => void; onOpen?: (r: HistoryRow) => void; onContext: (e: React.MouseEvent, r: HistoryRow) => void }) {
  const bad = isProblem(row)
  const icon = bad ? 'info' : row.kind === 'folder' ? 'folder' : row.what.startsWith('Download') ? 'download' : row.what.startsWith('Moved to Trash') ? 'trash' : row.what.startsWith('Uploaded') || row.what.startsWith('Backed') ? 'arrow-up' : 'file'
  return (
    <li data-row={row.id} data-problem={bad || undefined} onDoubleClick={() => onOpen?.(row)} onContextMenu={(e) => onContext(e, row)} className="flex min-h-12 items-center gap-3 border-b border-line px-4 py-1.5">
      <span className={`grid h-8 w-8 flex-none place-items-center rounded-md ${bad ? 'bg-paper-3 text-red-text' : 'bg-paper-2 text-ink-2'}`}><Icon name={icon} size={15} /></span>
      <div className="min-w-0 flex-1">
        <div className="truncate text-[13px] text-ink">{row.name}</div>
        <div className="truncate text-[12px] text-ink-3">{bad ? row.problem : row.what}</div>
      </div>
      {bad && <button type="button" className={BTN} onClick={() => onTry?.(row.id)}>Try again</button>}
      <span className="w-32 flex-none truncate text-right text-[12px] text-ink-3">{row.device}</span>
      <span className="w-12 flex-none text-right font-mono text-[12px] text-ink-3">{timeLabel(row.at)}</span>
    </li>
  )
}

export function HistoryView(p: HistoryViewProps) {
  const [menu, setMenu] = useState<{ x: number; y: number; row: HistoryRow } | null>(null)
  const problems = problemCount(p.rows)
  const shown = applyFilter(p.rows, p.filter, p.query)
  const days = groupDays(shown, p.now)

  return (
    <div className="flex min-h-0 flex-1 flex-col" data-section="sync-history">
      <div className="flex h-12 flex-none items-center gap-3 border-b border-line px-4">
        <h1 className="m-0 text-[15px] font-semibold text-ink">Sync history</h1>
        <span className="text-[12px] text-ink-3">Last 30 days</span>
        <div role="group" aria-label="Filter" className="ml-3 flex items-center gap-1">
          {FILTERS.map((f) => (
            <button key={f.id} type="button" aria-pressed={p.filter === f.id} data-filter={f.id} onClick={() => p.onFilter?.(f.id)}
              className={`h-7 whitespace-nowrap rounded-full border px-3 text-[12px] ${p.filter === f.id ? 'border-ink bg-ink text-paper' : 'border-line text-ink-2 hover:bg-paper-2'} ${FOCUS}`}>
              {f.label}{f.id === 'problems' && problems > 0 && <span className="ml-1.5 font-mono text-[11px]">· {problems}</span>}
            </button>
          ))}
        </div>
        <span className="flex-1" />
        <label className="relative">
          <span className="sr-only">Search history</span>
          <span className="pointer-events-none absolute left-2.5 top-2 text-ink-3"><Icon name="search" size={14} /></span>
          <input type="search" value={p.query} placeholder="Search history" data-search onChange={(e) => p.onQuery?.(e.target.value)}
            onKeyDown={(e) => { if (e.key === 'Escape') p.onQuery?.('') }}
            className={`h-8 w-56 rounded-md border border-line bg-paper pl-8 pr-2 text-[13px] text-ink placeholder:text-ink-3 ${FOCUS}`} />
        </label>
      </div>
      {/* The window's banner sits under the toolbar on every view (DW-0101 to DW-0106). */}
      <BannerSlot />

      {p.filter === 'problems' && problems > 0 && (
        <div role="status" data-bar="cant-sync" className="flex flex-none items-center gap-3 border-b border-line bg-paper-2 px-4 py-2.5">
          <div className="flex-1"><div className="text-[13px] font-medium text-ink">Can’t sync</div><div className="text-[12px] text-ink-3">These didn’t sync. Everything else is up to date.</div></div>
          <button type="button" className={BTN} onClick={p.onTryAll}>Try all again</button>
        </div>
      )}

      {p.rows.length === 0 ? (
        <div className="flex flex-1 flex-col items-center justify-center gap-3 px-8 text-center" data-empty="history">
          <div className="grid h-14 w-14 place-items-center rounded-2xl border border-line bg-paper-2 text-ink-3"><Icon name="clock" size={26} /></div>
          <h2 className="m-0 text-[15px] font-semibold text-ink">No changes yet</h2>
          <p className="m-0 max-w-sm text-[13px] leading-5 text-ink-3">Changes from this PC and your other devices show here for 30 days.</p>
        </div>
      ) : shown.length === 0 ? (
        <div className="flex flex-1 items-center justify-center text-[13px] text-ink-3" data-empty="matches">Nothing in the last 30 days matches.</div>
      ) : (
        <div className="min-h-0 flex-1 overflow-y-auto">
          {days.map((d) => (
            <section key={d.label} data-day={d.label}>
              <h2 className="sticky top-0 m-0 border-b border-line bg-paper px-4 py-1.5 text-[12px] font-medium text-ink-3">{d.label}</h2>
              <ul className="m-0 list-none p-0">
                {d.rows.map((r) => <Row key={r.id} row={r} onTry={p.onTry} onOpen={p.onOpen} onContext={(e, row) => { e.preventDefault(); setMenu({ x: e.clientX, y: e.clientY, row }) }} />)}
              </ul>
            </section>
          ))}
        </div>
      )}
      {menu && <Menu x={menu.x} y={menu.y} entries={MENU} label="History row" onPick={(id) => p.onMenu?.(id as 'reveal' | 'beebeeb' | 'copy', menu.row)} onClose={() => setMenu(null)} />}
    </div>
  )
}
