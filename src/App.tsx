/**
 * Beebeeb desktop — settings window shell.
 *
 * Pure React, no router. The five tabs in the left rail map 1:1 to
 * the page components in `./pages/`. Each tab renders without a route
 * change so window state (size, focus, position) stays stable as the
 * user clicks around.
 *
 * The window itself is created by Tauri (see Task 8 / lib.rs) — once
 * the rust-engineer side lands, this component runs inside a 680×540
 * non-resizable native window opened from the tray "Open Settings"
 * action.
 *
 * See docs/superpowers/plans/2026-05-07-desktop-sync-client.md (Task 8).
 */

import { useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Status from './pages/Status'
import SyncFolder from './pages/SyncFolder'
import Bandwidth from './pages/Bandwidth'
import Notifications from './pages/Notifications'
import Account from './pages/Account'

// `invoke` is imported per the plan signature even though the shell
// itself doesn't call it directly — every page component imports it
// independently. Re-exporting here keeps the import graph stable for
// future shell-level IPC (e.g. an "Open log directory" footer link).
void invoke

type Page = 'status' | 'sync-folder' | 'bandwidth' | 'notifications' | 'account'

export default function App() {
  const [page, setPage] = useState<Page>('status')
  const nav = (p: Page, label: string) => (
    <button
      onClick={() => setPage(p)}
      style={{
        width: '100%',
        textAlign: 'left',
        padding: '8px 16px',
        background: page === p ? 'rgba(251,191,36,0.15)' : 'transparent',
        border: 'none',
        cursor: 'pointer',
        color: page === p ? '#b45309' : '#374151',
        fontWeight: page === p ? 600 : 400,
      }}
    >
      {label}
    </button>
  )
  return (
    <div
      style={{
        display: 'flex',
        height: '100vh',
        fontFamily: 'Inter, sans-serif',
      }}
    >
      <aside
        style={{
          width: 160,
          background: '#f9fafb',
          borderRight: '1px solid #e5e7eb',
          padding: '12px 0',
        }}
      >
        <div style={{ padding: '0 16px 12px', fontWeight: 700, fontSize: 14 }}>
          Beebeeb
        </div>
        {nav('status', 'Status')}
        {nav('sync-folder', 'Sync Folder')}
        {nav('bandwidth', 'Bandwidth')}
        {nav('notifications', 'Notifications')}
        {nav('account', 'Account')}
      </aside>
      <main style={{ flex: 1, padding: 24, overflowY: 'auto' }}>
        {page === 'status' && <Status />}
        {page === 'sync-folder' && <SyncFolder />}
        {page === 'bandwidth' && <Bandwidth />}
        {page === 'notifications' && <Notifications />}
        {page === 'account' && <Account />}
      </main>
    </div>
  )
}
