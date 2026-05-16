import { useEffect, useMemo, useState } from 'react'
import {
  command,
  commandUnavailableLabel,
  formatBytes,
  loadSyncStatus,
  type StorageSummary,
  type SyncStatus,
} from '../desktopApi'

type PageLink = 'versions' | 'selective-sync' | 'finder' | 'account'

export default function Status({ onNavigate }: { onNavigate?: (page: PageLink) => void }) {
  const [status, setStatus] = useState<SyncStatus | null>(null)
  const [storage, setStorage] = useState<StorageSummary | null>(null)
  const [storageNotice, setStorageNotice] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    const refresh = async () => {
      const next = await loadSyncStatus()
      if (!cancelled) setStatus(next)
    }
    void refresh()
    const id = window.setInterval(refresh, 3000)
    return () => {
      cancelled = true
      window.clearInterval(id)
    }
  }, [])

  useEffect(() => {
    command<StorageSummary>('desktop_storage_summary').then((result) => {
      if (result.ok) {
        setStorage(result.value)
        return
      }
      setStorageNotice(
        result.unsupported ? commandUnavailableLabel('desktop_storage_summary') : result.reason,
      )
    })
  }, [])

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
              <div className="row-detail mono">{status.sync_root ?? 'Not installed'}</div>
            </div>
            <button className="button" onClick={() => onNavigate?.('finder')}>
              Open setup
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
