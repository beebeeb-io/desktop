/**
 * Presentational pieces of the macOS Settings window (task 1683 slice 4). No state, no
 * commands: every one of these is a function of its props, so the window's tests can expand
 * them and read the text a user would read.
 *
 * Layout rules that keep screenshot 3's two defects from coming back (spec section 8) live in
 * `macSettings.css`, next to the class names used here: a text block inside a flex row is
 * `min-width: 0`, text that can hold user data wraps with `overflow-wrap: anywhere`, grid
 * tracks are `minmax(0, 1fr)`. Colours are theme tokens only (tests/noHardcodedThemeColors).
 */
import type { KeyboardEvent, ReactNode } from 'react'

/** Line icons, 24 x 24 box, drawn with the current colour. Paths follow design/hifi (Ico and MpIco). */
const ICON_PATHS: Record<string, ReactNode> = {
  settings: (
    <>
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 0 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 0 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3h0a1.7 1.7 0 0 0 1-1.5V3a2 2 0 0 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8v0a1.7 1.7 0 0 0 1.5 1H21a2 2 0 0 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z" />
    </>
  ),
  users: (
    <>
      <path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2" />
      <circle cx="9" cy="7" r="4" />
      <path d="M22 21v-2a4 4 0 0 0-3-3.87" />
      <path d="M16 3.13a4 4 0 0 1 0 7.75" />
    </>
  ),
  refresh: (
    <>
      <path d="M20 11a8 8 0 0 0-14.5-3.5L4 9" />
      <path d="M4 4v5h5" />
      <path d="M4 13a8 8 0 0 0 14.5 3.5L20 15" />
      <path d="M20 20v-5h-5" />
    </>
  ),
  info: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 11v6" />
      <path d="M12 7.6v.01" />
    </>
  ),
  external: (
    <>
      <path d="M14 4h6v6" />
      <path d="M20 4l-9 9" />
      <path d="M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5" />
    </>
  ),
  lock: (
    <>
      <rect x="4" y="11" width="16" height="10" rx="2" />
      <path d="M8 11V7a4 4 0 0 1 8 0v4" />
    </>
  ),
  unlock: (
    <>
      <rect x="4" y="11" width="16" height="10" rx="2" />
      <path d="M8 11V7a4 4 0 0 1 7.6-1.7" />
    </>
  ),
  check: <path d="M20 6 9 17l-5-5" />,
  chevDown: <path d="m6 9 6 6 6-6" />,
  alert: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 7.5v5.5" />
      <path d="M12 16.6v.01" />
    </>
  ),
}

export function SettingsIcon({ name, size = 16, strokeWidth = 1.6 }: { name: string; size?: number; strokeWidth?: number }) {
  return (
    <svg
      className="ms-icon"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      {ICON_PATHS[name] ?? null}
    </svg>
  )
}

/** A titled card. Children are rows and notes; the last one loses its divider in CSS. */
export function SettingsGroup({ title, children }: { title?: string; children: ReactNode }) {
  return (
    <section className="ms-group" aria-label={title}>
      {title ? <h2 className="ms-group-title">{title}</h2> : null}
      <div className="ms-card">{children}</div>
    </section>
  )
}

/** label and hint on the left, the control on the right. The text column may shrink and wraps. */
export function SettingRow({
  name,
  label,
  hint,
  control,
  tall = false,
}: {
  name: string
  label: ReactNode
  hint?: ReactNode
  control?: ReactNode
  tall?: boolean
}) {
  return (
    <div className={tall ? 'ms-row ms-row--tall' : 'ms-row'} data-row={name}>
      <div className="ms-row-text">
        <div className="ms-row-label" id={`ms-${name}-label`}>
          {label}
        </div>
        {hint ? (
          <div className="ms-row-hint" id={`ms-${name}-hint`}>
            {hint}
          </div>
        ) : null}
      </div>
      {control ? <div className="ms-row-control">{control}</div> : null}
    </div>
  )
}

/**
 * A neutral switch (on: ink track, paper knob; off: paper-2 track with an ink-3 border and knob;
 * spec section 5 and the contrast pairs in section 10), not the amber shared toggle.
 */
export function Switch({
  name,
  on,
  onChange,
  disabled = false,
  hasHint = false,
}: {
  name: string
  on: boolean
  onChange: (next: boolean) => void
  disabled?: boolean
  /** The row has a hint line, so the switch is described by it. */
  hasHint?: boolean
}) {
  return (
    <button
      type="button"
      role="switch"
      className="ms-switch"
      aria-checked={on}
      aria-labelledby={`ms-${name}-label`}
      aria-describedby={hasHint ? `ms-${name}-hint` : undefined}
      disabled={disabled}
      onClick={() => onChange(!on)}
    >
      <span className="ms-switch-knob" aria-hidden="true" />
    </button>
  )
}

export function ToggleRow({
  name,
  label,
  hint,
  on,
  onChange,
  disabled,
  tall,
}: {
  name: string
  label: ReactNode
  hint?: ReactNode
  on: boolean
  onChange: (next: boolean) => void
  disabled?: boolean
  tall?: boolean
}) {
  return (
    <SettingRow
      name={name}
      label={label}
      hint={hint}
      tall={tall}
      control={<Switch name={name} on={on} onChange={onChange} disabled={disabled} hasHint={Boolean(hint)} />}
    />
  )
}

export function Btn({
  children,
  onClick,
  disabled,
  primary = false,
  danger = false,
  onKeyDown,
  ariaLabel,
}: {
  children: ReactNode
  onClick?: () => void
  disabled?: boolean
  primary?: boolean
  /** Destructive confirm: red fill, replacing the amber primary (task 1683 ruling, 2026-10-02). */
  danger?: boolean
  onKeyDown?: (event: KeyboardEvent<HTMLButtonElement>) => void
  ariaLabel?: string
}) {
  const variant = danger ? 'ms-btn--danger' : primary ? 'ms-btn--primary' : null
  return (
    <button
      type="button"
      className={variant ? `ms-btn ${variant}` : 'ms-btn'}
      onClick={onClick}
      onKeyDown={onKeyDown}
      disabled={disabled}
      aria-label={ariaLabel}
    >
      {children}
    </button>
  )
}

/** A native select: real keyboard and VoiceOver behaviour, with the design's chevron laid over it. */
export function Select({
  name,
  value,
  options,
  onChange,
  disabled,
}: {
  name: string
  value: number
  options: Array<{ value: number; label: string }>
  onChange: (value: number) => void
  disabled?: boolean
}) {
  return (
    <span className="ms-select">
      <select
        aria-labelledby={`ms-${name}-label`}
        value={value}
        disabled={disabled}
        onChange={(event) => onChange(Number(event.target.value))}
      >
        {options.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </select>
      <span className="ms-select-chevron" aria-hidden="true">
        <SettingsIcon name="chevDown" size={13} />
      </span>
    </span>
  )
}

/** A block under a row inside the same card. `alert` is a failure (once, inline); `status` is neutral. */
export function Note({
  kind,
  surface,
  title,
  children,
  reason,
  actions,
}: {
  kind: 'alert' | 'status'
  /** Names the failure's one surface, for tests that count error surfaces. */
  surface?: string
  title?: string
  children?: ReactNode
  reason?: string | null
  actions?: ReactNode
}) {
  return (
    <div
      className={kind === 'alert' ? 'ms-note ms-note--alert' : 'ms-note'}
      role={kind === 'alert' ? 'alert' : 'status'}
      data-error-surface={kind === 'alert' ? surface : undefined}
    >
      {kind === 'alert' ? (
        <span className="ms-note-glyph">
          <SettingsIcon name="alert" size={16} />
        </span>
      ) : null}
      <div className="ms-note-text">
        {title ? <div className="ms-note-title">{title}</div> : null}
        {children ? <div className="ms-note-body">{children}</div> : null}
        {reason ? <div className="ms-mono">{reason}</div> : null}
        {actions ? <div className="ms-note-actions">{actions}</div> : null}
      </div>
    </div>
  )
}
