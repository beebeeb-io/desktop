// The Open section's shared window parts: the toolbar under the window's caption bar (back, name, meta, actions),
// the details panel, the find bar and the "newer version" banner. Neutral greys; Amber only on a primary action.
import type { ReactNode } from 'react'
import { Icon } from '@beebeeb/shared/design'
import { BTN, FOCUS, IconButton } from '../files/ui'

/** Focus on mount without the autoFocus prop (dialogs and find open with focus in them). */
export const focusOnMount = (el: HTMLElement | null) => { if (el) setTimeout(() => el.focus(), 0) }

export function OpenToolbar({ name, dirty, meta, onBack, children }: { name: string; dirty?: boolean; meta?: ReactNode; onBack?: () => void; children?: ReactNode }) {
  return (
    <div data-open-toolbar className="flex h-12 flex-none items-center gap-2 border-b border-line px-3">
      <IconButton icon="chevron-right" label="Back to the folder" flip onClick={onBack} />
      <h1 className="m-0 flex min-w-0 items-center gap-2 truncate text-[15px] font-semibold text-ink">
        <span className="truncate">{name}</span>
        {dirty && <span role="img" aria-label="Unsaved changes" title="Unsaved changes" className="inline-block h-2 w-2 flex-none rounded-full bg-ink-2" />}
      </h1>
      {meta && <span className="min-w-0 flex-none truncate font-mono text-[12px] text-ink-3">{meta}</span>}
      <span className="flex-1" />
      {children}
    </div>
  )
}

export function ToolbarButton({ label, shortcut, onClick, pressed, icon, disabled }: { label: string; shortcut?: string; onClick?: () => void; pressed?: boolean; icon?: 'lock'; disabled?: boolean }) {
  return (
    <button type="button" onClick={onClick} aria-pressed={pressed} disabled={disabled} className={`${BTN} gap-1.5 px-3 ${pressed ? 'bg-paper-3' : ''} disabled:opacity-40`}>
      {icon && <Icon name={icon} size={13} />}
      {label}
      {shortcut && <span className="ml-1 font-mono text-[11px] font-normal text-ink-3">{shortcut}</span>}
    </button>
  )
}

export const MoreButton = ({ onClick }: { onClick?: () => void }) => <IconButton icon="more" label="More" onClick={onClick} />

export function DetailsRail({ title = 'Details', rows, onClose }: { title?: string; rows: [string, string][]; onClose?: () => void }) {
  return (
    <aside data-details-panel aria-label={title} className="flex w-72 flex-none flex-col border-l border-line bg-paper">
      <div className="flex h-12 flex-none items-center border-b border-line px-4">
        <h2 className="m-0 flex-1 text-[13px] font-semibold text-ink">{title}</h2>
        <button type="button" aria-label="Hide the panel (Alt+Shift+P)" title="Hide the panel (Alt+Shift+P)" onClick={onClose} className={`grid h-7 w-7 place-items-center rounded-md text-ink-3 hover:bg-paper-3 ${FOCUS}`}><Icon name="x" size={12} /></button>
      </div>
      <dl className="m-0 flex-1 overflow-auto px-4 py-3">
        {rows.map(([k, v]) => (
          <div key={k} data-fact className="flex flex-col gap-0.5 border-b border-line py-2 last:border-0">
            <dt className="text-[11px] uppercase tracking-wide text-ink-3">{k}</dt>
            <dd className="m-0 break-words font-mono text-[12px] text-ink">{v}</dd>
          </div>
        ))}
      </dl>
    </aside>
  )
}

export function NewerBanner({ text, onShow }: { text: string; onShow?: () => void }) {
  return (
    <div role="status" data-newer-banner className="flex h-10 flex-none items-center gap-3 border-b border-line bg-paper-2 px-4 text-[13px] text-ink">
      <span className="min-w-0 flex-1 truncate">{text}</span>
      <button type="button" onClick={onShow} className={`${BTN} h-7 px-3`}>Show it</button>
    </div>
  )
}

export function FindBar({ query, onQuery, position, onPrev, onNext, onDone, message }: { query: string; onQuery?: (q: string) => void; position?: string; onPrev?: () => void; onNext?: () => void; onDone?: () => void; message?: string }) {
  return (
    <div data-find-bar className="flex h-11 flex-none items-center gap-2 border-b border-line bg-paper-2 px-4">
      <input
        aria-label="Find in the document" value={query} onChange={(e) => onQuery?.(e.target.value)} ref={focusOnMount} spellCheck={false}
        onKeyDown={(e) => { if (e.key === 'Enter') (e.shiftKey ? onPrev : onNext)?.(); if (e.key === 'Escape') onDone?.() }}
        className={`h-8 w-56 rounded-md border border-line bg-paper px-3 text-[13px] text-ink ${FOCUS}`}
      />
      {message ? <span className="text-[12px] text-ink-3">{message}</span> : <span className="font-mono text-[12px] text-ink-3">{position}</span>}
      <IconButton icon="chevron-down" label="Previous match (Shift+Enter)" flip onClick={onPrev} />
      <IconButton icon="chevron-down" label="Next match (Enter)" onClick={onNext} />
      <span className="flex-1" />
      <button type="button" onClick={onDone} className={`${BTN} h-7 px-3`}>Done</button>
    </div>
  )
}
