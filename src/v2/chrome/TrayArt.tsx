// The b in the notification area and its menu (DW-1230, DW-1231, DW-1231b). Windows draws the taskbar and the
// menu; Beebeeb supplies the icons and the items. Windows does not tint notification-area icons, so there is one
// b per taskbar mode (light, dark) and a badged state is that b with its badge drawn in.

import './design.css'

export type TrayState = 'up-to-date' | 'syncing' | 'paused' | 'needs-decision' | 'stopped' | 'offline'
export type TaskbarMode = 'light' | 'dark'

export const TRAY_STATES: { state: TrayState; label: string; tooltip: string; flyouts: string }[] = [
  { state: 'up-to-date', label: 'Up to date', tooltip: 'Beebeeb: up to date', flyouts: 'Up to date · Locked · No plan · Update available · Storage almost full' },
  { state: 'syncing', label: 'Syncing', tooltip: 'Beebeeb: syncing', flyouts: 'Syncing' },
  { state: 'paused', label: 'Paused', tooltip: 'Beebeeb: paused', flyouts: 'Paused' },
  { state: 'needs-decision', label: 'Needs a decision', tooltip: 'Beebeeb: needs a decision', flyouts: 'Needs a decision' },
  { state: 'stopped', label: 'Stopped and storage full', tooltip: 'Beebeeb: stopped', flyouts: 'Stopped · Storage full · Signed out after 90 days' },
  { state: 'offline', label: 'Offline', tooltip: 'Beebeeb: no connection', flyouts: 'No connection' },
]

/** Orange: the token mix of amber-deep and red. Red: the red token. */
const ORANGE = 'color-mix(in oklch, var(--color-amber-deep) 50%, var(--color-red))'
const RED = 'var(--color-red)'

export function TrayIcon({ state, mode, size = 16 }: { state: TrayState; mode: TaskbarMode; size?: number }) {
  const ink = mode === 'dark' ? 'var(--tray-dark-ink)' : 'var(--tray-light-ink)'
  const ring = mode === 'dark' ? 'var(--tray-light-ink)' : 'var(--tray-dark-ink)'
  const badge = state === 'needs-decision' ? ORANGE : state === 'stopped' ? RED : null
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" data-tray-state={state} data-mode={mode} role="img" aria-label={TRAY_STATES.find((s) => s.state === state)?.tooltip}>
      <g opacity={state === 'offline' ? 0.45 : 1} fill="none" stroke={ink}>
        <path d="M6.5 2.5v19" strokeWidth="3" strokeLinecap="round" />
        <circle cx="12.5" cy="14.5" r="5.5" strokeWidth="3" />
      </g>
      {state === 'offline' && <path d="M3 21L21 3" stroke={ink} strokeWidth="2" strokeLinecap="round" />}
      {state === 'offline' && <path d="M3 21L21 3" stroke={ring} strokeWidth="0.6" strokeLinecap="round" opacity="0.6" />}
      {state === 'syncing' && (
        <g>
          <circle cx="18" cy="18" r="6" fill={ring} />
          <g className="origin-[18px_18px] motion-safe:animate-spin" style={{ animationDuration: '1s', animationTimingFunction: 'linear' }}>
            <path d="M18 14a4 4 0 1 1-4 4" fill="none" stroke={ink} strokeWidth="2" strokeLinecap="round" />
          </g>
        </g>
      )}
      {state === 'paused' && (
        <g>
          <circle cx="18" cy="18" r="6" fill={ring} />
          <path d="M16.2 15.3v5.4M19.8 15.3v5.4" stroke={ink} strokeWidth="2" strokeLinecap="round" />
        </g>
      )}
      {badge && (
        <g>
          <circle cx="18" cy="18" r="6.2" fill={ring} />
          <circle cx="18" cy="18" r="5" fill={badge} />
          <path d="M18 15.2v3.2M18 20.6v.1" stroke="var(--tray-dark-ink)" strokeWidth="1.8" strokeLinecap="round" />
        </g>
      )}
    </svg>
  )
}

/** The notification area as Windows draws it: a strip at the bottom of the screen with the b in it. */
export function TaskbarStrip({ state, mode }: { state: TrayState; mode: TaskbarMode }) {
  const dark = mode === 'dark'
  return (
    <div data-taskbar={mode} className={`inline-flex h-10 items-center gap-3 rounded-md px-3 ${dark ? 'bg-[var(--tray-dark-bg)] text-[var(--tray-dark-fg)]' : 'bg-[var(--tray-light-bg)] text-[var(--tray-light-fg)]'}`}>
      <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1.2" aria-hidden="true"><path d="M2 8l4-4 4 4" /></svg>
      <TrayIcon state={state} mode={mode} />
      <span className="font-mono text-[11px] leading-4">10:42</span>
    </div>
  )
}

export interface TrayMenuItem { label: string; disabled?: boolean; separator?: boolean }

export function trayMenuItems(paused: boolean, inExplorer = true): TrayMenuItem[] {
  return [
    { label: 'Open Beebeeb' },
    { label: 'Open in File Explorer', disabled: !inExplorer },
    { label: '', separator: true },
    { label: paused ? 'Resume syncing' : 'Pause syncing' },
    { label: 'Settings…' },
    { label: '', separator: true },
    { label: 'Quit Beebeeb' },
  ]
}

/** The menu of the b. Windows draws it (system greys); no shortcuts, as Windows draws tray menus. */
export function TrayMenu({ paused }: { paused: boolean }) {
  return (
    <div role="menu" aria-label="Beebeeb" data-tray-menu={paused ? 'paused' : 'running'} className="w-[240px] rounded-lg border border-[var(--tray-menu-border)] bg-[var(--tray-menu-bg)] p-1 text-[13px] leading-5 text-[var(--tray-menu-fg)] shadow-lg">
      {trayMenuItems(paused).map((item, i) => item.separator
        ? <div key={i} role="separator" className="mx-1 my-1 h-px bg-[var(--tray-menu-line)]" />
        : <div key={item.label} role="menuitem" aria-disabled={item.disabled || undefined} className={`rounded-md px-3 py-1.5 ${item.disabled ? 'text-[var(--tray-menu-off)]' : 'hover:bg-[var(--tray-menu-hover)]'}`}>{item.label}</div>)}
    </div>
  )
}
