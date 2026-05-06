/**
 * Sync status — primary settings page.
 *
 * Reads `sync_status` IPC every 3 s and surfaces the engine state
 * (running / paused / signed-out), in-flight count, conflicts, and
 * the configured sync root. The IPC shape lives in
 * `src-tauri/src/lib.rs::sync_status`; keep the SyncStatus interface
 * below in sync with the Rust serde struct.
 *
 * See docs/superpowers/plans/2026-05-07-desktop-sync-client.md (Task 9).
 */

import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'

interface SyncStatus {
  logged_in: boolean
  engine: string
  sync_root: string | null
  syncing: number
  cloud_only: number
  conflicts: number
}

export default function Status() {
  const [status, setStatus] = useState<SyncStatus | null>(null)

  useEffect(() => {
    const refresh = () =>
      invoke<SyncStatus>('sync_status').then(setStatus).catch(console.warn)
    refresh()
    const id = setInterval(refresh, 3000)
    return () => clearInterval(id)
  }, [])

  if (!status) return <p style={{ color: '#9ca3af' }}>Loading…</p>

  const stateLabel =
    status.engine === 'running'
      ? status.syncing > 0
        ? `Syncing ${status.syncing} file${status.syncing === 1 ? '' : 's'}…`
        : 'Synced'
      : status.logged_in
        ? 'Paused'
        : 'Not signed in'

  const dot = status.engine === 'running' ? '#22c55e' : '#9ca3af'

  return (
    <div>
      <h2 style={{ fontSize: 18, fontWeight: 600, marginBottom: 16 }}>Sync Status</h2>
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 10,
          marginBottom: 20,
        }}
      >
        <div
          style={{
            width: 10,
            height: 10,
            borderRadius: '50%',
            background: dot,
          }}
        />
        <span style={{ fontWeight: 500 }}>{stateLabel}</span>
      </div>
      <div
        style={{
          background: '#f3f4f6',
          borderRadius: 8,
          padding: 12,
          fontSize: 13,
        }}
      >
        <div>
          Cloud-only files: <strong>{status.cloud_only}</strong>
        </div>
        {status.conflicts > 0 && (
          <div style={{ color: '#ef4444' }}>
            Conflicts: <strong>{status.conflicts}</strong>
          </div>
        )}
        {status.sync_root && (
          <div style={{ marginTop: 8, color: '#6b7280' }}>{status.sync_root}</div>
        )}
      </div>
    </div>
  )
}
