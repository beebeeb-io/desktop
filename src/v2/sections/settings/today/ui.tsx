// Small pieces B13's panes share: error banner, skeleton, status pill, back link, toast, speed formatting.
import { useEffect, useRef, type ReactNode } from 'react'
import { Icon } from '@beebeeb/shared/design'
import { MB, mono } from '../shell/kit'
import { useSettings, type PaneId } from '../shell/context'

export function fmtSpeed(bps: number | null | undefined): string {
  if (bps == null || bps <= 0) return '—'
  return bps >= MB ? `${(bps / MB).toFixed(1)} MB/s` : `${Math.round(bps / 1024)} KB/s`
}

export function ErrorBanner({ title, children, action }: { title: string; children?: ReactNode; action?: ReactNode }) {
  return (
    <div role="alert" className="flex items-start gap-3 rounded-[12px] border border-red/40 bg-red/10 px-4 py-3" data-testid="banner-error">
      <Icon name="info" size={16} className="mt-0.5 shrink-0 text-[color:var(--color-red-text)]" />
      <div className="min-w-0 flex-1 text-[13px] leading-5">
        <div className="font-medium text-[color:var(--color-red-text)]">{title}</div>
        {children ? <div className="text-ink-2">{children}</div> : null}
      </div>
      {action}
    </div>
  )
}

export function InfoBanner({ title, children, action }: { title: string; children?: ReactNode; action?: ReactNode }) {
  return (
    <div role="status" className="flex items-start gap-3 rounded-[12px] border border-line bg-paper-2 px-4 py-3" data-testid="banner-info">
      <Icon name="info" size={16} className="mt-0.5 shrink-0 text-ink-3" />
      <div className="min-w-0 flex-1 text-[13px] leading-5">
        <div className="font-medium text-ink">{title}</div>
        {children ? <div className="text-ink-2">{children}</div> : null}
      </div>
      {action}
    </div>
  )
}

export function Skeleton({ className = '' }: { className?: string }) {
  return <div className={`animate-pulse rounded-md bg-paper-3 ${className}`} data-testid="skeleton" />
}

export function Pill({ children, tone = 'neutral' }: { children: ReactNode; tone?: 'neutral' | 'red' }) {
  return (
    <span className={`rounded-full border px-2 py-0.5 text-[11px] leading-4 ${tone === 'red' ? 'border-red/40 text-[color:var(--color-red-text)]' : 'border-line text-ink-2'}`}>{children}</span>
  )
}

export function BackLink({ to, children }: { to: PaneId; children: ReactNode }) {
  const { go } = useSettings()
  return <a href={`?section=settings&sub=${to}`} onClick={(e) => { e.preventDefault(); go(to) }} className="mb-2 inline-block text-[13px] text-ink-2 hover:underline">{children}</a>
}

export function Toast({ children }: { children: ReactNode }) {
  return <div role="status" className="fixed bottom-6 left-1/2 z-50 -translate-x-1/2 rounded-lg border border-line bg-paper px-4 py-2 text-[13px] text-ink shadow-lg" data-testid="toast">{children}</div>
}

/** A native-looking checkbox drawn in neutral grey (never the browser's default blue). */
export function Check({ state, label, onChange }: { state: 'on' | 'mixed' | 'off'; label: string; onChange: () => void }) {
  return (
    <button type="button" role="checkbox" aria-checked={state === 'mixed' ? 'mixed' : state === 'on'} aria-label={label} onClick={onChange}
      className={`grid h-4 w-4 shrink-0 place-items-center rounded border ${state === 'off' ? 'border-ink-3 bg-paper' : 'border-ink-2 bg-ink-2 text-paper'}`}>
      {state === 'on' ? <Icon name="check" size={11} /> : state === 'mixed' ? <span className="h-0.5 w-2 bg-paper" /> : null}
    </button>
  )
}

/** Frame helper: clicks a control once on mount (a state the person reaches by acting, such as an open pop-up). */
export function AutoClick({ label, children }: { label: string; children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => { const t = setTimeout(() => ref.current?.querySelector<HTMLElement>(`[aria-label="${label}"]`)?.click(), 0); return () => clearTimeout(t) }, [label])
  return <div ref={ref} className="flex min-h-0 flex-1 flex-col">{children}</div>
}

export const monoCls = mono
