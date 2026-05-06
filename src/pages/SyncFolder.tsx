/**
 * Sync-folder picker.
 *
 * Reads the current sync root from `sync_status` and lets the user
 * change it via the native folder dialog (`pick_sync_root`). Both
 * IPC commands live in `src-tauri/src/lib.rs`.
 *
 * See docs/superpowers/plans/2026-05-07-desktop-sync-client.md (Task 9).
 */

import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'

export default function SyncFolder() {
  const [syncRoot, setSyncRoot] = useState<string | null>(null)

  useEffect(() => {
    invoke<{ sync_root: string | null }>('sync_status')
      .then((s) => setSyncRoot(s.sync_root))
      .catch(console.warn)
  }, [])

  const changeFolderClick = () => {
    invoke<string | null>('pick_sync_root')
      .then((path) => {
        if (path) setSyncRoot(path)
      })
      .catch(console.warn)
  }

  return (
    <div>
      <h2 style={{ fontSize: 18, fontWeight: 600, marginBottom: 16 }}>Sync Folder</h2>
      <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
        <div
          style={{
            flex: 1,
            background: '#f3f4f6',
            borderRadius: 6,
            padding: '8px 12px',
            fontSize: 13,
          }}
        >
          {syncRoot ?? 'No folder selected'}
        </div>
        <button
          onClick={changeFolderClick}
          style={{
            padding: '8px 16px',
            background: '#fbbf24',
            color: '#92400e',
            border: 'none',
            borderRadius: 6,
            cursor: 'pointer',
            fontWeight: 600,
          }}
        >
          Change
        </button>
      </div>
      <p style={{ marginTop: 12, fontSize: 12, color: '#9ca3af' }}>
        Files in this folder sync with your Beebeeb vault. Encrypted before
        leaving your device.
      </p>
    </div>
  )
}
