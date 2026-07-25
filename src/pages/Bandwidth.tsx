/**
 * Bandwidth limits + pause toggle.
 *
 * Reads / writes the desktop config through IPC commands
 * `get_desktop_config` and `set_desktop_config` (added to lib.rs in
 * a sister rust-engineer task). The config persists in `desktop.toml`
 * next to the state DB.
 *
 * Slider changes debounce 500 ms before writing back so dragging
 * doesn't hammer the IPC bridge. The pause toggle writes immediately.
 *
 * See docs/superpowers/plans/2026-05-07-desktop-sync-client.md (Task 9).
 */

import { useEffect, useRef, useState } from 'react'
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

// Slider ceiling: 100 Mbps = 100 000 kbps. Anything higher is effectively
// unlimited for almost all consumer connections.
const SLIDER_MAX_KBPS = 100_000 // 100 Mbps
const DEBOUNCE_MS = 500

/**
 * Convert kbps to a human label shown on the slider.
 *
 * Display unit is Mbps (SI decimal: 1 Mbps = 1000 kbps) to match the CLI
 * `bb speedtest` output and standard router/ISP marketing figures.
 * For very low values we fall back to kbps to keep the display readable.
 */
function formatLimit(kbps: number): string {
  if (kbps === 0) return 'Max network speed: unlimited'
  // kbps → Mbps via decimal (1 Mbps = 1000 kbps, the network convention)
  const mbps = kbps / 1000
  if (mbps >= 1) return `Max network speed: ${mbps % 1 === 0 ? mbps : mbps.toFixed(1)} Mbps`
  return `Max network speed: ${kbps} kbps`
}

export default function Bandwidth() {
  const { showToast } = useToast()
  const [config, setConfig] = useState<DesktopConfig | null>(null)
  const writeTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

  useEffect(() => {
    invoke<DesktopConfig>('get_desktop_config')
      .then(setConfig)
      .catch((e: unknown) => {
        // Config IPC not available yet — fall back to defaults so the UI
        // still renders, and surface the error to the user.
        console.warn('get_desktop_config failed:', e)
        setConfig(DEFAULT_CONFIG)
        showToast({
          id: 'bandwidth-config-defaults',
          variant: 'warning',
          title: 'Bandwidth settings using defaults',
          message: 'Limits are not wired up yet; values shown are defaults and will not persist.',
          durationMs: 9000,
        })
      })
    return () => {
      if (writeTimer.current) clearTimeout(writeTimer.current)
    }
  }, [showToast])

  const writeBack = (next: DesktopConfig, immediate = false) => {
    if (writeTimer.current) clearTimeout(writeTimer.current)
    const fire = () => {
      invoke('set_desktop_config', { config: next }).catch((e: unknown) => {
        console.warn('set_desktop_config failed:', e)
        showToast({
          id: 'bandwidth-config-save-error',
          variant: 'error',
          title: 'Bandwidth settings were not saved',
          message: e instanceof Error ? e.message : String(e),
          durationMs: null,
        })
      })
    }
    if (immediate) fire()
    else writeTimer.current = setTimeout(fire, DEBOUNCE_MS)
  }

  if (!config) return <p style={{ color: '#9ca3af' }}>Loading…</p>

  const update = (patch: Partial<DesktopConfig>, immediate = false) => {
    const next = { ...config, ...patch }
    setConfig(next)
    writeBack(next, immediate)
  }

  return (
    <div>
      <h2 style={{ fontSize: 18, fontWeight: 600, marginBottom: 16 }}>Bandwidth</h2>

      <label
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 10,
          marginBottom: 20,
          fontSize: 14,
        }}
      >
        <input
          type="checkbox"
          checked={config.pause_sync}
          onChange={(e) => update({ pause_sync: e.target.checked }, true)}
        />
        <span style={{ fontWeight: 500 }}>Pause sync</span>
        <span style={{ color: '#9ca3af', fontSize: 12 }}>
          Stops upload + download until resumed.
        </span>
      </label>

      <div style={{ marginBottom: 20 }}>
        <div
          style={{
            display: 'flex',
            justifyContent: 'space-between',
            fontSize: 13,
            marginBottom: 6,
          }}
        >
          <span style={{ fontWeight: 500 }}>Upload limit</span>
          <span style={{ color: '#6b7280' }}>
            {formatLimit(config.upload_kbps_limit)}
          </span>
        </div>
        <input
          type="range"
          min={0}
          max={SLIDER_MAX_KBPS}
          step={1000}
          value={config.upload_kbps_limit}
          onChange={(e) =>
            update({ upload_kbps_limit: Number(e.target.value) })
          }
          style={{ width: '100%' }}
        />
      </div>

      <div>
        <div
          style={{
            display: 'flex',
            justifyContent: 'space-between',
            fontSize: 13,
            marginBottom: 6,
          }}
        >
          <span style={{ fontWeight: 500 }}>Download limit</span>
          <span style={{ color: '#6b7280' }}>
            {formatLimit(config.download_kbps_limit)}
          </span>
        </div>
        <input
          type="range"
          min={0}
          max={SLIDER_MAX_KBPS}
          step={1000}
          value={config.download_kbps_limit}
          onChange={(e) =>
            update({ download_kbps_limit: Number(e.target.value) })
          }
          style={{ width: '100%' }}
        />
      </div>

      <p style={{ marginTop: 16, fontSize: 12, color: '#9ca3af' }}>
        Drag a slider to the far left for unlimited.
      </p>
    </div>
  )
}
