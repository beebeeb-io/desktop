import { useEffect, useState } from 'react'
import {
  command,
  commandUnavailableLabel,
  type DesktopPlatform,
  type FinderInstallState,
  type MacosIntegrationResetResult,
  type SyncStatus,
} from '../desktopApi'

export default function SyncFolder() {
  const [syncRoot, setSyncRoot] = useState<string | null>(null)
  const [installState, setInstallState] = useState<FinderInstallState | null>(null)
  const [platform, setPlatform] = useState<DesktopPlatform>('unknown')
  const [busy, setBusy] = useState(false)
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
    setNotice(null)
    const result = await command<string | null>('pick_sync_root')
    setBusy(false)
    if (result.ok) {
      if (result.value) setSyncRoot(result.value)
      return
    }
    // SPLIT PENDING — owned by task 1318: toast this action failure, leave the load failure inline.
    // eslint-disable-next-line beebeeb/no-ad-hoc-error-surface
    setNotice(result.unsupported ? commandUnavailableLabel('pick_sync_root') : result.reason)
  }

  const installFinder = async () => {
    setBusy(true)
    setNotice(null)
    const result = await command<FinderInstallState>('install_finder_location', { path: syncRoot })
    setBusy(false)
    if (result.ok) {
      setInstallState(result.value)
      return
    }
    // SPLIT PENDING — owned by task 1318: toast this action failure, leave the load failure inline.
    // eslint-disable-next-line beebeeb/no-ad-hoc-error-surface
    setNotice(result.unsupported ? commandUnavailableLabel('install_finder_location') : result.reason)
  }

  const openFinder = async () => {
    setNotice(null)
    const result = await command<void>('open_finder_location', { path: platform === 'macos' ? null : syncRoot })
    if (!result.ok) {
      // SPLIT PENDING — owned by task 1318: toast this action failure, leave the load failure inline.
      // eslint-disable-next-line beebeeb/no-ad-hoc-error-surface
      setNotice(result.unsupported ? commandUnavailableLabel('open_finder_location') : result.reason)
    }
  }

  const resetFinderIntegration = async () => {
    const confirmed = window.confirm(
      'Reset Finder integration?\n\nBeebeeb will unregister the Finder location, turn off Start at login, remove the stale local socket, and preserve queued uploads and local sync state.'
    )
    if (!confirmed) return

    setBusy(true)
    setNotice(null)
    const result = await command<MacosIntegrationResetResult>('reset_macos_integration')
    setBusy(false)
    if (!result.ok) {
      // SPLIT PENDING — owned by task 1318: toast this action failure, leave the load failure inline.
      // eslint-disable-next-line beebeeb/no-ad-hoc-error-surface
      setNotice(result.unsupported ? commandUnavailableLabel('reset_macos_integration') : result.reason)
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
  const finderLastError = installState?.last_error?.trim()
  const isMacos = platform === 'macos'

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
      {finderLastError && (
        <div className="notice error" style={{ marginBottom: 14 }}>
          {finderLastError}
        </div>
      )}

      <div className="grid two">
        <div className="panel">
          <h2 className="section-title">Location</h2>
          <div className="panel" style={{ background: '#faf8f5' }}>
            <div className="section-label">{isMacos ? 'Finder location' : 'Folder path'}</div>
            <div className="mono" style={{ marginTop: 8, fontSize: 13 }}>
              {installState?.path ?? (isMacos ? 'Beebeeb in Finder' : syncRoot ?? 'No location selected')}
            </div>
          </div>
          <div className="button-row" style={{ marginTop: 14 }}>
            {!isMacos && (
              <button className="button" onClick={() => void chooseFolderClick()} disabled={busy}>
                Choose location
              </button>
            )}
            <button className="button primary" onClick={() => void installFinder()} disabled={busy}>
              Install in Finder
            </button>
            <button className="button" onClick={() => void openFinder()} disabled={(!isMacos && !syncRoot) || busy}>
              Open in Finder
            </button>
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
          <button className="button" disabled={busy} onClick={() => void resetFinderIntegration()}>
            Reset Finder integration…
          </button>
        </div>
      </div>
    </section>
  )
}
