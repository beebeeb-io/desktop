// Available offline (DW-0715, DW-1306, DW-0120): what stays on this PC. Presentational.
import { useLayoutEffect, useState } from 'react'
import { Icon } from '@beebeeb/shared/design'
import { FileIcon } from '@beebeeb/shared/drive-ui'
import { BTN, Menu, type MenuEntry } from '../files/ui'
import { fmtCount, fmtSize } from '../files/model'

export interface OfflineRow { id: string; name: string; isFolder: boolean; items?: number; bytes: number | null; mime?: string; status: string }
export const OFFLINE_MENU: MenuEntry[] = [
  { id: 'open', label: 'Open' },
  { id: 'reveal', label: 'Show in File Explorer' },
  { id: 'remove', label: 'Remove from this PC' },
]

export function summary(rows: OfflineRow[]): string {
  const known = rows.every((r) => r.bytes !== null)
  const n = `${rows.length} ${rows.length === 1 ? 'item' : 'items'}`
  return known ? `${n} · ${fmtSize(rows.reduce((s, r) => s + (r.bytes ?? 0), 0))}` : n
}

/** The right-click menu drawn open under a row, measured after layout (frames). */
function RowMenu({ id }: { id: string }) {
  const [pos, setPos] = useState<{ x: number; y: number } | null>(null)
  useLayoutEffect(() => {
    const r = document.querySelector(`[data-anchor-row="${id}"]`)?.getBoundingClientRect()
    if (r) setPos({ x: r.left + 280, y: r.bottom - 8 })
  }, [id])
  return pos ? <Menu x={pos.x} y={pos.y} entries={OFFLINE_MENU} label="Offline row" onPick={() => {}} /> : null
}

export function OfflineView({ rows, menuOn, onOpen, onMenu, onRemoveAll }: {
  rows: OfflineRow[]
  /** Frame hook: the right-click menu drawn open on this row id (DW-1306). */
  menuOn?: string
  onOpen?: (id: string) => void
  onMenu?: (id: 'open' | 'reveal' | 'remove', row: OfflineRow) => void
  onRemoveAll?: () => void
}) {
  const [menu, setMenu] = useState<{ x: number; y: number; row: OfflineRow } | null>(null)
  const [asking, setAsking] = useState(false)
  const anchored = menuOn ? rows.find((r) => r.id === menuOn) : undefined
  return (
    <div className="flex min-h-0 flex-1 flex-col" data-section="available-offline">
      <div className="flex h-12 flex-none items-center gap-3 border-b border-line px-4">
        <h1 className="m-0 text-[15px] font-semibold text-ink">Available offline</h1>
        {rows.length > 0 && <span className="font-mono text-[12px] text-ink-3" data-totals>{summary(rows)}</span>}
        <span className="flex-1" />
        {rows.length > 0 && (asking ? (
          <>
            <span className="text-[13px] text-ink-2">Remove every copy from this PC?</span>
            <button type="button" className={BTN} onClick={() => setAsking(false)}>Cancel</button>
            <button type="button" className={BTN} onClick={() => { setAsking(false); onRemoveAll?.() }}>Remove all</button>
          </>
        ) : <button type="button" className={BTN} onClick={() => setAsking(true)}>Remove all from this PC</button>)}
      </div>
      {rows.length === 0 ? (
        <div className="flex flex-1 flex-col items-center justify-center gap-3 px-8 text-center" data-empty="offline">
          <div className="grid h-14 w-14 place-items-center rounded-2xl border border-line bg-paper-2 text-ink-3"><Icon name="pin" size={26} /></div>
          <h2 className="m-0 text-[15px] font-semibold text-ink">Nothing available offline</h2>
          <p className="m-0 max-w-sm text-[13px] leading-5 text-ink-3">Right-click a file or folder and choose Make available offline to keep it on this PC.</p>
        </div>
      ) : (
        <>
          <p className="m-0 border-b border-line bg-paper-2 px-4 py-2.5 text-[12px] leading-5 text-ink-3" data-note>
            These stay on this PC and open without a connection. They don’t count toward your downloaded files limit.
          </p>
          <ul className="m-0 min-h-0 flex-1 list-none overflow-y-auto p-0">
            {rows.map((r) => (
              <li key={r.id} data-row={r.id} onDoubleClick={() => onOpen?.(r.id)} onContextMenu={(e) => { e.preventDefault(); setMenu({ x: e.clientX, y: e.clientY, row: r }) }}
                className={`flex h-12 items-center gap-3 border-b border-line px-4 ${menuOn === r.id ? 'bg-paper-2' : ''}`} data-anchor-row={r.id}>
                <FileIcon type={r.isFolder ? 'folder' : 'default'} />
                <span className="min-w-0 flex-1 truncate text-[13px] text-ink">{r.name}</span>
                <span className="w-44 flex-none text-right font-mono text-[12px] text-ink-3">{r.isFolder ? `${fmtCount(r.items)}${r.bytes !== null ? ` · ${fmtSize(r.bytes)}` : ''}` : fmtSize(r.bytes)}</span>
                <span className="w-24 flex-none text-right text-[12px] text-ink-3">{r.status}</span>
              </li>
            ))}
          </ul>
        </>
      )}
      {menu && <Menu x={menu.x} y={menu.y} entries={OFFLINE_MENU} label="Offline row" onPick={(id) => onMenu?.(id as 'open', menu.row)} onClose={() => setMenu(null)} />}
      {anchored && <RowMenu id={anchored.id} />}
    </div>
  )
}
