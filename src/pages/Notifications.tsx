/**
 * Notification preferences.
 *
 * Three toggles backed by the desktop config. The flags are consumed
 * by the native notification dispatcher in the daemon — when the
 * matching event fires (conflict, sync done, quota warning) it checks
 * the bool before calling `tauri_plugin_notification`.
 *
 * Each toggle writes immediately — no debounce because state changes
 * are rare and the user expects an instant ack.
 *
 * See docs/superpowers/plans/2026-05-07-desktop-sync-client.md (Task 9).
 */

import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useToast } from '../windows/ui'

interface DesktopConfig {
  upload_kbps_limit: number
  download_kbps_limit: number
  pause_sync: boolean
  notify_conflicts: boolean
  notify_sync_complete: boolean
  notify_quota_warnings: boolean
}

const DEFAULT_CONFIG: DesktopConfig = {
  upload_kbps_limit: 0,
  download_kbps_limit: 0,
  pause_sync: false,
  notify_conflicts: true,
  notify_sync_complete: false,
  notify_quota_warnings: true,
}

interface ToggleSpec {
  key: 'notify_conflicts' | 'notify_sync_complete' | 'notify_quota_warnings'
  label: string
  description: string
}

const TOGGLES: ToggleSpec[] = [
  {
    key: 'notify_conflicts',
    label: 'Conflict alerts',
    description: 'When local + remote both changed and we kept both copies.',
  },
  {
    key: 'notify_sync_complete',
    label: 'Sync complete',
    description: 'Quiet by default — most users want this off.',
  },
  {
    key: 'notify_quota_warnings',
    label: 'Storage quota warnings',
    description: 'When you cross 90 % and 100 % of your plan’s storage.',
  },
]

export default function Notifications() {
  const { showToast } = useToast()
  const [config, setConfig] = useState<DesktopConfig | null>(null)

  useEffect(() => {
    invoke<DesktopConfig>('get_desktop_config')
      .then(setConfig)
      .catch((e: unknown) => {
        console.warn('get_desktop_config failed:', e)
        setConfig(DEFAULT_CONFIG)
        showToast({
          id: 'notification-config-defaults',
          variant: 'warning',
          title: 'Notification settings using defaults',
          message: 'Notification preferences are not wired up yet; toggles shown are defaults and will not persist.',
          durationMs: 9000,
        })
      })
  }, [showToast])

  if (!config) return <p style={{ color: '#9ca3af' }}>Loading…</p>

  const toggle = (key: ToggleSpec['key']) => {
    const next = { ...config, [key]: !config[key] }
    setConfig(next)
    invoke('set_desktop_config', { config: next }).catch((e: unknown) => {
      console.warn('set_desktop_config failed:', e)
      showToast({
        id: 'notification-config-save-error',
        variant: 'error',
        title: 'Notification setting was not saved',
        message: e instanceof Error ? e.message : String(e),
        durationMs: null,
      })
    })
  }

  return (
    <div>
      <h2 style={{ fontSize: 18, fontWeight: 600, marginBottom: 16 }}>
        Notifications
      </h2>

      <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
        {TOGGLES.map((t) => {
          const inputId = `notification-${t.key}`
          return (
            <div
              key={t.key}
              style={{
                display: 'flex',
                alignItems: 'flex-start',
                gap: 10,
              }}
            >
              <input
                id={inputId}
                type="checkbox"
                checked={config[t.key]}
                onChange={() => toggle(t.key)}
                style={{ marginTop: 3 }}
              />
              <label htmlFor={inputId} style={{ cursor: 'pointer' }}>
                <span style={{ display: 'block', fontWeight: 500, fontSize: 14 }}>{t.label}</span>
                <span style={{ display: 'block', color: '#6b7280', fontSize: 12 }}>
                  {t.description}
                </span>
              </label>
            </div>
          )
        })}
      </div>
    </div>
  )
}
