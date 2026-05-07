/**
 * Selective Sync — choose which top-level folders sync to this device.
 *
 * The desktop's default behaviour is to mirror every file in the vault.
 * On a small SSD or a metered-bandwidth machine that's the wrong
 * default; this page lets the user opt individual top-level folders
 * out so they stay cloud-only and only download on demand.
 *
 * Wire-up:
 *   - `get_selective_sync`   → list of excluded folder IDs (Vec<String>)
 *   - `set_selective_sync`   → persist a new exclusion list
 *   - `list_vault_folders`   → top-level folders to render as checkboxes
 *
 * All three IPC commands live in `src-tauri/src/lib.rs`. The exclusion
 * list is stored in `~/.config/beebeeb/desktop.toml` under the
 * `excluded_folder_ids` key (see `config::DesktopConfig`).
 *
 * Style mirrors the rest of `pages/` — inline styles, no Tailwind, so
 * the settings shell renders the same in dev and in the Tauri WebView.
 *
 * Spec: `.claude/tasks/in-development/0090-desktop-selective-sync-ui.md`.
 */

import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'

interface VaultItem {
  id: string
  name: string
  is_folder: boolean
}

export default function SelectiveSync() {
  const [items, setItems] = useState<VaultItem[]>([])
  const [excluded, setExcluded] = useState<string[]>([])
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    const load = async () => {
      try {
        // Excluded list is the source of truth; default to empty if
        // the config key is missing so the page renders even on first
        // launch before any save has happened.
        const excl = await invoke<string[]>('get_selective_sync').catch(() => [])
        if (cancelled) return
        setExcluded(excl)

        // Vault folders come from the in-memory session via Rust.
        // The IPC returns [] when the user isn't signed in or the
        // API call fails — both render as the empty state below.
        const vault = await invoke<VaultItem[]>('list_vault_folders').catch(() => [])
        if (cancelled) return
        setItems(vault)
      } catch (e: unknown) {
        if (cancelled) return
        setError(e instanceof Error ? e.message : 'Failed to load')
      } finally {
        if (!cancelled) setLoading(false)
      }
    }
    void load()
    return () => {
      cancelled = true
    }
  }, [])

  const toggleFolder = async (folderId: string) => {
    const next = excluded.includes(folderId)
      ? excluded.filter((id) => id !== folderId)
      : [...excluded, folderId]
    setExcluded(next)
    setSaving(true)
    setError(null)
    try {
      await invoke('set_selective_sync', { excluded: next })
    } catch (e: unknown) {
      // Roll back the optimistic toggle so the UI matches what's on
      // disk if the save failed.
      setExcluded(excluded)
      setError(e instanceof Error ? e.message : 'Failed to save')
    } finally {
      setSaving(false)
    }
  }

  if (loading) {
    return (
      <div>
        <h2 style={{ fontSize: 18, fontWeight: 600, marginBottom: 16 }}>
          Sync Folders
        </h2>
        <p style={{ color: '#9ca3af', fontSize: 13 }}>Loading folders…</p>
      </div>
    )
  }

  return (
    <div>
      <h2 style={{ fontSize: 18, fontWeight: 600, marginBottom: 8 }}>
        Sync Folders
      </h2>
      <p
        style={{
          color: '#6b7280',
          fontSize: 13,
          marginBottom: 16,
          lineHeight: 1.5,
        }}
      >
        Choose which folders sync to this device. Unchecked folders stay
        cloud-only and download on demand.
      </p>

      {error && (
        <div
          style={{
            background: '#fee2e2',
            color: '#991b1b',
            border: '1px solid #fecaca',
            borderRadius: 6,
            padding: '8px 12px',
            fontSize: 12,
            marginBottom: 12,
          }}
        >
          {error}
        </div>
      )}

      {saving && (
        <div style={{ color: '#9ca3af', fontSize: 11, marginBottom: 8 }}>
          Saving…
        </div>
      )}

      {items.length === 0 ? (
        <div
          style={{
            background: '#f3f4f6',
            borderRadius: 8,
            padding: 16,
            color: '#6b7280',
            fontSize: 13,
          }}
        >
          No folders to choose from yet. Create folders in your vault and
          they'll appear here.
        </div>
      ) : (
        <ul
          style={{
            listStyle: 'none',
            margin: 0,
            padding: 0,
            display: 'flex',
            flexDirection: 'column',
            gap: 4,
          }}
        >
          {items
            .filter((i) => i.is_folder)
            .map((item) => {
              const checked = !excluded.includes(item.id)
              return (
                <li
                  key={item.id}
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 10,
                    padding: '8px 12px',
                    background: '#f9fafb',
                    borderRadius: 6,
                  }}
                >
                  <input
                    type="checkbox"
                    id={`folder-${item.id}`}
                    checked={checked}
                    disabled={saving}
                    onChange={() => void toggleFolder(item.id)}
                    style={{ width: 16, height: 16, cursor: 'pointer' }}
                  />
                  <label
                    htmlFor={`folder-${item.id}`}
                    style={{
                      fontSize: 13,
                      cursor: 'pointer',
                      color: checked ? '#111827' : '#6b7280',
                      flex: 1,
                    }}
                  >
                    {item.name}
                  </label>
                </li>
              )
            })}
        </ul>
      )}

      <p
        style={{
          marginTop: 16,
          fontSize: 11,
          color: '#9ca3af',
          lineHeight: 1.5,
        }}
      >
        Files that have already downloaded stay on this device until you
        remove them. Newly excluded folders just stop pulling fresh
        changes.
      </p>
    </div>
  )
}
