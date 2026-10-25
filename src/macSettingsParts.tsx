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
  // The Beebeeb window's own icons (task 1683 slice 1), drawn on design/hifi/macos-two-surface.html's 20-unit grid.
  home: <path transform="scale(1.2)" d="M3.5 9.2L10 3.5l6.5 5.7v7.3a1 1 0 0 1-1 1h-3.3v-5h-4.4v5H4.5a1 1 0 0 1-1-1z" />,
  gear: (
    <g transform="scale(1.2)">
      <circle cx="10" cy="10" r="2.6" />
      <path d="M10 1.9v2.3M10 15.8v2.3M1.9 10h2.3M15.8 10h2.3M4.3 4.3l1.6 1.6M14.1 14.1l1.6 1.6M4.3 15.7l1.6-1.6M14.1 5.9l1.6-1.6" />
      <circle cx="10" cy="10" r="5.6" />
    </g>
  ),
  folder: <path transform="scale(1.2)" d="M2.5 6.2A1.7 1.7 0 0 1 4.2 4.5h3.3l1.7 1.9h6.6a1.7 1.7 0 0 1 1.7 1.7v6.7a1.7 1.7 0 0 1-1.7 1.7H4.2a1.7 1.7 0 0 1-1.7-1.7z" />,
  back: <path transform="scale(1.2)" d="M12.5 4.5L7 10l5.5 5.5" />,
  // Review, Trash, Version history and a file: slice 4, drawn on the same 20-unit grid as the hi-fi's symbols.
  review: <path transform="scale(1.2)" d="M3 11.5h4l1.4 2h3.2l1.4-2h4M3 11.5l2-7h10l2 7v4a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1z" />,
  trash: <path transform="scale(1.2)" d="M3.5 5.5h13M8 5.5V3.8h4v1.7M5.2 5.5l.8 11a1 1 0 0 0 1 .9h6a1 1 0 0 0 1-.9l.8-11" />,
  history: <path transform="scale(1.2)" d="M3.5 10a6.5 6.5 0 1 0 1.9-4.6M3.3 3.5v3.4h3.4M10 6.5V10l2.4 1.6" />,
  file: (
    <g transform="scale(1.2)">
      <path d="M5 2.5h6.5L15.5 6.5v10a1 1 0 0 1-1 1h-9.5a1 1 0 0 1-1-1v-13a1 1 0 0 1 1-1z" />
      <path d="M11.5 2.5v4h4" />
    </g>
  ),
  shield: <path transform="scale(1.2)" d="M10 2.5l6 2.3v4.6c0 3.9-2.5 6.6-6 8.1-3.5-1.5-6-4.2-6-8.1V4.8z" />,
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
export function SettingsGroup({ title, headingId, footnote, children }: { title?: string; headingId?: string; footnote?: ReactNode; children: ReactNode }) {
  return (
    <section className="ms-group" aria-label={title}>
      {title ? (
        <h2 className="ms-group-title" id={headingId} tabIndex={headingId ? -1 : undefined}>
          {title}
        </h2>
      ) : null}
      <div className="ms-card">{children}</div>
      {footnote ? <p className="ms-footnote">{footnote}</p> : null}
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
  id,
}: {
  children: ReactNode
  onClick?: () => void
  disabled?: boolean
  primary?: boolean
  /** Destructive confirm: red fill, replacing the amber primary (task 1683 ruling, 2026-10-02). */
  danger?: boolean
  onKeyDown?: (event: KeyboardEvent<HTMLButtonElement>) => void
  ariaLabel?: string
  /** So a question can move focus to its safe answer (`focusById`) when it appears. */
  id?: string
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
      id={id}
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

/**
 * A native select over named choices (Appearance, Release channel): the same control and the same look as `Select`, whose
 * values are numbers. Real keyboard and VoiceOver behaviour; the design's chevron laid over it.
 */
export function ChoiceSelect({
  name,
  value,
  options,
  onChange,
  disabled,
}: {
  name: string
  value: string
  options: ReadonlyArray<{ value: string; label: string }>
  onChange: (value: string) => void
  disabled?: boolean
}) {
  return (
    <span className="ms-select">
      <select aria-labelledby={`ms-${name}-label`} value={value} disabled={disabled} onChange={(event) => onChange(event.target.value)}>
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
  reasonWraps,
  actions,
  mono = false,
}: {
  kind: 'alert' | 'status'
  /** Names the failure's one surface, for tests that count error surfaces. */
  surface?: string
  title?: string
  /** The body is a machine value (a measurement, a path): set in the mono face, and it wraps. */
  mono?: boolean
  children?: ReactNode
  reason?: string | null
  /** The mono line wraps instead of ending in "…" (a path the person must be able to read in full). */
  reasonWraps?: boolean
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
        {children ? <div className={mono ? 'ms-note-body ms-mono ms-mono--wrap' : 'ms-note-body'}>{children}</div> : null}
        {reason ? <div className={reasonWraps ? 'ms-mono ms-mono--wrap' : 'ms-mono'}>{reason}</div> : null}
        {actions ? <div className="ms-note-actions">{actions}</div> : null}
      </div>
    </div>
  )
}
