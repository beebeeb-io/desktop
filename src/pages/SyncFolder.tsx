import { useEffect, useState } from 'react'
import {
  command,
  commandUnavailableLabel,
  loadSyncStatus,
  type DesktopPlatform,
  type FinderInstallState,
  type MacosIntegrationResetResult,
  type SyncStatus,
} from '../desktopApi'
import { finderInstallNotice, finderInstallStateAfterAttempt, finderInstallStateWhileAttempting, finderLocationButtonPlan } from '../finderInstallCard'
import { useToast } from '../windows/ui'

// Inline confirm for the destructive Finder reset — no window.confirm(). Three states,
// following the DisconnectSection precedent in windows/views/AccountView.tsx:
//   idle       → shows the "Reset Finder integration…" button
//   confirming → shows the consequences copy + Reset / Cancel pair
//   busy       → shows the in-flight copy while the IPC call runs
// The action stays GATED behind an explicit confirmation. It deliberately does not become
// a toast: a toast is an announcement, not consent, and this unregisters the Finder
// location and turns off Start at login.
type ResetPhase = 'idle' | 'confirming' | 'busy'

export default function SyncFolder() {
  const { showToast } = useToast()
  const [resetPhase, setResetPhase] = useState<ResetPhase>('idle')
  const [rootUnavailable, setRootUnavailable] = useState(false)
  const [syncRoot, setSyncRoot] = useState<string | null>(null)
  // Deliberately inline (decision D1, task 1683 slice 5): the last install attempt's failure
  // lives in this state and GATES "Open in Finder" — that button only exists once the install
  // has succeeded (`finderLocationButtonPlan(installed)` below) — so it is an error that gates a
  // control, which the house rule keeps inline. It is also the state `finder_location_state`
  // persists, so the same failure is read back after a reopen. Toasting it too is the
  // double render this task removed; tests/finderInstallOneSurface.test.tsx pins "exactly one".
  // eslint-disable-next-line beebeeb/no-ad-hoc-error-surface -- gates "Open in Finder"; see above
  const [installState, setInstallState] = useState<FinderInstallState | null>(null)
  const [platform, setPlatform] = useState<DesktopPlatform>('unknown')
  const [busy, setBusy] = useState(false)
  // Survives the split: still carries the LOAD failure for the Finder install state, which
  // must persist because the panel stays on screen without it. The folder-picker, open-folder and
  // reset failures toast. The Finder INSTALL failure does not: it gates "Open in Finder", so it is
  // the inline banner driven by `installState` (decision D1, task 1683 slice 5).
  //
  // The `setNotice(null)` resets that used to open each action handler are gone ON PURPOSE.
  // They predate the split, when the same state carried both failures and clearing it before
  // an action was correct. Now that only the LOAD failure lives here, clearing it on an
  // unrelated click would dismiss a still-true explanation of a still-degraded surface —
  // which is precisely the persistence this split exists to preserve.
  const [notice, setNotice] = useState<string | null>(null)

  useEffect(() => {
    command<DesktopPlatform>('desktop_platform').then((result) => {
      if (result.ok) setPlatform(result.value)
    })
    command<SyncStatus>('sync_status').then((result) => {
      if (result.ok) setSyncRoot(result.value.sync_root)
    })
    command<FinderInstallState>('finder_location_state').then((result) => {
      if (result.ok) setInstallState(result.value)
      else setNotice(result.unsupported ? commandUnavailableLabel('finder_location_state') : result.reason)
    })
  }, [])

  const chooseFolderClick = async () => {
    setBusy(true)
    const result = await command<string | null>('pick_sync_root')
    setBusy(false)
    if (result.ok) {
      if (result.value) {
        setSyncRoot(result.value)
        setRootUnavailable(false)
      }
      return
    }
    showToast({
      variant: 'error',
      title: 'Couldn’t open the folder picker',
      message: result.unsupported ? commandUnavailableLabel('pick_sync_root') : result.reason,
    })
  }

  // D1 (task 1683 slice 5): a failed install GATES "Open in Finder", so it is ONE inline banner
  // (`installState.last_error` below), never also a toast. The backend saves the failure and
  // returns it as a state; a rejected command (a failure that was never saved) is folded into
  // the same state, so the pane renders it from one place. The previous attempt's failure is
  // cleared while a new attempt runs. See finderInstallCard.ts.
  const installFinder = async () => {
    setBusy(true)
    setInstallState(finderInstallStateWhileAttempting)
    const result = await command<FinderInstallState>('install_finder_location', { path: syncRoot })
    setBusy(false)
    setInstallState((previous) =>
      finderInstallStateAfterAttempt(result, previous, commandUnavailableLabel('install_finder_location')),
    )
  }

  // A transient action failure that gates nothing: toast (house rule, decision D1).
  const openSystemSettings = async () => {
    const result = await command<void>('open_login_items_and_extensions_settings')
    if (!result.ok) {
      showToast({
        variant: 'error',
        title: 'Couldn’t open System Settings',
        message: result.unsupported ? commandUnavailableLabel('open_login_items_and_extensions_settings') : result.reason,
      })
    }
  }

  const openFinder = async () => {
    setBusy(true)
    const current = platform === 'macos' ? null : await loadSyncStatus()
    if (platform !== 'macos') {
      setSyncRoot(current?.sync_root ?? null)
      setRootUnavailable(current == null)
      if (!current?.sync_root) {
        setBusy(false)
        return
      }
    }
    const result = await command<void>('open_finder_location', { path: platform === 'macos' ? null : current!.sync_root })
    setBusy(false)
    if (!result.ok) {
      showToast({
        variant: 'error',
        title: 'Couldn’t open the sync folder',
        message: result.unsupported ? commandUnavailableLabel('open_finder_location') : result.reason,
      })
    }
  }

  const resetFinderIntegration = async () => {
    setResetPhase('busy')
    setBusy(true)
    const result = await command<MacosIntegrationResetResult>('reset_macos_integration')
    setBusy(false)
    setResetPhase('idle')
    if (!result.ok) {
      showToast({
        variant: 'error',
        title: 'Couldn’t reset Finder integration',
        message: result.unsupported ? commandUnavailableLabel('reset_macos_integration') : result.reason,
      })
      return
    }

    const finderState = await command<FinderInstallState>('finder_location_state')
    if (finderState.ok) setInstallState(finderState.value)
    const preserved = result.value.pending_operations_preserved
    const details = [
      'Finder integration was reset.',
      preserved > 0 ? `${preserved} queued operation${preserved === 1 ? '' : 's'} preserved.` : null,
      result.value.removed_cache_files > 0 ? `${result.value.removed_cache_files} disposable cache file${result.value.removed_cache_files === 1 ? '' : 's'} removed.` : null,
      result.value.warnings.length > 0 ? result.value.warnings.join(' ') : null,
    ]
      .filter(Boolean)
      .join(' ')
    setNotice(details)
  }

  const installed = installState?.installed ?? false
  const finderNotice = finderInstallNotice(installState)
  const isMacos = platform === 'macos'
  // Task 1670: on macOS the pane must be truthful about install state — only
  // one of "Install in Finder" / "Open in Finder" at a time. Windows/Linux
  // (Cloud Files shell integration) keep the existing always-both behavior;
  // that surface is out of scope here (see task 1670's Notes).
  const buttonPlan = isMacos ? finderLocationButtonPlan(installed) : { showInstall: true, showOpen: true }

  return (
    <section className="page">
      <div className="page-header">
        <div>
          <h1 className="page-title">Finder location</h1>
          <p className="page-copy">
            Install Beebeeb as the Finder drive. On macOS the visible location is managed by
            File Provider; local sync state stays private.
          </p>
        </div>
        <span className="status-pill">
          <span className={`dot ${installed ? 'ok' : 'warn'}`} />
          {installed ? 'Installed' : 'Needs install'}
        </span>
      </div>

      {notice && <div className="notice" style={{ marginBottom: 14 }}>{notice}</div>}
      {/* role="alert": the toast this banner replaced was a live region (windows/ui.tsx Toast), so a
          screen-reader user must still be told an install failed. A user-disabled extension is not a
          failure but a fixable state: neutral, role="status", with the System Settings action. */}
      {finderNotice?.kind === 'error' && (
        <div className="notice error" role="alert" data-error-surface="finder-install" style={{ marginBottom: 14 }}>
          {finderNotice.message}
        </div>
      )}
      {finderNotice?.kind === 'user_disabled' && (
        <div className="notice" role="status" data-finder-state="user_disabled" style={{ marginBottom: 14 }}>
          <div>{finderNotice.message}</div>
          <div className="button-row" style={{ marginTop: 10 }}>
            <button className="button" onClick={() => void openSystemSettings()} disabled={busy}>
              Open Login Items &amp; Extensions
            </button>
          </div>
        </div>
      )}

      <div className="grid two">
        <div className="panel">
          <h2 className="section-title">Location</h2>
          <div className="panel" style={{ background: 'var(--paper-2)' }}>
            <div className="section-label">{isMacos ? 'Finder location' : 'Folder path'}</div>
            <div className="mono" style={{ marginTop: 8, fontSize: 13 }}>
              {isMacos ? installState?.path ?? 'Beebeeb in Finder' : rootUnavailable ? 'Sync folder unavailable' : syncRoot ?? 'Not configured on this PC yet'}
            </div>
          </div>
          <div className="button-row" style={{ marginTop: 14 }}>
            {!isMacos && (
              <button className="button" onClick={() => void chooseFolderClick()} disabled={busy}>
                Choose location
              </button>
            )}
            {buttonPlan.showInstall && (
              <button className="button amber" onClick={() => void installFinder()} disabled={busy}>
                Install in Finder
              </button>
            )}
            {buttonPlan.showOpen && (
              <button
                className={isMacos ? 'button amber' : 'button'}
                onClick={() => void openFinder()}
                disabled={(!isMacos && !syncRoot) || busy}
              >
                Open in Finder
              </button>
            )}
          </div>
        </div>

        <div className="panel">
          <h2 className="section-title">Drive model</h2>
          <div className="row">
            <div>
              <div className="row-title">My files</div>
              <div className="row-detail">Remote-first vault namespace.</div>
            </div>
          </div>
          <div className="row">
            <div>
              <div className="row-title">Shared with me</div>
              <div className="row-detail">Shared roots appear next to owned files.</div>
            </div>
          </div>
          <div className="row">
            <div>
              <div className="row-title">Offline</div>
              <div className="row-detail">Virtual view of pinned content.</div>
            </div>
          </div>
          <div className="row">
            <div>
              <div className="row-title">Conflicts</div>
              <div className="row-detail">Items needing version review.</div>
            </div>
          </div>
        </div>
      </div>

      <div className="panel" style={{ marginTop: 14 }}>
        <h2 className="section-title">Finder repair</h2>
        <div className="row">
          <div>
            <div className="row-title">Reset Finder integration</div>
            <div className="row-detail">
              Unregisters the Finder location, turns off Start at login, clears the local socket,
              and keeps queued uploads.
            </div>
          </div>
          {resetPhase === 'idle' && (
            <button className="button" disabled={busy} onClick={() => setResetPhase('confirming')}>
              Reset Finder integration…
            </button>
          )}
          {resetPhase === 'busy' && (
            <button className="button" disabled>
              Resetting…
            </button>
          )}
        </div>
        {resetPhase === 'confirming' && (
          <div className="notice" style={{ marginTop: 12 }}>
            <div style={{ marginBottom: 10 }}>
              <strong>Reset Finder integration?</strong> Beebeeb will unregister the Finder location,
              turn off Start at login, and remove the stale local socket. Queued uploads and local
              sync state are preserved.
            </div>
            <div className="button-row">
              <button className="button danger" onClick={() => void resetFinderIntegration()}>
                Reset Finder integration
              </button>
              <button className="button" onClick={() => setResetPhase('idle')}>
                Cancel
              </button>
            </div>
          </div>
        )}
      </div>
    </section>
  )
}
