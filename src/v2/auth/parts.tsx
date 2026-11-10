// B10's building blocks: the 960 x 640 window with the brand panel, the secure fields, banners, tiles and the Windows
// Hello prompt (Windows draws it; drawn here for reference, as the board does).
import type { InputHTMLAttributes, ReactNode, Ref } from 'react'
import { BBProductLockup, Icon, type IconName } from '@beebeeb/shared/design'
import { CaptionBarView } from '../chrome/CaptionBar'
import { initials } from './model'

/**
 * Every password, recovery-phrase, TOTP and backup-code field (task 2422, crypto review rule 8): no autofill, no
 * spelling, no autocorrect. WebView2's own autofill is off in Rust too (`auth_v2::webview_autofill`).
 */
export const SECRET_FIELD = { autoComplete: 'off', spellCheck: false, autoCorrect: 'off', autoCapitalize: 'off' } as const

/**
 * Note on type in buttons and inputs: the legacy stylesheet's unlayered `button, input { font: inherit }` beats Tailwind's
 * utilities on those elements, so the size and weight sit on an inner span or on the field's wrapper.
 */

/** The first-run / sign-in window: caption bar, brand panel at the left, the step at the right. */
export function AuthWindow({ children, overlay }: { children: ReactNode; overlay?: ReactNode }) {
  return (
    <div className="relative flex h-screen flex-col overflow-hidden bg-paper font-sans text-ink" data-auth-window>
      <CaptionBarView title="Beebeeb Drive" />
      <div className="flex min-h-0 flex-1">
        <BrandPanel />
        <main className="flex min-w-0 flex-1 overflow-auto px-10 py-6">
          {/* m-auto centres the step and still scrolls from its top when it is taller than the window. */}
          <div className="m-auto w-full max-w-[344px]">{children}</div>
        </main>
      </div>
      {overlay}
    </div>
  )
}

const FACTS: { icon: IconName; text: string }[] = [
  { icon: 'lock', text: 'Encrypted on this PC before anything syncs. Beebeeb can’t read it.' },
  { icon: 'users', text: 'One Beebeeb account for Drive, Pass, VPN, Chat and Mail.' },
  { icon: 'globe', text: 'Stored in Europe.' },
]

function BrandPanel() {
  return (
    <aside className="flex w-[320px] flex-none flex-col justify-between border-r border-line bg-paper-2 px-6 py-7" aria-label="About Beebeeb Drive">
      <div>
        <BBProductLockup product="drive" size={34} />
        <p className="mt-4 text-[13px] text-ink-2">Your files, encrypted on your devices.</p>
      </div>
      <ul className="m-0 flex list-none flex-col gap-3 p-0">
        {FACTS.map((f) => (
          <li key={f.icon} data-fact className="flex gap-2.5 text-[12px] leading-[17px] text-ink-2">
            <Icon name={f.icon} size={14} className="mt-0.5 flex-none text-ink-3" />
            <span>{f.text}</span>
          </li>
        ))}
      </ul>
    </aside>
  )
}

/** The small lockup above a step's heading. */
export function StepLockup() {
  return <BBProductLockup product="drive" size={26} />
}

export function Heading({ children, sub }: { children: ReactNode; sub?: ReactNode }) {
  return (
    <div className="mb-4">
      <h1 className="m-0 text-[22px] font-semibold tracking-[-0.01em] text-ink">{children}</h1>
      {sub && <p className="mt-1.5 text-[13px] leading-[18px] text-ink-2">{sub}</p>}
    </div>
  )
}

interface FieldProps extends Omit<InputHTMLAttributes<HTMLInputElement>, 'size'> {
  label: string
  icon?: IconName
  error?: string
  mono?: boolean
  secret?: boolean
  ref?: Ref<HTMLInputElement>
}

/** A labelled field. `secret` adds the no-autofill attributes and hides the text. */
export function Field({ label, icon, error, mono, secret, id, ...input }: FieldProps) {
  const fieldId = id ?? `f-${label.toLowerCase().replace(/\W+/g, '-')}`
  return (
    <div className="mb-2.5">
      <label htmlFor={fieldId} className="mb-1 block text-[12px] font-medium text-ink-2">{label}</label>
      <div className={`flex h-8 items-center gap-2 rounded-md border bg-paper px-2.5 text-[13px] text-ink ${mono ? 'font-mono' : 'font-sans'} ${error ? 'border-red' : 'border-line'} focus-within:ring-2 focus-within:ring-amber-deep`}>
        {icon && <Icon name={icon} size={15} className="flex-none text-ink-3" />}
        <input
          id={fieldId}
          {...(secret ? { ...SECRET_FIELD, type: 'password' } : {})}
          {...input}
          aria-invalid={error ? true : undefined}
          aria-describedby={error ? `${fieldId}-error` : input['aria-describedby']}
          className="min-w-0 flex-1 border-0 bg-transparent outline-none placeholder:text-ink-4"
        />
      </div>
      {error && <p id={`${fieldId}-error`} role="alert" className="mt-1.5 text-[12px] leading-4 text-red">{error}</p>}
    </div>
  )
}

export function PrimaryButton({ children, disabled, onClick, amber = true, type = 'button' }: { children: ReactNode; disabled?: boolean; onClick?: () => void; amber?: boolean; type?: 'button' | 'submit' }) {
  return (
    <button
      type={type}
      disabled={disabled}
      onClick={onClick}
      className={`flex h-8 w-full items-center justify-center gap-2 rounded-md text-[13px] font-medium disabled:cursor-default ${
        amber ? 'bg-amber text-amber-ink disabled:opacity-50' : 'border border-line bg-paper text-ink hover:bg-paper-2 disabled:opacity-60'
      }`}
    >
      <span className="inline-flex items-center gap-2 font-sans text-[13px] font-medium">{children}</span>
    </button>
  )
}

export function SecondaryButton({ children, onClick, disabled }: { children: ReactNode; onClick?: () => void; disabled?: boolean }) {
  return <PrimaryButton amber={false} onClick={onClick} disabled={disabled}>{children}</PrimaryButton>
}

/** Links are `<a>`. */
export function TextLink({ children, onClick, href = '#' }: { children: ReactNode; onClick?: () => void; href?: string }) {
  return (
    <a href={href} onClick={(e) => { if (onClick) { e.preventDefault(); onClick() } }} className="text-[12px] text-ink underline underline-offset-2 hover:text-ink-2">
      {children}
    </a>
  )
}

type Tone = 'red' | 'neutral' | 'green' | 'amber'
const TONE: Record<Tone, string> = {
  red: 'border-red-border bg-red-bg text-ink',
  neutral: 'border-line bg-paper-2 text-ink',
  green: 'border-green-border bg-green-bg text-ink',
  amber: 'border-amber-deep/40 bg-amber-bg text-ink',
}
const TONE_ICON: Record<Tone, IconName> = { red: 'info', neutral: 'info', green: 'check', amber: 'lock' }

export function Note({ tone, children, title }: { tone: Tone; children: ReactNode; title?: string }) {
  return (
    <div role={tone === 'red' ? 'alert' : 'status'} data-note={tone} className={`mb-4 flex gap-2.5 rounded-md border px-3 py-2.5 text-[12px] leading-[17px] ${TONE[tone]}`}>
      <Icon name={TONE_ICON[tone]} size={14} className={`mt-0.5 flex-none ${tone === 'red' ? 'text-red' : tone === 'amber' ? 'text-amber-deep' : tone === 'green' ? 'text-green' : 'text-ink-3'}`} />
      <div>
        {title && <div className="font-medium">{title}</div>}
        <div className={title ? 'text-ink-2' : ''}>{children}</div>
      </div>
    </div>
  )
}

export function Divider({ children }: { children: ReactNode }) {
  return (
    <div className="my-3 flex items-center gap-3 text-[11px] text-ink-3">
      <span className="h-px flex-1 bg-line" />
      {children}
      <span className="h-px flex-1 bg-line" />
    </div>
  )
}

/** A big round tile at the top of a result screen. */
export function Tile({ icon, tone }: { icon: IconName | 'hello'; tone: 'green' | 'neutral' | 'red' | 'amber' }) {
  const bg = { green: 'bg-green-bg text-green', neutral: 'bg-paper-3 text-ink-2', red: 'bg-red-bg text-red', amber: 'bg-amber-bg text-amber-deep' }[tone]
  return (
    <div data-tile={tone} className={`mb-4 grid h-14 w-14 place-items-center rounded-full ${bg}`}>
      {icon === 'hello' ? <HelloGlyph size={28} /> : <Icon name={icon} size={26} />}
    </div>
  )
}

export function Avatar({ name, size = 40 }: { name: string; size?: number }) {
  return (
    <span aria-hidden className="grid flex-none place-items-center rounded-full bg-paper-3 font-semibold text-ink-2" style={{ width: size, height: size, fontSize: size * 0.36 }}>
      {initials(name)}
    </span>
  )
}

/** The Windows Hello face glyph (line art, currentColor). */
export function HelloGlyph({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" aria-hidden>
      <path d="M4 8V6a2 2 0 0 1 2-2h2M16 4h2a2 2 0 0 1 2 2v2M20 16v2a2 2 0 0 1-2 2h-2M8 20H6a2 2 0 0 1-2-2v-2" />
      <path d="M9 9.5v1M15 9.5v1M12 9.5v3.5h-1M9.5 15.5c1.4 1.2 3.6 1.2 5 0" />
    </svg>
  )
}

/**
 * The Windows Security dialog (Windows draws it; this is the board's reference drawing). `state` failed shows the
 * system's own retry line.
 */
export function HelloPrompt({ title = 'Making sure it’s you', reason, state = 'looking', detail }: { title?: string; reason: string; state?: 'looking' | 'failed'; detail?: string }) {
  return (
    <div className="absolute inset-0 z-50 grid place-items-center bg-win-scrim" data-system-prompt>
      <div role="dialog" aria-label="Windows Security" className="w-[456px] rounded-lg border border-win-line-2 bg-win-title font-sans text-win-ink-strong shadow-2xl">
        <div className="px-6 pb-2 pt-5">
          <div className="text-[12px] text-win-ink-3">Windows Security</div>
          <div className="mt-3 text-[20px] font-semibold">{title}</div>
          <div className="mt-2 text-[13px]">{reason}</div>
          {detail && <div className="mt-1 text-[13px] text-win-ink-3">{detail}</div>}
          <div className="my-6 flex flex-col items-center gap-3">
            <span className="text-win-accent"><HelloGlyph size={56} /></span>
            <span className="text-[13px]">{state === 'failed' ? 'Couldn’t recognise you' : 'Looking for you…'}</span>
            {state === 'failed' && <span className="text-[12px] text-win-ink-3">Try again, or use your PIN.</span>}
          </div>
          <div className="text-[13px] text-win-accent">More choices</div>
        </div>
        <div className="flex justify-end gap-2 rounded-b-lg border-t border-win-line-3 bg-win-row px-6 py-4">
          <span className="rounded border border-win-button-line-3 bg-white px-6 py-1.5 text-[13px]">Cancel</span>
        </div>
      </div>
    </div>
  )
}

/** The countdown ring (DW-A07, A10). `fraction` 1 = full. */
export function Ring({ fraction, label, dim }: { fraction: number; label: string; dim?: boolean }) {
  const r = 15
  const c = 2 * Math.PI * r
  return (
    <span className={`inline-flex items-center gap-2 font-mono text-[12px] ${dim ? 'text-ink-4' : 'text-ink-2'}`}>
      <svg width="36" height="36" viewBox="0 0 36 36" aria-hidden>
        <circle cx="18" cy="18" r={r} fill="none" stroke="var(--color-line)" strokeWidth="3" />
        <circle cx="18" cy="18" r={r} fill="none" stroke="var(--color-ink-3)" strokeWidth="3" strokeDasharray={c} strokeDashoffset={c * (1 - Math.max(0, Math.min(1, fraction)))} transform="rotate(-90 18 18)" strokeLinecap="round" />
      </svg>
      {label}
    </span>
  )
}
