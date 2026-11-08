// The notification-area flyout (DW-1201 to DW-1222), mounted for ?window=tray once ./enabled.ts says so.
// It reads `popover_snapshot` while it is on screen, hands it to model.ts, and draws Flyout.tsx. Every action is
// one existing command; the web view holds no key, token or URL.

import { useCallback, useEffect, useState } from 'react'
import { command, popoverSnapshot, setSyncPaused, showMainAppWindow } from '../../desktopApi'
import type { PopoverSnapshot } from '../../popoverContract'
import { pollWhileVisible } from '../../windows/visiblePoll'
import { Flyout, type FlyoutHandlers } from './Flyout'
import { flyoutView, RECENT_LIMIT, type FlyoutCommand, type FlyoutExtras } from './model'
import { pickExtras, type DriveStatus } from '../source/extras'
import { hasTauri } from '../source'

/** The engine fields the snapshot lacks (B02b's drive_status). Absent stays absent: the flyout leaves that part out. */
export function extrasFromStatus(status: DriveStatus | null): FlyoutExtras {
  if (!status) return {}
  return {
    metered: status.metered, stoppedFiles: status.stoppedFiles, totalFiles: status.fileCount,
    uploadBps: status.uploadBps ?? null, etaSeconds: status.etaSeconds ?? null,
  }
}

const openLink = (url: string) => command<void>('plugin:opener|open_url', { url })

async function hideFlyout(): Promise<void> {
  if (!hasTauri()) return
  const { getCurrentWindow } = await import('@tauri-apps/api/window')
  await getCurrentWindow().hide()
}

/** Run one flyout action. Each is one command; the flyout hides after the ones that move to another window. */
export async function runFlyoutCommand(what: FlyoutCommand, snapshot: PopoverSnapshot | null): Promise<void> {
  switch (what) {
    case 'pause': await setSyncPaused(true); return
    case 'resume': await setSyncPaused(false); return
    case 'try-again':
    case 'sync-anyway': await command<void>('popover_retry_sync'); return
    case 'review': await command<void>('popover_open_window', { to: 'review' }); break
    case 'show-failed': await command<void>('popover_open_window', { to: 'front' }); break
    case 'sign-in': await showMainAppWindow(); break
    case 'unlock': await command<string>('popover_unlock'); return
    case 'finder-try-again': await command<void>('install_windows_shell_integration'); return
    case 'download-update': await command<void>('popover_open_window', { to: 'front' }); break
    case 'add-storage':
    case 'choose-plan': await openLink('https://beebeeb.io/pricing'); break
  }
  void snapshot
  await hideFlyout()
}

const engineExtras = pickExtras()

export default function TrayApp() {
  const [snapshot, setSnapshot] = useState<PopoverSnapshot | null>(null)
  const [status, setStatus] = useState<DriveStatus | null>(null)

  useEffect(() => {
    let cancelled = false
    const refresh = async () => {
      const [result, engine] = await Promise.all([popoverSnapshot(RECENT_LIMIT), engineExtras.status().catch(() => null)])
      if (!cancelled && result.ok) setSnapshot(result.value)
      if (!cancelled) setStatus(engine)
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

  const front = useCallback(async () => { await command<void>('popover_open_window', { to: 'front' }); await hideFlyout() }, [])
  const handlers: FlyoutHandlers = {
    onAction: (what) => void runFlyoutCommand(what, snapshot),
    onRow: () => void front(),
    onOpenBeebeeb: () => void front(),
    onOpenExplorer: () => void command<void>('open_finder_location', { path: null }).then(hideFlyout),
    onSettings: () => void command<void>('show_main_app_settings').then(hideFlyout),
    onViewHistory: () => void front(),
  }

  const extras: FlyoutExtras = { checking: snapshot === null }
  const view = snapshot
    ? flyoutView(snapshot, extrasFromStatus(status), Math.floor(Date.now() / 1000))
    : flyoutView(EMPTY_SNAPSHOT, extras)
  return <div className="grid h-screen place-items-center bg-paper"><Flyout view={view} handlers={handlers} /></div>
}

const EMPTY_SNAPSHOT: PopoverSnapshot = {
  phase: 'synced', sync_phase: 'synced', generated_at: 0,
  account: { email: null, logged_in: false, vault_unlocked: false, auth_expired: false },
  paused: false, engine: { state: 'idle', files_remaining: null, bytes_total: null, bytes_done: null, last_tick_ok_at: null },
  reason: null, finder: { setup: 'ready', reason: null, reason_line: null }, storage: null, storage_full: false,
  pending_changes: 0, conflicts: { count: 0, files: [] }, activity: [],
}
