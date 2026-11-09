// The notification-area flyout (DW-1201 to DW-1222), mounted for ?window=tray once ./enabled.ts says so.
// It reads `popover_snapshot` while it is on screen, hands it to model.ts, and draws Flyout.tsx. Every action is
// one existing command, and a refusal is said in one line; the web view holds no key, token or URL.

import { useCallback, useEffect, useState } from 'react'
import { command, popoverSnapshot, setSyncPaused, showMainAppWindow, type CommandResult } from '../../desktopApi'
import { useCapabilities, type DesktopCapabilities } from '../../capabilities'
import type { PopoverSnapshot } from '../../popoverContract'
import { pollWhileVisible } from '../../windows/visiblePoll'
import { Flyout, type FlyoutHandlers } from './Flyout'
import { flyoutView, RECENT_LIMIT, type FlyoutCommand, type FlyoutExtras, type FlyoutRow } from './model'
import { pickExtras, type DriveExtras, type DriveStatus } from '../source/extras'
import { hasTauri } from '../source'
import { hasNoPlan } from '../auth/Gate'
import { actionFailure } from '../sections/files/model'
import { openExternal } from '../external'
import { runUpdate, updateCopyFor } from '../update'
import { RESTART_COPY, type UpdateCopy } from '../updateCopy'

/** The engine fields the snapshot lacks (B02b's drive_status). Absent stays absent: the flyout leaves that part out. */
export function extrasFromStatus(status: DriveStatus | null): FlyoutExtras {
  if (!status) return {}
  return {
    metered: status.metered, stoppedFiles: status.stoppedFiles, totalFiles: status.fileCount,
    uploadBps: status.uploadBps ?? null, etaSeconds: status.etaSeconds ?? null,
  }
}

async function hideFlyout(): Promise<void> {
  if (!hasTauri()) return
  const { getCurrentWindow } = await import('@tauri-apps/api/window')
  await getCurrentWindow().hide()
}

const PRICING = 'https://beebeeb.io/pricing'
type Done = CommandResult<unknown>
const NOT_HERE: Done = { ok: false, reason: 'unsupported', unsupported: true }

/** What each action is, for its failure line ("Couldn’t pause syncing."). */
const WHAT: Record<FlyoutCommand | 'row' | 'open' | 'explorer' | 'settings', string> = {
  pause: 'pause syncing', resume: 'resume syncing', 'try-again': 'try again', 'sync-anyway': 'sync on this connection',
  review: 'open the decision', 'show-failed': 'show what didn’t sync', 'sign-in': 'open sign-in', unlock: 'open Beebeeb to unlock',
  'finder-try-again': 'add Beebeeb to File Explorer', update: 'update Beebeeb', 'add-storage': 'open beebeeb.io',
  'choose-plan': 'open beebeeb.io', row: 'show the file in File Explorer', open: 'open Beebeeb', explorer: 'open File Explorer', settings: 'open Settings',
}

/** A failed action in the flyout's words (files/model.ts' wording, one for the whole app): never the host's text. */
export function flyoutFailure(what: keyof typeof WHAT, r: Done): string | null {
  return r.ok ? null : actionFailure(WHAT[what], r.unsupported ? 'unsupported' : 'failed').text
}

/**
 * Run one flyout action: each is one command (on Windows the `popover_*` commands answer the "tray" window, M3e). It
 * resolves with the command's answer; the flyout hides only after an action that moved to another window or app.
 */
export async function runFlyoutCommand(what: FlyoutCommand, update: UpdateCopy = RESTART_COPY, caps: DesktopCapabilities | null = null): Promise<Done> {
  let r: Done
  switch (what) {
    case 'pause': return setSyncPaused(true)
    case 'resume': return setSyncPaused(false)
    case 'try-again': return command<void>('popover_retry_sync')
    // Blocked: nothing on this PC reads whether the connection is metered, nor syncs past it (G25). Never a retry
    // dressed up as "sync anyway".
    case 'sync-anyway': return NOT_HERE
    case 'finder-try-again': return command<void>('install_windows_shell_integration')
    case 'review': r = await command<void>('popover_open_window', { to: 'review' }); break
    case 'show-failed': r = await command<void>('popover_open_window', { to: 'problems' }); break
    case 'sign-in': r = await showMainAppWindow(); break
    // Windows: the window opens on its lock screen, which asks Windows Hello (DW-1211 → DW-0009).
    case 'unlock': r = await command<void>('popover_unlock'); break
    case 'update': r = await runUpdate(update, caps); break
    case 'add-storage':
    case 'choose-plan': r = await openExternal(PRICING); break
  }
  if (r.ok) await hideFlyout()
  return r
}

/** A Recent changes row: that file in File Explorer, selected in its folder (DW-1202#0 → DW-1180). A file that is no
 *  longer in the folder opens Trash in the window, where a trashed file is. */
export async function revealRow(row: FlyoutRow, path?: string): Promise<Done> {
  const id = row.id.startsWith('transfer-') && path ? `local:${path}` : row.id
  const shown = await command<void>('drive_reveal', { id })
  if (shown.ok) { await hideFlyout(); return shown }
  if (/^not-found/.test(shown.reason)) {
    const trash = await command<void>('popover_open_window', { to: 'trash' })
    if (trash.ok) await hideFlyout()
    return trash
  }
  return shown
}

/** The footer's "Open in File Explorer": the sync folder in File Explorer (DW-1180). It sends the folder the config holds,
 *  as v1 does (src/pages/SyncFolder.tsx): Rust refuses a null path on Windows, and the refusal is shown (#271 review). */
export async function openFileExplorer(): Promise<Done> {
  const root = await command<string | null>('get_sync_root')
  if (!root.ok) return root
  const r = await command<void>('open_finder_location', { path: root.value })
  if (r.ok) await hideFlyout()
  return r
}

/** No plan yet (DW-1213): the server's answer, read when the flyout shows and at most once a minute (a server call). */
const PLAN_EVERY_MS = 60_000

// Picked on first use, never at module load: a module evaluated while `window` is not the page's (a test that stubs
// it) would read `window.location` from the stub.
let engineExtras: DriveExtras | null = null
const engineExtrasNow = (): DriveExtras => (engineExtras ??= pickExtras())

export default function TrayApp() {
  const caps = useCapabilities()
  const [snapshot, setSnapshot] = useState<PopoverSnapshot | null>(null)
  const [status, setStatus] = useState<DriveStatus | null>(null)
  const [noPlan, setNoPlan] = useState(false)
  const [notice, setNotice] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    let planAt = 0
    const refresh = async () => {
      const [result, engine] = await Promise.all([popoverSnapshot(RECENT_LIMIT), engineExtrasNow().status().catch(() => null)])
      if (!cancelled && result.ok) setSnapshot(result.value)
      if (!cancelled) setStatus(engine)
      if (result.ok && result.value.account.logged_in && Date.now() - planAt >= PLAN_EVERY_MS) {
        planAt = Date.now()
        const sub = await command<Parameters<typeof hasNoPlan>[0]>('account_subscription')
        // An answer that didn't come leaves the state as it was: no plan is said only when the server says so.
        if (!cancelled && sub.ok) setNoPlan(hasNoPlan(sub.value))
      }
    }
    const stop = pollWhileVisible(refresh, 3000)
    return () => { cancelled = true; stop() }
  }, [])

  // Closes on Esc and when it loses focus; a second click on the b is the shell's.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') void hideFlyout() }
    window.addEventListener('keydown', onKey)
    let unlisten: (() => void) | undefined
    if (hasTauri()) {
      void import('@tauri-apps/api/window').then(async ({ getCurrentWindow }) => {
        unlisten = await getCurrentWindow().onFocusChanged(({ payload: focused }) => { if (!focused) void getCurrentWindow().hide() })
      })
    }
    return () => { window.removeEventListener('keydown', onKey); unlisten?.() }
  }, [])

  const update = updateCopyFor(caps)
  // Each action's answer: a refusal stays on screen as one line until the next action (or the flyout hides).
  const report = useCallback((what: Parameters<typeof flyoutFailure>[0]) => (r: Done) => setNotice(flyoutFailure(what, r)), [])
  const front = useCallback(async (): Promise<Done> => {
    const r = await command<void>('popover_open_window', { to: 'front' })
    if (r.ok) await hideFlyout()
    return r
  }, [])
  const paths = new Map((snapshot?.activity ?? []).map((a) => [a.id, a.path]))
  const handlers: FlyoutHandlers = {
    onAction: (what) => { setNotice(null); void runFlyoutCommand(what, update, caps).then(report(what)) },
    onRow: (row) => { setNotice(null); void revealRow(row, paths.get(row.id)).then(report('row')) },
    onOpenBeebeeb: () => { setNotice(null); void front().then(report('open')) },
    onOpenExplorer: () => { setNotice(null); void openFileExplorer().then(report('explorer')) },
    onSettings: () => {
      setNotice(null)
      void command<void>('show_main_app_settings').then(async (r) => { if (r.ok) await hideFlyout(); report('settings')(r) })
    },
    onViewHistory: () => {
      setNotice(null)
      void command<void>('popover_open_window', { to: 'history' }).then(async (r) => { if (r.ok) await hideFlyout(); report('open')(r) })
    },
  }

  const extras: FlyoutExtras = { checking: snapshot === null }
  const view = snapshot
    ? flyoutView(snapshot, { ...extrasFromStatus(status), noPlan, update }, Math.floor(Date.now() / 1000))
    : flyoutView(EMPTY_SNAPSHOT, extras)
  return <div className="grid h-screen place-items-center bg-paper"><Flyout view={view} handlers={handlers} notice={notice} /></div>
}

const EMPTY_SNAPSHOT: PopoverSnapshot = {
  phase: 'synced', sync_phase: 'synced', generated_at: 0,
  account: { email: null, logged_in: false, vault_unlocked: false, auth_expired: false },
  paused: false, engine: { state: 'idle', files_remaining: null, bytes_total: null, bytes_done: null, last_tick_ok_at: null },
  reason: null, finder: { setup: 'ready', reason: null, reason_line: null }, storage: null, storage_full: false,
  pending_changes: 0, conflicts: { count: 0, files: [] }, activity: [],
}
