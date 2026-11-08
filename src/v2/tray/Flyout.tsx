// The flyout surface (DW-1201 to DW-1222): fixed 340 x 520, a status block, the Recent changes list (the only
// part that scrolls) and a 52 px footer with three buttons. Presentational: it takes a view and callbacks.

import { useEffect, useRef, type ReactNode } from 'react'
import {
  LIST_COUNT, LIST_END, LIST_LABEL, VIEW_FULL_HISTORY,
  type FlyoutCommand, type FlyoutIcon, type FlyoutRow, type FlyoutView,
} from './model'

export interface FlyoutHandlers {
  onAction: (command: FlyoutCommand) => void
  onRow: (row: FlyoutRow) => void
  onOpenBeebeeb: () => void
  onOpenExplorer: () => void
  onSettings: () => void
  onViewHistory: () => void
}

const STROKE = { fill: 'none', stroke: 'currentColor', strokeWidth: 1.5, strokeLinecap: 'round', strokeLinejoin: 'round' } as const

function Icon({ name, size = 20 }: { name: FlyoutIcon; size?: number }) {
  const p = { width: size, height: size, viewBox: '0 0 20 20', 'aria-hidden': true, ...STROKE }
  switch (name) {
    case 'check': return <svg {...p}><circle cx="10" cy="10" r="7.5" /><path d="M6.7 10.2l2.2 2.2 4.4-4.6" /></svg>
    case 'sync': return <svg {...p}><path d="M16 7.5A6.5 6.5 0 0 0 4.4 6.2M4 12.5a6.5 6.5 0 0 0 11.6 1.3" /><path d="M16.2 3.8v3.9h-3.9M3.8 16.2v-3.9h3.9" /></svg>
    case 'pause': return <svg {...p}><circle cx="10" cy="10" r="7.5" /><path d="M8.2 7.5v5M11.8 7.5v5" /></svg>
    case 'alert': return <svg {...p}><circle cx="10" cy="10" r="7.5" /><path d="M10 6v4.6M10 13.4v.1" /></svg>
    case 'lock': return <svg {...p}><rect x="4.5" y="9" width="11" height="7.5" rx="1.8" /><path d="M7 9V6.8a3 3 0 0 1 6 0V9" /></svg>
    case 'cloud-off': return <svg {...p}><path d="M6 15.5h8.2a3 3 0 0 0 .6-5.9A4.7 4.7 0 0 0 5.6 8.4 3.6 3.6 0 0 0 6 15.5z" /><path d="M3.5 3.5l13 13" /></svg>
    case 'db': return <svg {...p}><ellipse cx="10" cy="5.5" rx="5.5" ry="2.3" /><path d="M4.5 5.5v9c0 1.3 2.5 2.3 5.5 2.3s5.5-1 5.5-2.3v-9M4.5 10c0 1.3 2.5 2.3 5.5 2.3s5.5-1 5.5-2.3" /></svg>
    case 'download': return <svg {...p}><path d="M10 3.5v9M6.3 9.3L10 13l3.7-3.7M4 16.3h12" /></svg>
    case 'user': return <svg {...p}><circle cx="10" cy="7" r="3" /><path d="M4.2 16.3c.7-3 3.2-4.4 5.8-4.4s5.1 1.4 5.8 4.4" /></svg>
    case 'folder': return <svg {...p}><path d="M3.5 6.5a1.5 1.5 0 0 1 1.5-1.5h3l1.6 1.8H15a1.5 1.5 0 0 1 1.5 1.5v6A1.5 1.5 0 0 1 15 16H5a1.5 1.5 0 0 1-1.5-1.5z" /></svg>
    case 'plan': return <svg {...p}><rect x="3.5" y="5" width="13" height="10" rx="2" /><path d="M3.5 8.5h13" /></svg>
    case 'wait': return <svg {...p}><circle cx="10" cy="10" r="7.5" /><path d="M10 6v4.3l2.7 1.6" /></svg>
  }
}

const TONE_ICON: Record<FlyoutView['tone'], string> = {
  good: 'bg-green-bg text-green-text',
  busy: 'bg-paper-3 text-ink-2',
  attention: 'bg-paper-3 text-ink-2',
  neutral: 'bg-paper-3 text-ink-2',
  error: 'bg-red-bg text-red-text',
  locked: 'bg-amber-bg text-amber-deep',
}

function Bar({ percent, up, label }: { percent: number | null; up: boolean; label: string }) {
  return (
    <div
      role="progressbar" aria-label={label} aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent ?? undefined}
      className="h-1 w-full overflow-hidden rounded-full bg-paper-3"
    >
      <i className={`block h-full rounded-full ${up ? 'bg-amber' : 'bg-ink-4'} ${percent === null ? 'w-1/3 animate-pulse' : ''}`} style={percent === null ? undefined : { width: `${percent}%` }} />
    </div>
  )
}

function FileTile({ name }: { name: string }) {
  const ext = name.includes('.') ? name.split('.').pop()!.slice(0, 4).toUpperCase() : ''
  return (
    <span aria-hidden="true" className="grid size-8 flex-none place-items-center rounded-md border border-line bg-paper-2 font-mono text-[9px] text-ink-3">
      {ext || '·'}
    </span>
  )
}

function Row({ row, onRow }: { row: FlyoutRow; onRow: (row: FlyoutRow) => void }) {
  return (
    <li>
      <button
        type="button" onClick={() => onRow(row)}
        className="flex w-full items-start gap-2.5 rounded-md px-2 py-1.5 text-left hover:bg-paper-3 focus-visible:outline-2 focus-visible:outline-ink-3"
      >
        <FileTile name={row.name} />
        <span className="min-w-0 flex-1">
          <span className="block truncate text-[13px] leading-[18px] text-ink">{row.name}</span>
          <span className={`block truncate text-[12px] leading-4 ${row.failed ? 'text-red-text' : 'text-ink-3'}`}>{row.failedLine ?? row.meta}</span>
          {row.percent !== null ? <span className="mt-1 block"><Bar percent={row.percent} up={row.direction !== 'down'} label={`${row.name}, ${row.percent}%`} /></span> : null}
        </span>
        <span className="flex-none pt-0.5 font-mono text-[11px] leading-[18px] text-ink-3">{row.time}</span>
      </button>
    </li>
  )
}

function Skeleton() {
  return (
    <div className="flex items-start gap-3" aria-busy="true" aria-label="Checking" data-skeleton="true">
      <span className="size-10 flex-none animate-pulse rounded-full bg-paper-3" />
      <span className="mt-1 flex-1 space-y-2">
        <span className="block h-3.5 w-32 animate-pulse rounded bg-paper-3" />
        <span className="block h-3 w-48 animate-pulse rounded bg-paper-3" />
      </span>
    </div>
  )
}

function FooterButton({ children, disabled, onClick }: { children: ReactNode; disabled?: boolean; onClick: () => void }) {
  return (
    <button
      type="button" disabled={disabled} onClick={onClick}
      className="min-h-8 flex-1 rounded-md border border-line bg-paper px-1.5 text-[12px]! leading-4! text-ink hover:bg-paper-3 focus-visible:outline-2 focus-visible:outline-ink-3 disabled:cursor-default disabled:text-ink-4 disabled:hover:bg-paper"
    >
      {children}
    </button>
  )
}

export function Flyout({ view, handlers, scrollToEnd = false }: { view: FlyoutView; handlers: FlyoutHandlers; scrollToEnd?: boolean }) {
  const listRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (scrollToEnd && listRef.current) listRef.current.scrollTop = listRef.current.scrollHeight
  }, [scrollToEnd, view])
  const rows = view.list.kind === 'rows' ? view.list.rows : []
  const atLimit = rows.length >= 50

  return (
    <div role="dialog" aria-label="Beebeeb" data-flyout={view.kind} className="flex h-[520px] w-[340px] flex-col overflow-hidden rounded-xl border border-line bg-paper font-sans text-ink shadow-lg">
      <section aria-label="Status" className="flex-none px-4 pb-3 pt-4">
        {view.title === null ? <Skeleton /> : (
          <div className="flex items-start gap-3">
            <span className={`grid size-10 flex-none place-items-center rounded-full ${TONE_ICON[view.tone]}`}>
              {view.icon ? <Icon name={view.icon} /> : null}
            </span>
            <div className="min-w-0 flex-1">
              <h1 className="text-[15px] font-semibold leading-5 text-ink">{view.title}</h1>
              {view.line ? <p className="mt-0.5 text-[12px] leading-4 text-ink-3">{view.line}</p> : null}
              {view.reason ? <p className="mt-1 font-mono text-[11px] leading-4 text-ink-3" data-reason="true">{view.reason}</p> : null}
            </div>
          </div>
        )}
        {view.progress ? (
          <div className="mt-3 flex items-center gap-2">
            <div className="flex-1"><Bar percent={view.progress.percent} up label={view.title ?? 'Progress'} /></div>
            {view.progress.speed ? <span className="font-mono text-[11px] text-ink-3">{view.progress.speed}</span> : null}
          </div>
        ) : null}
        {view.action ? (
          <button
            type="button" onClick={() => handlers.onAction(view.action!.command)}
            className={`mt-3 h-8 rounded-md px-3 text-[13px]! leading-5! font-medium! focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ink-3 ${view.action.primary ? 'bg-amber text-ink hover:bg-amber-deep hover:text-paper' : 'border border-line bg-paper text-ink hover:bg-paper-3'}`}
          >
            {view.action.label}
          </button>
        ) : null}
        {view.reason ? (
          <button type="button" onClick={() => void navigator.clipboard?.writeText(view.reason!)} className="ml-3 mt-3 h-8 text-[12px]! text-ink-3 underline underline-offset-2">Copy details</button>
        ) : null}
      </section>

      <section aria-label={LIST_LABEL} className="flex min-h-0 flex-1 flex-col border-t border-line">
        <div className="flex flex-none items-baseline justify-between px-4 pb-1 pt-2.5">
          <h2 className="text-[12px] font-medium text-ink-2">{LIST_LABEL}</h2>
          {view.list.kind === 'rows' ? <span className="font-mono text-[11px] text-ink-4">{LIST_COUNT}</span> : null}
        </div>
        <div ref={listRef} className="min-h-0 flex-1 overflow-y-auto px-2 pb-2">
          {view.list.kind === 'rows' ? (
            <>
              <ul>{rows.map((r) => <Row key={r.id} row={r} onRow={handlers.onRow} />)}</ul>
              {atLimit ? (
                <div className="px-2 py-3 text-center" data-list-end="true">
                  <p className="text-[12px] text-ink-3">{LIST_END}</p>
                  <button type="button" onClick={handlers.onViewHistory} className="mt-2 h-8 rounded-md border border-line px-3 text-[12px]! leading-4! text-ink hover:bg-paper-3">{VIEW_FULL_HISTORY}</button>
                </div>
              ) : null}
            </>
          ) : view.list.kind === 'hidden' ? (
            <div className="px-3 py-6 text-center"><p className="text-[13px] text-ink-2">{view.list.title}</p><p className="mt-1 text-[12px] text-ink-3">{view.list.line}</p></div>
          ) : (
            <p className="px-3 py-8 text-center text-[13px] text-ink-3">{view.list.title}</p>
          )}
        </div>
      </section>

      <footer className="flex h-[52px] flex-none items-center gap-1.5 border-t border-line bg-paper-2 px-2">
        <FooterButton onClick={handlers.onOpenBeebeeb}>Open Beebeeb</FooterButton>
        <FooterButton disabled={!view.explorerEnabled} onClick={handlers.onOpenExplorer}>Open in File Explorer</FooterButton>
        <FooterButton onClick={handlers.onSettings}>Settings</FooterButton>
      </footer>
    </div>
  )
}
