/**
 * The pieces Settings › Sync and the Windows tray popover share to show what sync is doing (0.8.15): the poll of
 * `sync_activity_summary`, the status icon, the startup check's progress bar, and the Pause / Resume control.
 * The words come from `syncActivityModel.ts`; nothing here decides a sentence.
 *
 * Colours are the toast tokens (`--ok-icon`, `--warn-icon`, `--ink-3` under a `--paper` glyph), whose contrast
 * `tests/toastContrast.test.tsx` already checks in every theme. Amber stays for encryption state and the one primary
 * action: Resume.
 */
import { useCallback, useEffect, useRef, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { loadSyncActivitySummary, setSyncPaused } from '../desktopApi'
import { ENGINE_STATUS_EVENT } from '../popoverContract'
import { formatCount, type StartupCheck, type SyncActivitySummary, type SyncActivityView } from './syncActivityModel'
import { T } from './ui'
import { currentPollWindow, pollWhileVisible, windowVisible } from './visiblePoll'

/** How often a surface re-reads the summary while it is mounted, next to its `sync_status` poll. */
export const SYNC_ACTIVITY_POLL_MS = 3000
/** An `engine-status` burst (one per tick, one per pause toggle) is read once, this long after its first event. */
const ENGINE_EVENT_COALESCE_MS = 250

/** What a failed Pause / Resume says: the app menu's own sentences (`MENU_PAUSE_FAILED_SENTENCE` in lib.rs). */
export const PAUSE_FAILED = { title: 'Couldn’t pause syncing', message: 'Beebeeb couldn’t pause syncing, so it carries on as before.' }
export const RESUME_FAILED = { title: 'Couldn’t resume syncing', message: 'Beebeeb couldn’t resume syncing, so it stays paused.' }

/**
 * The live `sync_activity_summary`: read on mount, every {@link SYNC_ACTIVITY_POLL_MS}, and shortly after each
 * `engine-status` event (a tick, or a pause toggled from the app menu, the tray menu or another window).
 * `undefined` until the first read answers, `null` when it could not be read. `refresh` reads it now; only the
 * newest read's answer is kept.
 *
 * Task 2243: the poll and the event reads happen only while the window is on screen, and showing it reads at once.
 * The tray flyout is hidden almost all the time, and each read opens the state DB.
 */
export function useSyncActivitySummary(): { summary: SyncActivitySummary | null | undefined; refresh: () => Promise<void> } {
  const [summary, setSummary] = useState<SyncActivitySummary | null | undefined>(undefined)
  const latest = useRef(0)
  const mounted = useRef(true)

  const refresh = useCallback(async () => {
    const ticket = ++latest.current
    const next = await loadSyncActivitySummary()
    if (mounted.current && ticket === latest.current) setSummary(next)
  }, [])

  useEffect(() => {
    mounted.current = true
    let pending: number | undefined
    let unlisten: (() => void) | undefined
    const win = currentPollWindow()
    const stopPoll = pollWhileVisible(refresh, SYNC_ACTIVITY_POLL_MS, win)
    // Outside Tauri (a plain browser, a test) there is no event bus: the poll alone keeps the surface current.
    void Promise.resolve()
      .then(() => listen(ENGINE_STATUS_EVENT, () => {
        if (pending !== undefined) return
        pending = window.setTimeout(() => {
          pending = undefined
          void windowVisible(win).then((visible) => {
            if (visible) void refresh()
          })
        }, ENGINE_EVENT_COALESCE_MS)
      }))
      .then((fn) => {
        if (mounted.current) unlisten = fn
        else fn()
      })
      .catch(() => undefined)
    return () => {
      mounted.current = false
      stopPoll()
      if (pending !== undefined) window.clearTimeout(pending)
      unlisten?.()
    }
  }, [refresh])

  return { summary, refresh }
}

/**
 * Pause or resume through the menu's own backend, then re-read the summary so the surface shows the new state at once.
 * `onFailure` gets the sentence to show; the pause flag is left as it was.
 */
export function useSyncPauseToggle(
  view: SyncActivityView,
  refresh: () => Promise<void>,
  onFailure: (failure: { title: string; message: string }) => void,
): { busy: boolean; toggle: () => Promise<void> } {
  const [busy, setBusy] = useState(false)
  const toggle = async () => {
    if (busy) return
    const pausing = !view.paused
    setBusy(true)
    const result = await setSyncPaused(pausing)
    await refresh()
    setBusy(false)
    if (!result.ok) onFailure(pausing ? PAUSE_FAILED : RESUME_FAILED)
  }
  return { busy, toggle }
}

const ICON_BACKGROUND: Record<SyncActivityView['tone'], string> = {
  good: 'var(--ok-icon)',
  busy: T.ink3,
  attention: 'var(--warn-icon)',
  neutral: T.ink4,
}

/** The round status icon: a check when nothing is pending, arrows while work moves, bars while paused, "!" otherwise. */
export function SyncActivityIcon({ view, size = 16 }: { view: SyncActivityView; size?: number }) {
  const glyph = size * 0.62
  const common = { width: glyph, height: glyph, viewBox: '0 0 16 16', fill: 'none', stroke: 'currentColor', strokeLinecap: 'round' as const, strokeLinejoin: 'round' as const }
  let icon
  if (view.tone === 'good') {
    icon = <svg {...common} strokeWidth="2.2"><path d="M3.5 8.5 L6.5 11.5 L12.5 4.5" /></svg>
  } else if (view.kind === 'paused') {
    icon = <svg {...common} strokeWidth="2.4"><path d="M5.5 4 L5.5 12 M10.5 4 L10.5 12" /></svg>
  } else if (view.tone === 'busy') {
    icon = (
      <svg {...common} strokeWidth="1.9">
        <path d="M12.5 6.5 A4.8 4.8 0 0 0 4 5.2 M3.5 9.5 A4.8 4.8 0 0 0 12 10.8" />
        <path d="M12.6 3.4 L12.6 6.6 L9.4 6.6 M3.4 12.6 L3.4 9.4 L6.6 9.4" />
      </svg>
    )
  } else {
    icon = (
      <svg {...common} strokeWidth="2.2">
        <path d="M8 3.8 L8 9" />
        <circle cx="8" cy="12.2" r="0.7" fill="currentColor" stroke="none" />
      </svg>
    )
  }
  return (
    <span
      aria-hidden
      style={{
        width: size,
        height: size,
        borderRadius: 999,
        background: ICON_BACKGROUND[view.tone],
        color: T.paper,
        display: 'inline-flex',
        alignItems: 'center',
        justifyContent: 'center',
        flexShrink: 0,
      }}
    >
      {icon}
    </span>
  )
}

/** The startup check's progress: a thin bar with "done of total" in mono under it. */
export function SyncCheckProgress({ check, compact = false }: { check: StartupCheck; compact?: boolean }) {
  const pct = check.total > 0 ? Math.min(100, (check.done / check.total) * 100) : 0
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginTop: compact ? 6 : 10 }}>
      <div
        role="progressbar"
        aria-label="Startup check"
        aria-valuemin={0}
        aria-valuemax={check.total}
        aria-valuenow={check.done}
        style={{ flex: 1, height: compact ? 3 : 4, borderRadius: 999, background: T.paper3, overflow: 'hidden' }}
      >
        <div style={{ width: `${pct}%`, height: '100%', borderRadius: 999, background: T.ink3 }} />
      </div>
      <span style={{ fontFamily: T.fontMono, fontSize: compact ? 10 : 10.5, color: T.ink3, whiteSpace: 'nowrap' as const }}>
        {formatCount(check.done)} / {formatCount(check.total)}
      </span>
    </div>
  )
}

/**
 * Pause syncing (secondary, outlined) or Resume syncing (the primary action, amber). Disabled while signed out and
 * while the last press is still being written.
 */
export function SyncPauseButton({ view, busy, onToggle }: { view: SyncActivityView; busy: boolean; onToggle: () => void }) {
  const resume = view.paused
  const label = busy ? (resume ? 'Resuming…' : 'Pausing…') : resume ? 'Resume syncing' : 'Pause syncing'
  const disabled = busy || !view.pausable
  return (
    <button
      type="button"
      onClick={onToggle}
      disabled={disabled}
      aria-busy={busy || undefined}
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 7,
        height: 32,
        padding: '0 14px',
        fontSize: 12.5,
        fontFamily: T.fontSans,
        fontWeight: 500,
        borderRadius: 6,
        border: `1px solid ${resume ? T.amberDeep : T.line2}`,
        background: resume ? T.amber : T.paper,
        color: T.ink,
        cursor: disabled ? 'not-allowed' : 'pointer',
        opacity: disabled ? 0.6 : 1,
        whiteSpace: 'nowrap' as const,
        flexShrink: 0,
      }}
    >
      <svg width="11" height="11" viewBox="0 0 16 16" aria-hidden fill={resume ? 'currentColor' : 'none'} stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
        {resume ? <path d="M5 3.5 L12.5 8 L5 12.5 Z" /> : <path d="M5.5 3.5 L5.5 12.5 M10.5 3.5 L10.5 12.5" />}
      </svg>
      {label}
    </button>
  )
}
