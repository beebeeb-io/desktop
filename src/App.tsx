import { useEffect, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { command, loadSyncStatus, type SyncStatus } from './desktopApi'
import Status from './pages/Status'
import SyncFolder from './pages/SyncFolder'
import SelectiveSync from './pages/SelectiveSync'
import Bandwidth from './pages/Bandwidth'
import Notifications from './pages/Notifications'
import Account from './pages/Account'
import Shared from './pages/Shared'
import VersionCenter from './pages/VersionCenter'
import UpdateBanner from './UpdateBanner'

type Page =
  | 'status'
  | 'finder'
  | 'selective-sync'
  | 'shared'
  | 'versions'
  | 'account'
  | 'bandwidth'
  | 'notifications'

const COMPACT_APP_NAV_EVENT = 'compact-app:navigate'
const PAGE_IDS: ReadonlySet<string> = new Set([
  'status',
  'finder',
  'selective-sync',
  'shared',
  'versions',
  'account',
  'bandwidth',
  'notifications',
])

function pageFromString(value: string | null): Page | null {
  return value != null && PAGE_IDS.has(value) ? (value as Page) : null
}

function initialPage(): Page {
  const params = new URLSearchParams(window.location.search)
  return pageFromString(params.get('nav')) ?? 'status'
}

const NAV: Array<{ id: Page; label: string; icon: string }> = [
  { id: 'status', label: 'Status', icon: '●' },
  { id: 'finder', label: 'Finder location', icon: '⌂' },
  { id: 'selective-sync', label: 'Selective sync', icon: '↓' },
  { id: 'shared', label: 'Shared roots', icon: '↔' },
  { id: 'versions', label: 'Versions & conflicts', icon: '⎇' },
  { id: 'account', label: 'Account & security', icon: '⌘' },
  { id: 'bandwidth', label: 'Network', icon: '⇅' },
  { id: 'notifications', label: 'Notifications', icon: '!' },
]

export default function App() {
  const [page, setPage] = useState<Page>(() => initialPage())
  const [status, setStatus] = useState<SyncStatus | null>(null)
  const [version, setVersion] = useState<string | null>(null)
  const [versionCenterRefresh, setVersionCenterRefresh] = useState(0)

  useEffect(() => {
    let cancelled = false
    const refresh = async () => {
      const next = await loadSyncStatus()
      if (!cancelled) setStatus(next)
    }
    void refresh()
    const id = window.setInterval(refresh, 5000)
    return () => {
      cancelled = true
      window.clearInterval(id)
    }
  }, [])

  useEffect(() => {
    command<string>('app_version').then((result) => {
      if (result.ok) setVersion(result.value)
    })
  }, [])

  useEffect(() => {
    let cancelled = false
    const unlisten = Promise.all([
      listen('engine-conflict', () => {
        if (cancelled) return
        setPage('versions')
        setVersionCenterRefresh((value) => value + 1)
      }),
      listen('version-center-review', () => {
        if (cancelled) return
        setPage('versions')
        setVersionCenterRefresh((value) => value + 1)
      }),
      listen('version-restored', () => {
        if (cancelled) return
        setPage('versions')
        setVersionCenterRefresh((value) => value + 1)
      }),
      listen<string>(COMPACT_APP_NAV_EVENT, (event) => {
        if (cancelled) return
        const next = pageFromString(event.payload)
        if (next != null) setPage(next)
      }),
    ])
    return () => {
      cancelled = true
      void unlisten.then((callbacks) => callbacks.forEach((callback) => callback()))
    }
  }, [])

  const renderPage = () => {
    switch (page) {
      case 'status':
        return <Status onNavigate={setPage} />
      case 'finder':
        return <SyncFolder />
      case 'selective-sync':
        return <SelectiveSync />
      case 'shared':
        return <Shared />
      case 'versions':
        return <VersionCenter refreshSignal={versionCenterRefresh} />
      case 'account':
        return <Account />
      case 'bandwidth':
        return <Bandwidth />
      case 'notifications':
        return <Notifications />
    }
  }

  const locked = status?.logged_in && status.engine !== 'running'

  return (
    <div style={{ display: 'flex', flexDirection: 'column', minHeight: '100vh' }}>
      <UpdateBanner />
      <div className="app-shell" style={{ flex: 1 }}>
        <aside className="sidebar">
          <div className="brand">
            <div className="brand-mark">b</div>
            <div>
              <div className="brand-name">Beebeeb</div>
              <div className="brand-subtitle">macOS Drive</div>
            </div>
          </div>

          <div className="nav-group">
            {NAV.map((item) => (
              <button
                key={item.id}
                className={`nav-button ${page === item.id ? 'active' : ''}`}
                onClick={() => setPage(item.id)}
              >
                <span className="nav-icon">{item.icon}</span>
                <span>{item.label}</span>
              </button>
            ))}
          </div>

          <div className="sidebar-footer">
            <div className="status-pill" style={{ marginBottom: 10 }}>
              <span
                className={`dot ${status?.engine === 'running' ? 'ok' : locked ? 'warn' : ''}`}
              />
              {status?.logged_in ? (locked ? 'Needs unlock' : 'Signed in') : 'Signed out'}
            </div>
            <div>{status?.engine === 'running' ? 'Finder-backed vault' : 'Finder location not installed'}</div>
            {version && <div style={{ marginTop: 8 }}>Version {version}</div>}
          </div>
        </aside>

        <main className="content">{renderPage()}</main>
      </div>
    </div>
  )
}
