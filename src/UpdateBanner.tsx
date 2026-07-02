/**
 * UpdateBanner — persistent top-of-window banner shown when a desktop update
 * is available. Listens for the Tauri `update-available` event emitted by the
 * Rust updater (startup + every 4 h). Calls the `install_update` IPC when the
 * user clicks "Restart to update".
 *
 * Event: "update-available" → payload { version: string; body: string }
 * IPC:   install_update()   → downloads, installs, then app.restart()
 *
 * Design rules:
 *   - Amber accent only on the primary action button.
 *   - Honest voice: no reassurances, no marketing language.
 *   - Dismissible (per-session; next event fires it again if the user ignores it).
 *   - Shows a loading state while install_update is in flight (can take seconds).
 *   - Shows an inline error if install_update fails — never fails silently.
 */

import { useEffect, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { command } from './desktopApi'

// Design tokens — must match the Windows app / tray surfaces
const T = {
  paper: 'oklch(0.985 0.004 85)',
  line: 'oklch(0.90 0.008 82)',
  line2: 'oklch(0.83 0.01 80)',
  ink: 'oklch(0.18 0.01 70)',
  ink2: 'oklch(0.34 0.012 75)',
  ink3: 'oklch(0.52 0.01 78)',
  amber: 'oklch(0.82 0.17 84)',
  amberDeep: 'oklch(0.66 0.15 72)',
  amberBg: 'oklch(0.97 0.03 92)',
  fontSans: "'Inter', 'Segoe UI', system-ui, ui-sans-serif, sans-serif",
  fontMono: "'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace",
} as const

interface UpdatePayload {
  version: string
  body: string
}

type InstallState = 'idle' | 'installing' | 'error'

export default function UpdateBanner() {
  const [update, setUpdate] = useState<UpdatePayload | null>(null)
  const [dismissed, setDismissed] = useState(false)
  const [installState, setInstallState] = useState<InstallState>('idle')
  const [installError, setInstallError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false

    const unlisten = listen<UpdatePayload>('update-available', (event) => {
      if (!cancelled) {
        // Re-show the banner if a new event arrives (e.g. user dismissed
        // and the 4-hour check fires again with the same or a newer version).
        setUpdate(event.payload)
        setDismissed(false)
        setInstallState('idle')
        setInstallError(null)
      }
    })

    return () => {
      cancelled = true
      void unlisten.then((fn) => fn())
    }
  }, [])

  const handleInstall = async () => {
    setInstallState('installing')
    setInstallError(null)

    // Race the invoke against a 30-second timeout so a CDN stall or a Rust
    // hang never leaves the banner stuck on "Installing…" forever.
    //
    // Happy path: app.restart() kills the process before either the timer or
    // the invoke resolves, so neither branch fires — the timeout is harmless.
    // The 30s window is far longer than a normal install→restart sequence, so
    // a successful install will never spuriously hit the timeout.
    let timeoutId: ReturnType<typeof setTimeout> | undefined
    const timeoutPromise = new Promise<'timeout'>((resolve) => {
      timeoutId = setTimeout(() => resolve('timeout'), 30_000)
    })

    try {
      const raceResult = await Promise.race([
        command<void>('install_update'),
        timeoutPromise,
      ])

      if (raceResult === 'timeout') {
        setInstallState('error')
        setInstallError('Install timed out — try again.')
        return
      }

      // raceResult is the command result (the invoke settled before the timer).
      // Clear the timer so it can't fire after we've already handled the result.
      clearTimeout(timeoutId)
      if (raceResult.ok) {
        // The Rust side calls app.restart() — this branch may never execute.
        // If we somehow reach it, keep showing the installing state.
        return
      }
      setInstallState('error')
      setInstallError(raceResult.reason)
    } finally {
      clearTimeout(timeoutId)
    }
  }

  if (!update || dismissed) return null

  return (
    <div
      role="status"
      aria-live="polite"
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 12,
        padding: '9px 16px',
        background: T.amberBg,
        borderBottom: `1px solid oklch(0.88 0.05 84)`,
        fontFamily: T.fontSans,
        fontSize: 12.5,
        color: T.ink,
        flexShrink: 0,
      }}
    >
      {/* Amber dot — encryption-state / primary accent */}
      <div style={{
        width: 7,
        height: 7,
        borderRadius: '50%',
        background: T.amberDeep,
        flexShrink: 0,
      }} />

      {/* Message */}
      <div style={{ flex: 1, minWidth: 0 }}>
        <span style={{ fontWeight: 600 }}>
          Version {update.version} is available.
        </span>
        {' '}
        <span style={{ color: T.ink2 }}>
          Restart to apply the update.
        </span>
        {update.body && (
          <span
            style={{
              display: 'block',
              marginTop: 2,
              fontSize: 11.5,
              color: T.ink3,
              fontFamily: T.fontMono,
              whiteSpace: 'nowrap',
              overflow: 'hidden',
              textOverflow: 'ellipsis',
            }}
          >
            {update.body}
          </span>
        )}
        {installState === 'error' && installError && (
          <span style={{
            display: 'block',
            marginTop: 4,
            fontSize: 11.5,
            color: 'oklch(0.42 0.15 25)',
            lineHeight: 1.4,
          }}>
            Install failed: {installError}
          </span>
        )}
      </div>

      {/* Primary action — amber, matches brand rule */}
      <button
        onClick={() => void handleInstall()}
        disabled={installState === 'installing'}
        aria-busy={installState === 'installing'}
        style={{
          display: 'inline-flex',
          alignItems: 'center',
          gap: 6,
          padding: '6px 13px',
          fontSize: 12,
          fontFamily: T.fontSans,
          fontWeight: 600,
          borderRadius: 6,
          border: `1px solid ${T.amberDeep}`,
          background: installState === 'installing' ? T.amberBg : T.amber,
          color: T.ink,
          cursor: installState === 'installing' ? 'not-allowed' : 'pointer',
          opacity: installState === 'installing' ? 0.7 : 1,
          whiteSpace: 'nowrap' as const,
          letterSpacing: '-0.005em',
          flexShrink: 0,
          transition: 'opacity 150ms ease',
        }}
      >
        {installState === 'installing' ? (
          <>
            <SpinnerIcon />
            Installing…
          </>
        ) : (
          'Restart to update'
        )}
      </button>

      {/* Dismiss — only available when not installing */}
      {installState !== 'installing' && (
        <button
          onClick={() => setDismissed(true)}
          aria-label="Dismiss update notification"
          style={{
            width: 22,
            height: 22,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            fontSize: 14,
            lineHeight: 1,
            color: T.ink3,
            background: 'transparent',
            border: 'none',
            borderRadius: 4,
            cursor: 'pointer',
            flexShrink: 0,
            padding: 0,
          }}
          onMouseEnter={(e) => { e.currentTarget.style.background = 'oklch(0.945 0.008 82)' }}
          onMouseLeave={(e) => { e.currentTarget.style.background = 'transparent' }}
        >
          ×
        </button>
      )}
    </div>
  )
}

// Minimal inline spinner — no external dep, matches the tray's Spinner style
function SpinnerIcon() {
  return (
    <svg
      width="12"
      height="12"
      viewBox="0 0 12 12"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      style={{ animation: 'bb-spin 0.75s linear infinite' }}
    >
      <style>{`@keyframes bb-spin { to { transform: rotate(360deg); } }`}</style>
      <path d="M6 1 A5 5 0 0 1 11 6" opacity="0.9" />
      <path d="M11 6 A5 5 0 0 1 6 11" opacity="0.6" />
      <path d="M6 11 A5 5 0 0 1 1 6" opacity="0.35" />
    </svg>
  )
}
