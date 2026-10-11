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
  // The fields changed since the last write went out. A write carries ONLY these (task 1889, fix round 1 M5): the
  // config this page read on mount is never sent back, so a pause made from the tray meanwhile is not undone.
  const unsent = useRef<Partial<DesktopConfig>>({})
  // Which change touched each field last (task 1889, fix round 2 N1). A failed write puts a field back to be sent again
  // only if no later change has touched it, whether that later change is still unsent or already went out: a retry must
  // never write an older value over a newer one.
  const changeNumber = useRef(0)
  const lastChange = useRef<Record<string, number>>({})
  // The writes are sent one at a time, in the order the changes were made (task 1889, fix round 4). Two requests in
  // flight at once land in whatever order the IPC and the config lock deliver them, so "Pause on" then "Pause off" sent
  // quickly could end paused on the disk. A write is a local file replace that takes milliseconds, so a switch waiting
  // behind a slider's write is not something a person sees.
  const sending = useRef<Promise<void>>(Promise.resolve())

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

  const writeBack = (patch: Partial<DesktopConfig>, immediate = false) => {
    if (writeTimer.current) clearTimeout(writeTimer.current)
    for (const field of Object.keys(patch)) lastChange.current[field] = ++changeNumber.current
    unsent.current = { ...unsent.current, ...patch }
    const fire = () => {
      const body = unsent.current
      unsent.current = {}
      const fields = Object.keys(body) as Array<keyof DesktopConfig>
      if (fields.length === 0) return // an earlier write already carried everything
      const changeOf = Object.fromEntries(fields.map((field) => [field, lastChange.current[field]]))
      const send = () =>
        invoke('set_desktop_config', { config: body }).then(
          () => undefined,
          (e: unknown) => {
            // The change did not reach the file: it goes out again with the next write, unless a later change has
            // touched the same field since (that one is newer, and either went out already or is waiting its turn).
            const retry: Partial<Record<keyof DesktopConfig, unknown>> = {}
            for (const field of fields) {
              if (lastChange.current[field] === changeOf[field]) retry[field] = body[field]
            }
            unsent.current = { ...unsent.current, ...(retry as Partial<DesktopConfig>) }
            console.warn('set_desktop_config failed:', e)
            showToast({
              id: 'bandwidth-config-save-error',
              variant: 'error',
              title: 'Bandwidth settings were not saved',
              message: e instanceof Error ? e.message : String(e),
              durationMs: null,
            })
          },
        )
      sending.current = sending.current.then(send)
    }
    if (immediate) fire()
    else writeTimer.current = setTimeout(fire, DEBOUNCE_MS)
  }

  if (!config) return <p style={{ color: 'var(--ink-3)' }}>Loading…</p>

  const update = (patch: Partial<DesktopConfig>, immediate = false) => {
    const next = { ...config, ...patch }
    setConfig(next)
    writeBack(patch, immediate)
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
          style={{ accentColor: 'var(--amber)' }}
        />
        <span style={{ fontWeight: 500 }}>Pause sync</span>
        <span style={{ color: 'var(--ink-3)', fontSize: 12 }}>
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
          <span style={{ color: 'var(--ink-3)' }}>
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
          <span style={{ color: 'var(--ink-3)' }}>
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

      <p style={{ marginTop: 16, fontSize: 12, color: 'var(--ink-3)' }}>
        Drag a slider to the far left for unlimited.
      </p>
    </div>
  )
}
