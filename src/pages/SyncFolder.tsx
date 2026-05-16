import { useEffect, useState } from 'react'
import { command, commandUnavailableLabel, type FinderInstallState, type SyncStatus } from '../desktopApi'

export default function SyncFolder() {
  const [syncRoot, setSyncRoot] = useState<string | null>(null)
  const [installState, setInstallState] = useState<FinderInstallState | null>(null)
  const [busy, setBusy] = useState(false)
  const [notice, setNotice] = useState<string | null>(null)

  useEffect(() => {
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
    setNotice(result.unsupported ? commandUnavailableLabel('install_finder_location') : result.reason)
  }

  const openFinder = async () => {
    setNotice(null)
    const result = await command<void>('open_finder_location', { path: syncRoot })
    if (!result.ok) {
      setNotice(result.unsupported ? commandUnavailableLabel('open_finder_location') : result.reason)
    }
  }

  const installed = installState?.installed ?? false

  return (
    <section className="page">
      <div className="page-header">
        <div>
          <h1 className="page-title">Finder location</h1>
          <p className="page-copy">
            Install Beebeeb as the Finder drive. This is the namespace users browse; pinning
            controls local availability separately.
          </p>
        </div>
        <span className="status-pill">
          <span className={`dot ${installed ? 'ok' : 'warn'}`} />
          {installed ? 'Installed' : 'Needs install'}
        </span>
      </div>

      {notice && <div className="notice" style={{ marginBottom: 14 }}>{notice}</div>}

      <div className="grid two">
        <div className="panel">
          <h2 className="section-title">Location</h2>
          <div className="panel" style={{ background: '#faf8f5' }}>
            <div className="section-label">Finder path</div>
            <div className="mono" style={{ marginTop: 8, fontSize: 13 }}>
              {installState?.path ?? syncRoot ?? 'No location selected'}
            </div>
          </div>
          <div className="button-row" style={{ marginTop: 14 }}>
            <button className="button" onClick={() => void chooseFolderClick()} disabled={busy}>
              Choose location
            </button>
            <button className="button primary" onClick={() => void installFinder()} disabled={busy}>
              Install in Finder
            </button>
            <button className="button" onClick={() => void openFinder()} disabled={!syncRoot || busy}>
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
    </section>
  )
}
