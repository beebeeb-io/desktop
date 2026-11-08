// Settings kit for desktop v2 (B12): the page, group card, row, switch, pop-up and dialog every pane is drawn with.
// One accent: Amber only on the primary action and on encryption state. Sizes, times and codes are mono.
import { useEffect, useRef, useState, type ReactNode } from 'react'
import { BBButton, BBToggle, Icon } from '@beebeeb/shared/design'

export const mono = 'font-mono tabular-nums'

export function Page({ title, sub, action, children, back }: { title: string; sub?: string; action?: ReactNode; children: ReactNode; back?: ReactNode }) {
  return (
    <div className="mx-auto w-full max-w-[640px] px-8 py-6" data-testid="pane">
      {back}
      <div className="mb-4 flex items-center justify-between gap-4">
        <div>
          <h1 className="text-[20px] font-semibold leading-7 text-ink">{title}</h1>
          {sub ? <p className="text-[13px] text-ink-3">{sub}</p> : null}
        </div>
        {action}
      </div>
      <div className="flex flex-col gap-5">{children}</div>
    </div>
  )
}

export function Group({ title, note, children }: { title?: string; note?: ReactNode; children: ReactNode }) {
  return (
    <section>
      {title ? <h2 className="mb-1.5 px-1 text-[12px] font-medium uppercase tracking-wide text-ink-3">{title}</h2> : null}
      <div className="divide-y divide-line overflow-hidden rounded-[12px] border border-line bg-paper" data-testid="group">
        {children}
      </div>
      {note ? <p className="mt-1.5 px-1 text-[12px] leading-4 text-ink-3">{note}</p> : null}
    </section>
  )
}

interface RowProps {
  label: ReactNode
  sub?: ReactNode
  value?: ReactNode
  right?: ReactNode
  onClick?: () => void
  href?: string
  dim?: boolean
  mono?: boolean
}

export function Row({ label, sub, value, right, onClick, href, dim, mono: monoValue }: RowProps) {
  const body = (
    <>
      <div className="min-w-0 flex-1">
        <div className="text-[13px] leading-5 text-ink">{label}</div>
        {sub ? <div className="text-[12px] leading-4 text-ink-3">{sub}</div> : null}
      </div>
      {value != null ? <div className={`shrink-0 text-[13px] text-ink-2 ${monoValue ? mono : ''}`}>{value}</div> : null}
      {right}
      {onClick || href ? <Icon name="chevron-right" size={14} className="shrink-0 text-ink-4" /> : null}
    </>
  )
  const cls = `flex min-h-[44px] items-center gap-3 px-4 py-2 text-left ${dim ? 'opacity-50' : ''}`
  if (href) return <a href={href} className={`${cls} hover:bg-paper-2`}>{body}</a>
  if (onClick) return <button type="button" onClick={onClick} className={`${cls} w-full hover:bg-paper-2`}>{body}</button>
  return <div className={cls} data-testid="row">{body}</div>
}

export function Switch({ on, onChange, label, disabled }: { on: boolean; onChange?: (v: boolean) => void; label: string; disabled?: boolean }) {
  return <BBToggle on={on} onChange={onChange} disabled={disabled} aria-label={label} />
}

/** A pop-up choice (Theme, Release channel, the lock delay). Closes on pick, Esc or a click outside. */
export function Choice<T extends string>({ value, options, onPick, label, open: openProp }: {
  value: T
  options: { value: T; label: string }[]
  onPick: (v: T) => void
  label: string
  open?: boolean
}) {
  const [open, setOpen] = useState(!!openProp)
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (!open) return
    const off = (e: MouseEvent) => { if (!ref.current?.contains(e.target as Node)) setOpen(false) }
    const key = (e: KeyboardEvent) => { if (e.key === 'Escape') { e.stopPropagation(); setOpen(false) } }
    document.addEventListener('mousedown', off)
    document.addEventListener('keydown', key, true)
    return () => { document.removeEventListener('mousedown', off); document.removeEventListener('keydown', key, true) }
  }, [open])
  const current = options.find((o) => o.value === value)
  return (
    <div ref={ref} className="relative">
      <button type="button" aria-label={label} aria-haspopup="listbox" aria-expanded={open} onClick={() => setOpen((o) => !o)}
        className="flex h-7 items-center gap-1 rounded-md border border-line bg-paper px-2 text-[13px] text-ink hover:bg-paper-2">
        {current?.label ?? value}
        <Icon name="chevron-down" size={12} className="text-ink-3" />
      </button>
      {open ? (
        <ul role="listbox" aria-label={label} className="absolute right-0 top-8 z-30 min-w-[140px] rounded-lg border border-line bg-paper py-1 shadow-lg">
          {options.map((o) => (
            <li key={o.value} role="option" aria-selected={o.value === value}>
              <button type="button" onClick={() => { onPick(o.value); setOpen(false) }}
                className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[13px] text-ink hover:bg-paper-2">
                <span className="w-3">{o.value === value ? <Icon name="check" size={12} /> : null}</span>
                {o.label}
              </button>
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  )
}

export function Meter({ fraction, amber, label }: { fraction: number; amber?: boolean; label: string }) {
  const pct = Math.max(0, Math.min(1, fraction)) * 100
  return (
    <div role="progressbar" aria-label={label} aria-valuenow={Math.round(pct)} aria-valuemin={0} aria-valuemax={100} className="h-1.5 w-full overflow-hidden rounded-full bg-paper-3">
      <div className={`h-full rounded-full ${amber ? 'bg-amber' : 'bg-ink-3'}`} style={{ width: `${pct}%` }} />
    </div>
  )
}

export function Btn({ children, variant = 'default', ...rest }: { children: ReactNode; variant?: 'amber' | 'default' | 'ghost' | 'danger'; onClick?: () => void; disabled?: boolean; 'aria-label'?: string }) {
  return <BBButton variant={variant} size="sm" type="button" className="!px-2.5 !py-1 !text-[13px] leading-5" {...rest}>{children}</BBButton>
}

/** Modal dialog or alert: focus lands inside, Esc cancels, the page behind is dimmed. */
export function Dialog({ title, children, actions, onCancel, wide, labelId }: { title: string; children: ReactNode; actions: ReactNode; onCancel: () => void; wide?: boolean; labelId: string }) {
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => {
    ref.current?.querySelector<HTMLElement>('button, input, [tabindex]')?.focus()
    const key = (e: KeyboardEvent) => { if (e.key === 'Escape') { e.stopPropagation(); onCancel() } }
    document.addEventListener('keydown', key, true)
    return () => document.removeEventListener('keydown', key, true)
  }, [onCancel])
  return (
    <div className="fixed inset-0 z-40 grid place-items-center bg-ink/30" data-testid="dialog-scrim">
      <div ref={ref} role="dialog" aria-modal="true" aria-labelledby={labelId} className={`${wide ? 'w-[520px]' : 'w-[420px]'} rounded-[12px] border border-line bg-paper p-5 shadow-xl`}>
        <h2 id={labelId} className="mb-2 text-[16px] font-semibold text-ink">{title}</h2>
        <div className="text-[13px] leading-5 text-ink-2">{children}</div>
        <div className="mt-5 flex justify-end gap-2">{actions}</div>
      </div>
    </div>
  )
}

export function Slider({ min, max, value, onChange, label }: { min: number; max: number; value: number; onChange: (v: number) => void; label: string }) {
  return (
    <input type="range" aria-label={label} min={min} max={max} value={value} onChange={(e) => onChange(Number(e.target.value))}
      className="h-1.5 w-full cursor-pointer accent-[color:var(--color-ink-3)]" />
  )
}

export const GB = 1024 ** 3
export const MB = 1024 ** 2
export function fmtBytes(bytes: number): string {
  if (bytes >= 1024 * GB) return `${(bytes / (1024 * GB)).toFixed(bytes % (1024 * GB) ? 1 : 0)} TB`
  if (bytes >= GB) return `${(bytes / GB).toFixed(1).replace(/\.0$/, '')} GB`
  if (bytes >= MB) return `${Math.round(bytes / MB)} MB`
  return `${Math.round(bytes / 1024)} KB`
}
