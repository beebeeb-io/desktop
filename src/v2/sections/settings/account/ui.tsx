// B14's small parts on top of B12's kit: the sub-page frame with Back (Esc), the code boxes, the password field,
// the checkup ring and the QR code. Secret fields carry autocomplete off, spellcheck off and autocorrect off.
import { useEffect, useRef, type ReactNode } from 'react'
import { BBButton, Icon } from '@beebeeb/shared/design'
import { FIXTURE_NOW } from '../../../source/fixtures/account'
import { hasTauri } from '../../../source'

const now = () => (hasTauri() ? Date.now() : FIXTURE_NOW)

export function relative(iso: string | null): string {
  if (!iso) return 'Not yet'
  const min = Math.max(0, Math.round((now() - Date.parse(iso)) / 60_000))
  if (min < 2) return 'Active now'
  if (min < 60) return `Active ${min} minutes ago`
  const h = Math.round(min / 60)
  if (h < 24) return `Active ${h} hour${h === 1 ? '' : 's'} ago`
  const d = Math.round(h / 24)
  return `Active ${d} day${d === 1 ? '' : 's'} ago`
}

export function ago(iso: string | null): string {
  if (!iso) return 'not yet'
  const d = Math.round((now() - Date.parse(iso)) / 86_400_000)
  if (d < 1) return 'today'
  if (d === 1) return 'yesterday'
  if (d < 31) return `${d} days ago`
  const m = Math.round(d / 30)
  return `${m} month${m === 1 ? '' : 's'} ago`
}

export function day(iso: string | null): string {
  if (!iso) return ''
  return new Date(iso).toLocaleDateString('en-GB', { day: 'numeric', month: 'long', year: 'numeric', timeZone: 'Europe/Amsterdam' })
}

export function dateTime(iso: string): string {
  const d = new Date(iso)
  return `${d.toLocaleDateString('en-GB', { day: 'numeric', month: 'long', timeZone: 'Europe/Amsterdam' })} at ${d.toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit', timeZone: 'Europe/Amsterdam' })}`
}

/** Esc goes back one page (the settings view's Esc, Back to Drive, only fires on a top page). */
export function useEscBack(back: () => void, active = true) {
  useEffect(() => {
    if (!active) return
    const key = (e: KeyboardEvent) => { if (e.key === 'Escape' && !e.defaultPrevented) { e.preventDefault(); e.stopPropagation(); back() } }
    document.addEventListener('keydown', key, true)
    return () => document.removeEventListener('keydown', key, true)
  }, [back, active])
}

export function BackLink({ label, onBack }: { label: string; onBack: () => void }) {
  return (
    <button type="button" onClick={onBack} className="mb-3 inline-flex items-center gap-1 text-[13px] text-ink-2 hover:text-ink">
      <Icon name="chevron-right" size={13} className="rotate-180" />{label}
    </button>
  )
}

const secretInputProps = { autoComplete: 'off', spellCheck: false, autoCorrect: 'off', autoCapitalize: 'off' } as const

/** One input labelled "N-digit code", drawn as N boxes. Digits can be pasted. */
export function CodeBoxes({ length, value, onChange, label, error, focus }: { length: number; value: string; onChange: (v: string) => void; label: string; error?: boolean; focus?: boolean }) {
  const ref = useFocus<HTMLInputElement>(focus)
  return (
    <div className="relative inline-flex gap-1.5" data-testid="code-boxes">
      {Array.from({ length }, (_, i) => (
        <span key={i} data-filled={i < value.length || undefined}
          className={`grid h-10 w-8 place-items-center rounded-md border bg-paper font-mono text-[17px] text-ink ${error ? 'border-red' : i === value.length ? 'border-ink-3' : 'border-line'} ${length === 8 && i === 4 ? 'ml-2' : ''}`}>
          {value[i] ?? ''}
        </span>
      ))}
      <input ref={ref} aria-label={label} inputMode="numeric" maxLength={length} value={value} {...secretInputProps}
        onChange={(e) => onChange(e.target.value.replace(/\D/g, '').slice(0, length))}
        className="absolute inset-0 cursor-text opacity-0" />
    </div>
  )
}

export function Field({ label, children, error, hint }: { label: string; children: ReactNode; error?: string | null; hint?: ReactNode }) {
  return (
    <label className="flex flex-col gap-1">
      <span className="text-[12px] font-medium text-ink-2">{label}</span>
      {children}
      {error ? <span role="alert" className="text-[12px] text-red">{error}</span> : hint ? <span className="text-[12px] text-ink-3">{hint}</span> : null}
    </label>
  )
}

const inputCls = (error?: boolean) =>
  `h-8 w-full rounded-md border bg-paper px-2.5 text-[13px] text-ink outline-none focus:border-ink-3 ${error ? 'border-red' : 'border-line'}`

/** Focus on mount (a page or dialog opens on its first field). */
function useFocus<T extends HTMLElement>(on?: boolean) {
  const ref = useRef<T>(null)
  useEffect(() => { if (on) ref.current?.focus() }, [on])
  return ref
}

export function TextInput({ value, onChange, error, focus, type = 'text', label }: { value: string; onChange: (v: string) => void; error?: boolean; focus?: boolean; type?: string; label: string }) {
  const ref = useFocus<HTMLInputElement>(focus)
  return <input ref={ref} aria-label={label} type={type} value={value} onChange={(e) => onChange(e.target.value)} className={inputCls(error)} />
}

/** A secure password field: never autofilled with a new password, no spellcheck. */
export function PasswordInput({ value, onChange, error, focus, label, isNew }: { value: string; onChange: (v: string) => void; error?: boolean; focus?: boolean; label: string; isNew?: boolean }) {
  const ref = useFocus<HTMLInputElement>(focus)
  return <input ref={ref} aria-label={label} type="password" value={value} onChange={(e) => onChange(e.target.value)}
    autoComplete={isNew ? 'new-password' : 'current-password'} spellCheck={false} autoCorrect="off" autoCapitalize="off" className={inputCls(error)} />
}

/** The checkup ring: hidden from assistive tech (the count is read as text). */
export function Ring({ done, total }: { done: number; total: number }) {
  const r = 22, c = 2 * Math.PI * r, f = total ? done / total : 0
  return (
    <svg width="56" height="56" viewBox="0 0 56 56" aria-hidden="true">
      <circle cx="28" cy="28" r={r} fill="none" stroke="var(--color-paper-3)" strokeWidth="5" />
      <circle cx="28" cy="28" r={r} fill="none" stroke="var(--color-ink-2)" strokeWidth="5" strokeLinecap="round"
        strokeDasharray={`${c * f} ${c}`} transform="rotate(-90 28 28)" />
      <text x="28" y="32" textAnchor="middle" className="fill-ink font-mono text-[12px]">{done}/{total}</text>
    </svg>
  )
}

/** The QR code from module rows (dark = 1), with a quiet zone. Its text alternative is the key. */
export function Qr({ rows, alt }: { rows: string[]; alt: string }) {
  const n = rows.length, q = 4
  return (
    <svg role="img" aria-label={alt} width="148" height="148" viewBox={`0 0 ${n + 2 * q} ${n + 2 * q}`} shapeRendering="crispEdges" className="rounded-md border border-line bg-white" data-testid="qr">
      {rows.flatMap((row, y) => [...row].map((m, x) => (m === '1' ? <rect key={`${x}-${y}`} x={x + q} y={y + q} width="1" height="1" fill="#111" /> : null)))}
    </svg>
  )
}

export function Pill({ children, tone = 'neutral' }: { children: ReactNode; tone?: 'neutral' | 'green' | 'red' }) {
  const cls = tone === 'green' ? 'bg-green-bg text-green-text' : tone === 'red' ? 'bg-red-bg text-red-text' : 'bg-paper-3 text-ink-2'
  return <span className={`inline-flex h-5 items-center rounded-full px-2 text-[11px] font-medium ${cls}`}>{children}</span>
}

export function Check() {
  return <Icon name="check" size={14} className="shrink-0 text-ink" aria-hidden="true" />
}

/** B12's kit button, plus the dimmed disabled state the board draws ("dimmed until six digits"). */
export function Btn({ children, variant = 'default', ...rest }: { children: ReactNode; variant?: 'amber' | 'default' | 'ghost' | 'danger'; onClick?: () => void; disabled?: boolean; 'aria-label'?: string }) {
  return (
    <BBButton variant={variant} size="sm" type="button"
      className="whitespace-nowrap !px-2.5 !py-1 !text-[13px] leading-5 disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:brightness-100" {...rest}>
      {children}
    </BBButton>
  )
}
