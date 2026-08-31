import { useEffect, useMemo, useState } from 'react'
import {
  command,
  commandUnavailableLabel,
  formatBytes,
  loadSyncStatus,
  type FinderInstallState,
  type StorageSummary,
  type SyncStatus,
} from '../desktopApi'

type PageLink = 'versions' | 'selective-sync' | 'finder' | 'account'

function finderSetupState(installState: FinderInstallState | null) {
  if (!installState) {
    return { label: 'Loading', className: '', detail: 'Checking Finder setup.' }
  }

  const status = installState.status?.toLowerCase()
  const setupBlocked =
    Boolean(installState.last_error?.trim()) ||
    status === 'setup_blocked' ||
    status === 'blocked' ||
    status === 'failed' ||
    status === 'error'

  if (setupBlocked) {
    return {
      label: 'Setup blocked',
      className: 'error',
      detail: installState.last_error?.trim() ?? 'Finder setup needs attention.',
    }
  }

  if (installState.installed || status === 'installed' || status === 'ready') {
    return {
      label: 'Installed',
      className: 'ok',
      detail: installState.path ?? 'Finder location is installed.',
    }
  }

  return {
    label: 'Needs install',
    className: 'warn',
    detail: installState.path ?? 'Install the Finder location to browse Beebeeb files.',
  }
}

export default function Status({ onNavigate }: { onNavigate?: (page: PageLink) => void }) {
  const [status, setStatus] = useState<SyncStatus | null>(null)
  const [finderInstallState, setFinderInstallState] = useState<FinderInstallState | null>(null)
  const [finderNotice, setFinderNotice] = useState<string | null>(null)
  const [storage, setStorage] = useState<StorageSummary | null>(null)
  const [storageNotice, setStorageNotice] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    const refresh = async () => {
      const [next, finderState] = await Promise.all([
        loadSyncStatus(),
        command<FinderInstallState>('finder_location_state'),
      ])
      if (cancelled) return
      setStatus(next)
      if (finderState.ok) {
        setFinderInstallState(finderState.value)
        setFinderNotice(null)
      } else {
        setFinderInstallState(null)
        setFinderNotice(
          finderState.unsupported ? commandUnavailableLabel('finder_location_state') : finderState.reason,
        )
      }
    }
    void refresh()
    const id = window.setInterval(refresh, 3000)
    return () => {
      cancelled = true
      window.clearInterval(id)
    }
  }, [])

  const statusLoggedIn = status?.logged_in
  const statusEngine = status?.engine

  useEffect(() => {
    if (statusLoggedIn == null || statusEngine == null) return
    if (!statusLoggedIn || statusEngine !== 'running') {
      setStorage(null)
      setStorageNotice(statusLoggedIn ? 'Unlock the vault to load storage.' : 'Sign in to load storage.')
      return
    }

    let cancelled = false
    setStorageNotice(null)
    command<StorageSummary>('desktop_storage_summary').then((result) => {
      if (cancelled) return
      if (result.ok) {
        setStorage(result.value)
        return
      }
      setStorage(null)
      setStorageNotice(
        result.unsupported ? commandUnavailableLabel('desktop_storage_summary') : result.reason,
      )
    })
    return () => {
      cancelled = true
    }
  }, [statusLoggedIn, statusEngine])

  const health = useMemo(() => {
    if (!status) return { label: 'Loading', className: '', detail: 'Waiting for daemon status.' }
    if (!status.logged_in) {
      return { label: 'Signed out', className: 'warn', detail: 'Sign in to start the drive.' }
    }
    if (status.conflicts > 0) {
      return { label: 'Needs review', className: 'error', detail: 'Resolve conflicts before assuming all files are current.' }
    }
    if (status.engine === 'running' && status.syncing > 0) {
      return { label: 'Syncing', className: 'ok', detail: `${status.syncing} item${status.syncing === 1 ? '' : 's'} in the queue.` }
    }
    if (status.engine === 'running') {
      return { label: 'Up to date', className: 'ok', detail: 'Metadata and transfer loops are idle.' }
    }
    return { label: 'Locked or paused', className: 'warn', detail: 'Unlock the vault or resume sync from Account & security.' }
  }, [status])

  const storagePct =
    storage && storage.quota_bytes > 0
      ? Math.min(100, Math.round((storage.used_bytes / storage.quota_bytes) * 100))
      : 0
  const finderSetup = finderSetupState(finderInstallState)

  const openSetup = async () => {
    if (!status?.logged_in) {
      const opened = await command<void>('open_onboarding_window')
      if (!opened.ok) {
        // SPLIT PENDING — owned by task 1318: toast this action failure, leave the load failure inline.
        // eslint-disable-next-line beebeeb/no-ad-hoc-error-surface
        setStorageNotice(opened.unsupported ? commandUnavailableLabel('open_onboarding_window') : opened.reason)
      }
      return
    }
    onNavigate?.('finder')
  }

  if (!status) {
    return (
      <section className="page">
        <div className="page-header">
          <div>
            <h1 className="page-title">Drive status</h1>
            <p className="page-copy">Loading daemon status…</p>
          </div>
        </div>
      </section>
    )
  }

  return (
    <section className="page">
      <div className="page-header">
        <div>
          <h1 className="page-title">Drive status</h1>
          <p className="page-copy">
            Beebeeb Desktop is the control center. Finder is the file surface; this page shows
            whether the daemon can hydrate, upload, and keep versions safe.
          </p>
        </div>
        <span className="status-pill">
          <span className={`dot ${health.className}`} />
          {health.label}
        </span>
      </div>

      <div className="grid three">
        <div className="metric">
          <div className="metric-label">Transfer queue</div>
          <div className="metric-value">{status.syncing}</div>
          <div className="metric-detail">Hydrates and uploads waiting now</div>
        </div>
        <div className="metric">
          <div className="metric-label">Online-only</div>
          <div className="metric-value">{status.cloud_only}</div>
          <div className="metric-detail">Visible without local bytes</div>
        </div>
        <div className="metric">
          <div className="metric-label">Conflicts</div>
          <div className="metric-value">{status.conflicts}</div>
          <div className="metric-detail">Version reviews needing action</div>
        </div>
      </div>

      <div className="grid two" style={{ marginTop: 14 }}>
        <div className="panel">
          <h2 className="section-title">Sync health</h2>
          <p className="page-copy" style={{ margin: '0 0 14px' }}>
            {health.detail}
          </p>
          <div className="row">
            <div>
              <div className="row-title">Finder location</div>
              <div className="row-detail">
                <span className="status-pill" style={{ marginRight: 8 }}>
                  <span className={`dot ${finderSetup.className}`} />
                  {finderSetup.label}
                </span>
                <span className={finderSetup.label === 'Setup blocked' ? undefined : 'mono'}>
                  {finderNotice ?? finderSetup.detail}
                </span>
              </div>
            </div>
            <button className="button" onClick={() => void openSetup()}>
              {status.logged_in ? 'Open setup' : 'Sign in'}
            </button>
          </div>
          <div className="row">
            <div>
              <div className="row-title">Vault access</div>
              <div className="row-detail">
                {status.logged_in ? 'Session present' : 'No session token'} · engine {status.engine}
              </div>
            </div>
            <button className="button" onClick={() => onNavigate?.('account')}>
              Account
            </button>
          </div>
        </div>

        <div className="panel">
          <h2 className="section-title">Storage</h2>
          {storage ? (
            <>
              <div className="progress-track" aria-label={`Storage ${storagePct}% full`}>
                <div className="progress-fill" style={{ width: `${storagePct}%` }} />
              </div>
              <div className="row" style={{ marginTop: 14 }}>
                <div>
                  <div className="row-title">
                    {formatBytes(storage.used_bytes)} of {formatBytes(storage.quota_bytes)}
                  </div>
                  <div className="row-detail">
                    Cache {formatBytes(storage.cache_bytes)} · pinned {formatBytes(storage.pinned_bytes)}
                  </div>
                </div>
              </div>
            </>
          ) : (
            <div className="notice">{storageNotice ?? 'Loading storage summary…'}</div>
          )}
        </div>
      </div>

      <div className="panel" style={{ marginTop: 14 }}>
        <h2 className="section-title">Queue, errors, versions</h2>
        <div className="grid three">
          <button className="button" onClick={() => onNavigate?.('versions')}>
            Review version center
          </button>
          <button className="button" onClick={() => onNavigate?.('selective-sync')}>
            Manage offline folders
          </button>
          <button className="button" onClick={() => onNavigate?.('account')}>
            Diagnostics and lock state
          </button>
        </div>
      </div>
    </section>
  )
}
