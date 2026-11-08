// Trash (DW-0747), its menu (DW-0748), the warning with a code (DW-0749), emptying (DW-0749b) and empty (DW-0117).
// Views only: the live section and the frames pass the state.
import { useRef, useState } from 'react'
import { Icon } from '@beebeeb/shared/design'
import { FileIcon, getFileType, type TrashEntry } from '@beebeeb/shared/drive-ui'
import { BTN, FOCUS, Modal, type MenuEntry } from '../files/ui'
import { fmtCount, fmtSize, fmtWhen } from '../files/model'

const COLS = 'grid-cols-[minmax(0,2.4fr)_minmax(130px,1fr)_96px_minmax(0,1.4fr)]'
const BTN_RED = `inline-flex h-8 items-center justify-center rounded-md bg-red px-4 text-[13px] font-medium text-white hover:brightness-95 disabled:cursor-not-allowed disabled:opacity-40 ${FOCUS}`

export const trashMenu = (): MenuEntry[] => [
  { id: 'restore', label: 'Restore' },
  { id: 'details', label: 'Details', shortcut: 'Alt+Enter', dividerAfter: true },
  { id: 'delete', label: 'Delete permanently…' },
]

export const totalBytes = (entries: TrashEntry[]) => entries.reduce((n, e) => n + (e.item.size ?? 0), 0)
const size = (b: number) => fmtSize(b).replace('.0 ', ' ')
export const countLine = (n: number, bytes: number) => `${n} ${n === 1 ? 'item' : 'items'} · ${size(bytes)}`

export interface TrashViewProps {
  entries: TrashEntry[]
  /** Overrides the sum of the rows (the server's own total). */
  bytes?: number
  now?: Date
  selected?: Set<string>
  menuOn?: string
  emptying?: { done: number; total: number } | null
  onSelect?: (id: string, o: { ctrl: boolean; shift: boolean }) => void
  onContext?: (entry: TrashEntry, x: number, y: number) => void
  onDelete?: () => void
  onPreview?: (entry: TrashEntry) => void
  onEmpty?: () => void
}

export function TrashView(p: TrashViewProps) {
  const n = p.entries.length
  const bytes = p.bytes ?? totalBytes(p.entries)
  const sel = p.selected ?? new Set<string>()
  return (
    <section aria-label="Trash" data-trash={n} className="flex min-h-0 min-w-0 flex-1 flex-col">
      <div role="toolbar" aria-label="Trash" className="flex h-12 flex-none items-center gap-3 border-b border-line px-4">
        <h1 className="m-0 text-[15px] font-semibold text-ink">Trash</h1>
        {n > 0 && <span className="text-[12px] text-ink-3"><span className="font-mono">{countLine(n, bytes)}</span>, counts toward your storage</span>}
        <span className="flex-1" />
        {n > 0 && <button type="button" className={BTN} disabled={!!p.emptying} onClick={p.onEmpty} aria-disabled={!!p.emptying} style={p.emptying ? { opacity: 0.45 } : undefined}>Empty Trash…</button>}
      </div>
      {p.emptying && (
        <div role="status" data-emptying className="mx-4 mt-3 rounded-lg border border-line bg-paper-2 px-4 py-3 text-[13px] text-ink-2">
          <p className="m-0">Emptying Trash. <span className="font-mono">{p.emptying.done}</span> of <span className="font-mono">{p.emptying.total}</span> items deleted for good. You can leave this view; it carries on.</p>
          <div role="progressbar" aria-label="Emptying Trash" aria-valuemin={0} aria-valuemax={p.emptying.total} aria-valuenow={p.emptying.done} className="mt-2 h-1 overflow-hidden rounded-full bg-paper-3">
            <span className="block h-full bg-ink-3" style={{ width: `${(p.emptying.done / Math.max(1, p.emptying.total)) * 100}%` }} />
          </div>
        </div>
      )}
      {n === 0 ? (
        <div data-empty="trash" className="flex flex-1 flex-col items-center justify-center gap-2 px-8 text-center">
          <div className="mb-1 grid h-16 w-16 place-items-center rounded-2xl border border-line bg-paper-2 text-ink-3"><Icon name="trash" size={26} /></div>
          <h2 className="m-0 text-[15px] font-semibold text-ink">Trash is empty</h2>
          <p className="m-0 max-w-[380px] text-[13px] leading-5 text-ink-2">Files you delete stay here until you delete them for good.</p>
        </div>
      ) : (
        <div role="grid" tabIndex={0} aria-label="Trash" aria-multiselectable="true" data-list-count={n} className="flex min-h-0 flex-1 flex-col focus-visible:outline-none"
          onKeyDown={(e) => { if (e.key === 'Delete' && sel.size > 0) p.onDelete?.() }}>
          <div role="row" className={`grid ${COLS} h-8 flex-none items-center gap-3 border-b border-line px-4 text-[12px] text-ink-3`}>
            {['Name', 'Deleted', 'Size', 'Where it was'].map((c) => <span key={c} role="columnheader" aria-sort={c === 'Deleted' ? 'descending' : undefined}>{c}</span>)}
          </div>
          <div className="min-h-0 flex-1 overflow-auto">
            {p.entries.map((e) => {
              const on = sel.has(e.item.id) || p.menuOn === e.item.id
              return (
                <div key={e.item.id} role="row" tabIndex={-1} aria-selected={on} data-row={e.item.id} draggable
                  onDragStart={(ev) => ev.dataTransfer.setData('application/x-bb-trash-ids', JSON.stringify(sel.has(e.item.id) ? [...sel] : [e.item.id]))}
                  onClick={(ev) => p.onSelect?.(e.item.id, { ctrl: ev.ctrlKey || ev.metaKey, shift: ev.shiftKey })}
                  onKeyDown={(ev) => { if (ev.key === ' ') { ev.preventDefault(); p.onPreview?.(e) } }}
                  onContextMenu={(ev) => { ev.preventDefault(); p.onContext?.(e, ev.clientX, ev.clientY) }}
                  className={`grid ${COLS} h-10 cursor-default items-center gap-3 px-4 text-[13px] ${on ? 'bg-paper-3' : 'hover:bg-paper-2'} focus-visible:outline-none`}>
                  <span role="gridcell" className="flex min-w-0 items-center gap-2.5"><FileIcon type={getFileType(e.item.name, e.item.isFolder)} size={22} /><span className="truncate text-ink" title={e.item.name}>{e.item.name}</span></span>
                  <span role="gridcell" className="font-mono text-[12px] text-ink-3">{fmtWhen(e.trashedAt, p.now)}</span>
                  <span role="gridcell" className="font-mono text-[12px] text-ink-3">{e.item.isFolder && e.item.size == null ? fmtCount(e.item.childCount) : fmtSize(e.item.size)}</span>
                  <span role="gridcell" className="truncate text-[12px] text-ink-2" title={e.originalPath}>{e.originalPath || 'Files'}</span>
                </div>
              )
            })}
          </div>
          <p className="m-0 flex-none border-t border-line bg-paper-2 px-4 py-2 text-[12px] leading-4 text-ink-3">Items are deleted for good after 30 days. Restoring puts an item back where it came from; if that folder is gone, into Files.</p>
        </div>
      )}
    </section>
  )
}

/** Six boxes for an authenticator code; digits only. */
function CodeBoxes({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  const ref = useRef<HTMLInputElement>(null)
  return (
    <div className="relative my-3">
      <input ref={ref} aria-label="Authenticator code" inputMode="numeric" autoComplete="off" spellCheck={false} autoCorrect="off" maxLength={6}
        value={value} onChange={(e) => onChange(e.target.value.replace(/\D/g, '').slice(0, 6))}
        className="absolute inset-0 h-full w-full cursor-text opacity-0" />
      <div aria-hidden="true" data-code-boxes={6} className="flex gap-2">
        {Array.from({ length: 6 }, (_, i) => (
          <span key={i} className={`grid h-11 w-10 place-items-center rounded-md border bg-paper font-mono text-[18px] text-ink ${i === value.length ? 'border-ink-3' : 'border-line'}`}>{value[i] ?? ''}</span>
        ))}
      </div>
    </div>
  )
}

/** DW-0749: Empty Trash, or Delete permanently for a selection, with a code (or the password on an account without
 *  two-factor). The secret goes one way, into Rust, which does the step-up itself. */
export function DeleteForGoodDialog({ title, count, bytes, factor, initialCode = '', busy, error, onConfirm, onCancel }: {
  title: string; count: number; bytes: number; factor: 'code' | 'password'; initialCode?: string; busy?: boolean; error?: string | null
  onConfirm?: (secret: { code?: string; password?: string }) => void; onCancel?: () => void
}) {
  const [code, setCode] = useState(initialCode)
  const [password, setPassword] = useState('')
  const ok = factor === 'code' ? code.length === 6 : password.length > 0
  const what = count === 1 ? '1 item' : `${count} items`
  const confirm = () => {
    if (!ok || busy) return
    onConfirm?.(factor === 'code' ? { code } : { password })
    setCode('')
    setPassword('')
  }
  return (
    <Modal title={title} onClose={onCancel} width={460}
      footer={<><span className="flex-1" /><button type="button" className={BTN} onClick={onCancel}>Cancel</button><button type="button" className={BTN_RED} disabled={!ok || busy} onClick={confirm}>{title.replace(/\?$/, '')}</button></>}>
      <p className="m-0">{what} and {size(bytes)} will be permanently deleted. They can’t be restored by you or by Beebeeb, on any device.</p>
      {factor === 'code' ? (
        <>
          <div className="mt-3 text-[12px] font-medium text-ink-2">Authenticator code</div>
          <CodeBoxes value={code} onChange={setCode} />
        </>
      ) : (
        <label className="mt-3 block">
          <span className="mb-1 block text-[12px] font-medium text-ink-2">Password</span>
          <input type="password" aria-label="Password" autoComplete="current-password" value={password} onChange={(e) => setPassword(e.target.value)} onKeyDown={(e) => { if (e.key === 'Enter') confirm() }}
            className={`h-8 w-full rounded-md border border-line bg-paper px-2.5 text-[13px] text-ink ${FOCUS}`} />
        </label>
      )}
      {error && <p role="alert" className="m-0 text-[12px] text-[color:var(--color-red-text)]">{error}</p>}
    </Modal>
  )
}
