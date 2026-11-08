// Open files: states (DW-1516..1519). The board's words; never a raw error code.
import { Icon } from '@beebeeb/shared/design'
import { BTN, FOCUS } from '../files/ui'
import { Centered } from './Viewers'
import { fmtBytes } from './model'

const tile = 'grid h-14 w-14 place-items-center rounded-xl border border-line bg-paper-2 text-ink-2'

export function DecryptingView({ name, done, total, onCancel }: { name: string; done?: number; total?: number; onCancel?: () => void }) {
  const pct = done !== undefined && total ? Math.min(100, (done / total) * 100) : null
  const mb = (n: number) => Math.round(n / 1024 ** 2)
  return (
    <Centered>
      <div data-state="decrypting" className="flex w-80 flex-col items-center gap-3">
        <div className={tile}><Icon name="lock" size={24} /></div>
        <h2 className="m-0 text-[15px] font-semibold text-ink">Decrypting on this PC</h2>
        <div className="text-[13px] text-ink-2">{name}</div>
        {pct !== null && done !== undefined && total !== undefined && <div className="font-mono text-[12px] text-ink-3">{mb(done)} of {mb(total)} MB</div>}
        <div role="progressbar" aria-label="Decrypting" aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct === null ? undefined : Math.round(pct)} className="h-1.5 w-full overflow-hidden rounded-full bg-paper-3">
          <div className={`h-full rounded-full bg-ink-2 ${pct === null ? 'w-1/3 animate-pulse' : ''}`} style={pct === null ? undefined : { width: `${pct}%` }} />
        </div>
        <button type="button" className={BTN} onClick={onCancel}>Cancel</button>
      </div>
    </Centered>
  )
}

export function TooLargeView({ name, size, free, onFreeUp, onDownloadTo }: { name: string; size: number; free?: number; onFreeUp?: () => void; onDownloadTo?: () => void }) {
  return (
    <Centered>
      <div data-state="too-large" className="flex max-w-md flex-col items-center gap-3">
        <div className={tile}><Icon name="file" size={24} /></div>
        <h2 className="m-0 text-[15px] font-semibold text-ink">Too large to preview here</h2>
        <p className="m-0 text-[13px] leading-5 text-ink-2">
          {name} is {fmtBytes(size)}. Opening it decrypts it on this PC, which needs that much free space{free !== undefined ? `; there's ${fmtBytes(free)}.` : '.'}
        </p>
        <div className="flex gap-2">
          <button type="button" className={BTN} onClick={onFreeUp}>Free up space…</button>
          <button type="button" className={BTN} onClick={onDownloadTo}>Download to…</button>
        </div>
      </div>
    </Centered>
  )
}

/** The status line for the too-large state (DW-1517). */
export function NoRoomStatus({ needed, free }: { needed: number; free: number }) {
  return (
    <footer role="status" className="flex h-7 flex-none items-center border-t border-line bg-paper-2 px-3 text-[12px] text-ink-3">
      Not enough space to decrypt · {fmtBytes(needed)} needed · {fmtBytes(free)} free on this PC
    </footer>
  )
}

export function DecryptFailedView({ onRetry, onReport }: { onRetry?: () => void; onReport?: () => void }) {
  return (
    <Centered>
      <div data-state="decrypt-failed" role="alert" className="flex max-w-md flex-col items-center gap-3">
        <div className="grid h-14 w-14 place-items-center rounded-xl border border-red-border bg-red-bg text-red-text"><Icon name="flag" size={24} /></div>
        <h2 className="m-0 text-[15px] font-semibold text-ink">Couldn{'’'}t decrypt this file</h2>
        <p className="m-0 text-[13px] leading-5 text-ink-2">Nothing changed: the copy in Beebeeb is untouched. This can happen when a download is cut off.</p>
        <div className="flex gap-2">
          <button type="button" className={BTN} onClick={onRetry}>Try again</button>
          <button type="button" className={BTN} onClick={onReport}>Report a problem</button>
        </div>
      </div>
    </Centered>
  )
}

export function OpenInAppNote({ children, onOpen }: { children: string; onOpen?: () => void }) {
  return (
    <div role="note" className="flex flex-none items-center gap-3 border-b border-line bg-paper-2 px-4 py-2 text-[13px] text-ink-2">
      <span className="min-w-0 flex-1">{children}</span>
      <button type="button" onClick={onOpen} className={`${BTN} h-7 px-3 ${FOCUS}`}>Open in another app…</button>
    </div>
  )
}
