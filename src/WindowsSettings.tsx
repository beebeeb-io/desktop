/**
 * WindowsSettings — Fluent-style Windows settings window for Beebeeb.
 *
 * Mounted when ?window=settings&platform=windows (or the default window on
 * Windows, as detected by the Rust side). Faithfully ported from
 * HiWindowsSettings in design/hifi/hifi-desktop.jsx.
 *
 * The nav sidebar has three groups: Beebeeb / Security / System.
 * The main panel currently renders the Sync section (the active item in the
 * mockup). All other nav items navigate to the equivalent pages already
 * implemented in App.tsx (Status, SyncFolder, SelectiveSync, Bandwidth,
 * Account).
 *
 * Invoke commands used:
 *   sync_status, desktop_storage_summary, desktop_config, set_desktop_config
 *   free_up_space (returns bytes_freed: number)
 *   windows_shell_integration_state → { installed: boolean }
 *   install_windows_shell_integration
 *   autostart_enabled, toggle_autostart
 */

import { useEffect, useRef, useState } from 'react'
import {
  command,
  commandUnavailableLabel,
  formatBytes,
  loadSyncStatus,
  type DesktopConfig,
  type FinderInstallState,
  type StorageSummary,
  type SyncStatus,
  DEFAULT_CONFIG,
} from './desktopApi'
import UpdateBanner from './UpdateBanner'
import { useRegionCity } from './windows/useRegion'

// Windows Explorer/Cloud Files integration uses the same shape as macOS Finder
type ShellIntegrationState = FinderInstallState

// ── Design tokens ─────────────────────────────────────────────────────────

const T = {
  paper: 'oklch(0.985 0.004 85)',
  paper2: 'oklch(0.968 0.006 85)',
  paper3: 'oklch(0.945 0.008 82)',
  line: 'oklch(0.90 0.008 82)',
  line2: 'oklch(0.83 0.01 80)',
  ink: 'oklch(0.18 0.01 70)',
  ink2: 'oklch(0.34 0.012 75)',
  ink3: 'oklch(0.52 0.01 78)',
  ink4: 'oklch(0.68 0.008 80)',
  amber: 'oklch(0.82 0.17 84)',
  amberDeep: 'oklch(0.66 0.15 72)',
  amberBg: 'oklch(0.97 0.03 92)',
  green: 'oklch(0.72 0.16 155)',
  fontSans: "'Inter', 'Segoe UI', system-ui, ui-sans-serif, sans-serif",
  fontMono: "'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace",
} as const

// ── Nav structure ─────────────────────────────────────────────────────────

type NavId =
  | 'sync'
  | 'launch'
  | 'explorer-integration'
  | 'updates'
  | 'advanced'

// Nav items that are always reachable, whether or not the user is signed in.
// Everything else is auth-gated: hidden from the sidebar when logged_in is false.
const ALWAYS_ACCESSIBLE: ReadonlySet<NavId> = new Set(['launch', 'explorer-integration', 'updates', 'advanced'])

interface NavItem {
  id: NavId
  label: string
  icon: string
  active?: boolean
}

const NAV_SECTIONS: Array<{ heading: string; items: NavItem[] }> = [
  {
    heading: 'Beebeeb',
    items: [
      { id: 'sync', label: 'Sync', icon: 'cloud', active: true },
    ],
  },
  {
    heading: 'System',
    items: [
      { id: 'launch', label: 'Launch', icon: 'play' },
      { id: 'explorer-integration', label: 'Explorer integration', icon: 'folder' },
      { id: 'updates', label: 'Updates', icon: 'download' },
      { id: 'advanced', label: 'Advanced', icon: 'cog' },
    ],
  },
]

// ── Mini icon set ─────────────────────────────────────────────────────────

function NavIcon({ name, size = 13, color = 'currentColor' }: { name: string; size?: number; color?: string }) {
  const s = size
  switch (name) {
    case 'user':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4">
          <circle cx="8" cy="5.5" r="2.5" />
          <path d="M2 14 C2 11 4.5 9 8 9 C11.5 9 14 11 14 14" strokeLinecap="round" />
        </svg>
      )
    case 'cloud':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4">
          <path d="M4.5 11 C2.5 11 2 8 4 8 C4 5.5 7 4 9.5 5 C11 4 14 5.5 13.5 8 C15.5 8 15.5 11 13.5 11 Z" />
        </svg>
      )
    case 'folder':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4">
          <path d="M2 5 L2 13 L14 13 L14 6 L7 6 L5.5 4 L2 4 Z" strokeLinejoin="round" />
        </svg>
      )
    case 'bolt':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill={color}>
          <path d="M9 2 L4 9 L8 9 L7 14 L12 7 L8 7 Z" />
        </svg>
      )
    case 'lock':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4">
          <rect x="3" y="7" width="10" height="7" rx="2" />
          <path d="M5 7 L5 5 C5 3.3 11 3.3 11 5 L11 7" />
        </svg>
      )
    case 'key':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4" strokeLinecap="round">
          <circle cx="6" cy="6.5" r="3" />
          <path d="M8.5 9 L14 14.5 M12 12.5 L13.5 11" />
        </svg>
      )
    case 'device':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4">
          <rect x="2" y="3" width="9" height="7" rx="1.5" />
          <rect x="12" y="5" width="3" height="5" rx="1" />
          <line x1="2" y1="12" x2="11" y2="12" />
        </svg>
      )
    case 'play':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill={color}>
          <path d="M5 3 L13 8 L5 13 Z" />
        </svg>
      )
    case 'cog':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4">
          <circle cx="8" cy="8" r="2.5" />
          <path d="M8 1.5 L8 3.5 M8 12.5 L8 14.5 M1.5 8 L3.5 8 M12.5 8 L14.5 8 M3.5 3.5 L5 5 M11 11 L12.5 12.5 M12.5 3.5 L11 5 M5 11 L3.5 12.5" strokeLinecap="round" />
        </svg>
      )
    case 'download':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round">
          <path d="M8 2 L8 10 M5 7 L8 10 L11 7" />
          <path d="M2 13 L14 13" />
        </svg>
      )
    case 'search':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4">
          <circle cx="7" cy="7" r="4" />
          <line x1="10" y1="10" x2="14" y2="14" strokeLinecap="round" />
        </svg>
      )
    case 'shield':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4" strokeLinejoin="round">
          <path d="M8 2 L13 4 L13 8 C13 11.5 10.5 13.5 8 14.5 C5.5 13.5 3 11.5 3 8 L3 4 Z" />
        </svg>
      )
    default:
      return <span style={{ display: 'block', width: s, height: s }} />
  }
}

// Toggle switch
function Toggle({ on, onChange }: { on: boolean; onChange: (next: boolean) => void }) {
  return (
    <button
      role="switch"
      aria-checked={on}
      onClick={() => onChange(!on)}
      style={{
        width: 36,
        height: 20,
        borderRadius: 999,
        border: 'none',
        padding: 2,
        background: on ? T.amberDeep : T.line2,
        cursor: 'pointer',
        transition: 'background 150ms ease',
        display: 'inline-flex',
        alignItems: 'center',
        flexShrink: 0,
      }}
    >
      <div style={{
        width: 16,
        height: 16,
        borderRadius: '50%',
        background: T.paper,
        transform: on ? 'translateX(16px)' : 'translateX(0)',
        transition: 'transform 150ms ease',
        boxShadow: '0 1px 3px rgba(0,0,0,0.25)',
      }} />
    </button>
  )
}

// Chip badge
function Chip({ children }: { children: React.ReactNode }) {
  return (
    <span style={{
      display: 'inline-flex',
      alignItems: 'center',
      padding: '1px 7px',
      fontSize: 10,
      fontFamily: T.fontMono,
      border: `1px solid ${T.line2}`,
      borderRadius: 999,
      background: T.paper2,
      color: T.ink3,
      whiteSpace: 'nowrap' as const,
    }}>
      {children}
    </span>
  )
}

// Primary button
function PrimaryBtn({
  children,
  onClick,
  disabled,
}: {
  children: React.ReactNode
  onClick?: () => void
  disabled?: boolean
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 6,
        padding: '7px 14px',
        fontSize: 12.5,
        fontFamily: T.fontSans,
        fontWeight: 500,
        borderRadius: 6,
        border: `1px solid ${T.ink}`,
        background: T.ink,
        color: T.paper,
        cursor: disabled ? 'not-allowed' : 'pointer',
        opacity: disabled ? 0.55 : 1,
        whiteSpace: 'nowrap' as const,
        letterSpacing: '-0.005em',
      }}
    >
      {children}
    </button>
  )
}

// ── Signed-out gate ───────────────────────────────────────────────────────

function SignedOutGate() {
  return (
    <div style={{
      display: 'flex',
      flexDirection: 'column',
      alignItems: 'center',
      justifyContent: 'center',
      flex: 1,
      padding: '28px 36px',
      gap: 10,
    }}>
      <NavIcon name="lock" size={24} color={T.ink3} />
      <div style={{ fontSize: 15, fontWeight: 600, color: T.ink, marginTop: 6 }}>
        Sign in required
      </div>
      <div style={{ fontSize: 12, color: T.ink3, textAlign: 'center' as const, lineHeight: 1.6, maxWidth: 280 }}>
        Sign in to Beebeeb to access settings.
      </div>
    </div>
  )
}

// ── Sync panel ────────────────────────────────────────────────────────────

interface SyncRow {
  label: string
  hint: string
  toggle?: boolean
  locked?: boolean
  value?: string
  configKey?: keyof DesktopConfig
}

const SYNC_ROWS: SyncRow[] = [
  {
    label: 'Keep files encrypted end-to-end',
    hint: 'This cannot be disabled. Every byte on disk is ciphertext.',
    toggle: true,
    locked: true,
  },
  {
    label: 'Sync on metered connections',
    hint: 'When off, Beebeeb waits for Wi-Fi or Ethernet.',
    toggle: false,
  },
  {
    label: 'Upload rate limit',
    hint: 'Auto = adapt to available bandwidth',
    value: 'Auto',
    configKey: 'upload_kbps_limit',
  },
  {
    label: 'Download rate limit',
    hint: 'Auto = adapt to available bandwidth',
    value: 'Auto',
    configKey: 'download_kbps_limit',
  },
  {
    label: 'Files on Demand (online-only by default)',
    hint: 'Placeholders appear in File Explorer. Open a file to download.',
    toggle: true,
  },
  {
    label: 'Show sync overlays in File Explorer',
    hint: 'The ✓, cloud, and spinner icons on your files.',
    toggle: true,
  },
]

function SyncPanel({
  status,
  storage,
  config,
  onConfigChange,
  notice,
}: {
  status: SyncStatus | null
  storage: StorageSummary | null
  config: DesktopConfig
  onConfigChange: (patch: Partial<DesktopConfig>) => void
  notice: string | null
}) {
  const [freeUpBusy, setFreeUpBusy] = useState(false)
  const [freeUpNote, setFreeUpNote] = useState<string | null>(null)

  const synced7d = storage ? formatBytes(storage.used_bytes) : '…'
  const onPc = storage ? formatBytes(storage.pinned_bytes + storage.cache_bytes) : '…'
  const [lastSyncLabel, setLastSyncLabel] = useState('…')
  useEffect(() => {
    if (status?.syncing === 0 && status.engine === 'running') {
      setLastSyncLabel('Just now')
    } else if (status?.engine === 'running') {
      setLastSyncLabel('Syncing…')
    } else {
      setLastSyncLabel('—')
    }
  }, [status?.syncing, status?.engine])

  const freeUp = async () => {
    setFreeUpBusy(true)
    setFreeUpNote(null)
    const result = await command<{ bytes_freed: number }>('free_up_space')
    setFreeUpBusy(false)
    if (result.ok) {
      setFreeUpNote(`Freed ${formatBytes(result.value.bytes_freed)}`)
    } else {
      setFreeUpNote(result.unsupported ? commandUnavailableLabel('free_up_space') : result.reason)
    }
  }

  // Toggle values come from config — no separate local state
  const metered = config.metered ?? false
  const filesOnDemand = config.files_on_demand ?? true
  const overlays = config.sync_overlays ?? true

  return (
    <div style={{ overflow: 'auto', padding: '28px 36px', flex: 1 }}>
      {/* Header */}
      <div style={{ display: 'flex', alignItems: 'baseline', gap: 10, marginBottom: 4 }}>
        <h1 style={{
          margin: 0,
          fontSize: 26,
          fontWeight: 700,
          letterSpacing: '-0.025em',
          color: T.ink,
          lineHeight: 1.15,
        }}>
          Sync
        </h1>
        <span style={{
          fontSize: 11.5,
          fontFamily: T.fontMono,
          color: T.ink3,
        }}>
          &middot; {status?.engine === 'running' && status.syncing === 0 ? 'All devices up to date' : 'Syncing…'}
        </span>
      </div>
      <p style={{ margin: '0 0 24px', fontSize: 12, color: T.ink3, lineHeight: 1.6 }}>
        Manage what syncs with this PC and how. Files are encrypted on this device before upload.
      </p>

      {/* Status card */}
      <div style={{
        padding: 20,
        borderRadius: 10,
        background: `linear-gradient(135deg, ${T.amberBg}, ${T.paper2})`,
        border: `1px solid ${T.amberDeep}`,
        marginBottom: 22,
        display: 'grid',
        gridTemplateColumns: 'repeat(4, minmax(0, 1fr))',
        gap: 20,
      }}>
        {[
          { k: 'State', v: status?.engine === 'running' ? (status.syncing > 0 ? 'Syncing' : 'Up to date') : 'Paused' },
          { k: 'On this PC', v: onPc },
          { k: 'In vault', v: synced7d },
          { k: 'Last sync', v: lastSyncLabel },
        ].map(({ k, v }) => (
          <div key={k}>
            <div style={{
              fontSize: 9.5,
              fontFamily: T.fontMono,
              textTransform: 'uppercase' as const,
              letterSpacing: '0.07em',
              color: T.ink3,
              marginBottom: 4,
            }}>{k}</div>
            <div style={{
              fontSize: 15,
              fontWeight: 600,
              fontFamily: T.fontMono,
              color: T.ink,
            }}>{v}</div>
          </div>
        ))}
      </div>

      {/* Settings rows */}
      <div style={{
        background: T.paper,
        borderRadius: 10,
        border: `1px solid ${T.line}`,
        overflow: 'hidden',
      }}>
        {SYNC_ROWS.map((row, i) => {
          const isLast = i === SYNC_ROWS.length - 1
          let control: React.ReactNode = null

          if (row.locked) {
            control = (
              <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                <Toggle on={true} onChange={() => { /* locked */ }} />
              </div>
            )
          } else if (row.label === 'Sync on metered connections') {
            control = (
              <Toggle
                on={metered}
                onChange={(v) => onConfigChange({ metered: v })}
              />
            )
          } else if (row.label === 'Files on Demand (online-only by default)') {
            control = (
              <Toggle
                on={filesOnDemand}
                onChange={(v) => onConfigChange({ files_on_demand: v })}
              />
            )
          } else if (row.label === 'Show sync overlays in File Explorer') {
            control = (
              <Toggle
                on={overlays}
                onChange={(v) => onConfigChange({ sync_overlays: v })}
              />
            )
          } else if (row.value !== undefined) {
            const limit = row.configKey === 'upload_kbps_limit' ? config.upload_kbps_limit : config.download_kbps_limit
            control = (
              <span style={{ fontSize: 11.5, fontFamily: T.fontMono, color: T.ink3 }}>
                {limit === 0 ? 'Auto' : `${limit} KB/s`} &#x25be;
              </span>
            )
          }

          return (
            <div
              key={i}
              style={{
                display: 'grid',
                gridTemplateColumns: '1fr 140px',
                padding: '14px 18px',
                alignItems: 'center',
                gap: 12,
                borderBottom: isLast ? 'none' : `1px solid ${T.line}`,
              }}
            >
              <div>
                <div style={{
                  fontSize: 13,
                  fontWeight: 500,
                  color: T.ink,
                  display: 'flex',
                  alignItems: 'center',
                  gap: 8,
                  flexWrap: 'wrap' as const,
                }}>
                  {row.label}
                  {row.locked && <Chip>Locked · by design</Chip>}
                </div>
                <div style={{ fontSize: 11.5, color: T.ink3, marginTop: 3, lineHeight: 1.4 }}>
                  {row.hint}
                </div>
              </div>
              <div style={{ display: 'flex', justifyContent: 'flex-end', alignItems: 'center' }}>
                {control}
              </div>
            </div>
          )
        })}
      </div>

      {/* Notice */}
      {notice && (
        <div style={{
          marginTop: 14,
          padding: '10px 14px',
          borderRadius: 8,
          border: `1px solid oklch(0.88 0.05 84)`,
          background: T.amberBg,
          fontSize: 12,
          color: T.amberDeep,
          lineHeight: 1.5,
        }}>
          {notice}
        </div>
      )}

      {/* Storage footer */}
      <div style={{
        marginTop: 24,
        padding: 18,
        background: T.paper2,
        border: `1px solid ${T.line}`,
        borderRadius: 10,
        display: 'flex',
        alignItems: 'center',
        gap: 14,
      }}>
        <div style={{
          width: 32,
          height: 32,
          borderRadius: 8,
          background: T.paper,
          border: `1px solid ${T.line}`,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          flexShrink: 0,
        }}>
          <NavIcon name="cloud" size={14} color={T.amberDeep} />
        </div>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ fontSize: 13, fontWeight: 500, color: T.ink, marginBottom: 2 }}>
            Free up space
          </div>
          <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.4 }}>
            Convert files you have not opened in 30 days to online-only.
            Keeps them in the vault, removes bytes from this PC.
          </div>
          {freeUpNote && (
            <div style={{ fontSize: 11, color: T.ink3, marginTop: 4, fontFamily: T.fontMono }}>
              {freeUpNote}
            </div>
          )}
        </div>
        <PrimaryBtn onClick={() => void freeUp()} disabled={freeUpBusy}>
          {freeUpBusy ? 'Working…' : 'Free up space'}
        </PrimaryBtn>
      </div>
    </div>
  )
}

// ── Sidebar placeholder panels ────────────────────────────────────────────

// ── Launch panel (System → Launch) ────────────────────────────────────────
//
// "Start at login" maps to the tauri-plugin-autostart registry Run key on
// Windows (HKCU\…\Run). `toggle_autostart` flips it and returns the new state;
// `autostart_enabled` reads it. Both commands already exist in lib.rs and are
// registered in the invoke_handler.
function LaunchPanel() {
  const [enabled, setEnabled] = useState<boolean | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    command<boolean>('autostart_enabled').then((r) => {
      if (cancelled) return
      if (r.ok) setEnabled(r.value)
      else setError(r.unsupported ? commandUnavailableLabel('autostart_enabled') : r.reason)
    })
    return () => { cancelled = true }
  }, [])

  const toggle = async () => {
    setBusy(true)
    setError(null)
    const r = await command<boolean>('toggle_autostart')
    setBusy(false)
    if (r.ok) { setEnabled(r.value); return }
    setError(r.unsupported ? commandUnavailableLabel('toggle_autostart') : r.reason)
  }

  return (
    <div style={{ overflow: 'auto', padding: '28px 36px', flex: 1 }}>
      <h1 style={{ margin: '0 0 8px', fontSize: 26, fontWeight: 700, letterSpacing: '-0.025em', color: T.ink, lineHeight: 1.15 }}>
        Launch
      </h1>
      <p style={{ margin: '0 0 24px', fontSize: 12, color: T.ink3, lineHeight: 1.6 }}>
        Control how Beebeeb starts on this PC.
      </p>
      {error && (
        <div style={{
          padding: '10px 12px', borderRadius: 6, marginBottom: 16,
          border: '1px solid oklch(0.88 0.05 25)', background: 'oklch(0.98 0.02 25)',
          color: 'oklch(0.42 0.15 25)', fontSize: 12,
        }}>
          {error}
        </div>
      )}
      <div style={{
        display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        gap: 16, padding: '14px 16px', borderRadius: 8,
        border: `1px solid ${T.line}`, background: T.paper2,
      }}>
        <div>
          <div style={{ fontSize: 13, fontWeight: 600, color: T.ink, marginBottom: 3 }}>
            Start at login
          </div>
          <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.5 }}>
            Beebeeb launches automatically and resumes syncing when you sign in to Windows.
          </div>
        </div>
        <div style={{ opacity: busy || enabled === null ? 0.5 : 1, pointerEvents: busy ? 'none' : 'auto' }}>
          <Toggle on={enabled === true} onChange={() => void toggle()} />
        </div>
      </div>
    </div>
  )
}

// ── Explorer integration panel (System → Explorer integration) ────────────
//
// `windows_shell_integration_state` reports whether Beebeeb is registered as a
// File Explorer sync location (Cloud Files API). `install_windows_shell_integration`
// registers it. Both commands already exist in lib.rs.
function ExplorerIntegrationPanel() {
  const [state, setState] = useState<ShellIntegrationState | null>(null)
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const regionCity = useRegionCity()

  const refresh = async () => {
    const r = await command<ShellIntegrationState>('windows_shell_integration_state')
    if (r.ok) {
      setState(r.value)
      setError(null)
    } else {
      setError(r.unsupported ? commandUnavailableLabel('windows_shell_integration_state') : r.reason)
    }
    setLoading(false)
  }

  useEffect(() => {
    let cancelled = false
    void (async () => {
      const r = await command<ShellIntegrationState>('windows_shell_integration_state')
      if (cancelled) return
      if (r.ok) setState(r.value)
      else setError(r.unsupported ? commandUnavailableLabel('windows_shell_integration_state') : r.reason)
      setLoading(false)
    })()
    return () => { cancelled = true }
  }, [])

  const enable = async () => {
    setBusy(true)
    setError(null)
    const r = await command<ShellIntegrationState>('install_windows_shell_integration')
    setBusy(false)
    if (r.ok) {
      setState(r.value)
      // Re-read in case install returns a transitional state
      void refresh()
      return
    }
    setError(r.unsupported ? commandUnavailableLabel('install_windows_shell_integration') : r.reason)
  }

  const active = state?.installed === true

  return (
    <div style={{ overflow: 'auto', padding: '28px 36px', flex: 1 }}>
      <h1 style={{ margin: '0 0 8px', fontSize: 26, fontWeight: 700, letterSpacing: '-0.025em', color: T.ink, lineHeight: 1.15 }}>
        Explorer integration
      </h1>
      <p style={{ margin: '0 0 24px', fontSize: 12, color: T.ink3, lineHeight: 1.6 }}>
        Beebeeb appears in File Explorer as a sync folder. Files are encrypted on this PC before they
        leave for {regionCity} — Explorer only ever shows you the decrypted view.
      </p>

      {error && (
        <div style={{
          padding: '10px 12px', borderRadius: 6, marginBottom: 16,
          border: '1px solid oklch(0.88 0.05 25)', background: 'oklch(0.98 0.02 25)',
          color: 'oklch(0.42 0.15 25)', fontSize: 12, lineHeight: 1.5,
        }}>
          {error}
        </div>
      )}

      <div style={{
        display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        gap: 16, padding: '14px 16px', borderRadius: 8,
        border: `1px solid ${T.line}`, background: T.paper2,
      }}>
        <div style={{ minWidth: 0 }}>
          <div style={{ fontSize: 13, fontWeight: 600, color: T.ink, marginBottom: 3 }}>
            File Explorer location
          </div>
          <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.5 }}>
            {loading
              ? 'Checking…'
              : active
                ? 'Active — Beebeeb is registered as a sync folder on this PC.'
                : 'Not set up yet on this PC.'}
          </div>
        </div>
        {!loading && !active && (
          <PrimaryBtn onClick={() => void enable()} disabled={busy}>
            {busy ? 'Enabling…' : 'Enable'}
          </PrimaryBtn>
        )}
        {!loading && active && (
          <Chip>Active</Chip>
        )}
      </div>
    </div>
  )
}

// ── Updates panel (System → Updates) ─────────────────────────────────────────
//
// Shows the current app version. The actual update banner (UpdateBanner.tsx)
// is shown at the top of the window whenever the backend emits
// "update-available". This panel is the "nothing pending" counterpart — it
// answers "is the updater working?" without nagging.
//
// We deliberately do NOT show a "last checked" timestamp: the real update check
// runs in the Rust backend at startup and every 4 h, and the frontend has no
// event telling it when that happened. Inventing a time from when the panel
// opened would be a false claim — honest over reassuring. We state the true
// cadence instead.
function UpdatesPanel() {
  const [version, setVersion] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    command<string>('app_version').then((r) => {
      if (!cancelled && r.ok) setVersion(r.value)
    })
    return () => { cancelled = true }
  }, [])

  return (
    <div style={{ overflow: 'auto', padding: '28px 36px', flex: 1 }}>
      <h1 style={{
        margin: '0 0 8px',
        fontSize: 26,
        fontWeight: 700,
        letterSpacing: '-0.025em',
        color: T.ink,
        lineHeight: 1.15,
      }}>
        Updates
      </h1>
      <p style={{ margin: '0 0 24px', fontSize: 12, color: T.ink3, lineHeight: 1.6 }}>
        Beebeeb checks for updates at launch and every 4 hours. When one is available, a
        banner appears at the top of this window.
      </p>

      {/* Version card */}
      <div style={{
        display: 'flex',
        alignItems: 'center',
        gap: 14,
        padding: '16px 18px',
        borderRadius: 10,
        border: `1px solid ${T.line}`,
        background: T.paper2,
        marginBottom: 16,
      }}>
        <div style={{
          width: 36,
          height: 36,
          borderRadius: 9,
          background: T.paper,
          border: `1px solid ${T.line}`,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          flexShrink: 0,
        }}>
          <NavIcon name="download" size={15} color={T.amberDeep} />
        </div>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ fontSize: 13, fontWeight: 600, color: T.ink, marginBottom: 3 }}>
            {version != null ? `Version ${version}` : 'Loading…'}
          </div>
          <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.4, fontFamily: T.fontMono }}>
            Up to date
          </div>
        </div>
        <Chip>Up to date</Chip>
      </div>

      <p style={{ margin: '0 0 16px', fontSize: 11.5, color: T.ink3, lineHeight: 1.6 }}>
        Beebeeb checks for updates automatically at startup and every 4 hours.
      </p>

      <p style={{ margin: 0, fontSize: 11.5, color: T.ink3, lineHeight: 1.6 }}>
        Updates are verified with a minisign signature before installation. No update is
        applied without your confirmation via the "Restart to update" banner.
      </p>
    </div>
  )
}

// Sections that are desktop-native but have no backend yet — honest empty state
const NATIVE_PENDING_SECTIONS: Partial<Record<NavId, string>> = {
  advanced: 'Advanced options are not available in this build.',
}

function NativePendingPanel({ label, description }: { label: string; description: string }) {
  return (
    <div style={{ overflow: 'auto', padding: '28px 36px', flex: 1 }}>
      <h1 style={{ margin: '0 0 8px', fontSize: 26, fontWeight: 700, letterSpacing: '-0.025em', color: T.ink, lineHeight: 1.15 }}>
        {label}
      </h1>
      <p style={{ margin: 0, fontSize: 12, color: T.ink3, lineHeight: 1.6 }}>
        {description}
      </p>
    </div>
  )
}

function PanelRouter({ navId }: { navId: NavId }) {
  const allItems = NAV_SECTIONS.flatMap((s) => s.items)
  const label = allItems.find((i) => i.id === navId)?.label ?? navId

  if (navId === 'explorer-integration') {
    return <ExplorerIntegrationPanel />
  }
  if (navId === 'updates') {
    return <UpdatesPanel />
  }
  const pendingDesc = NATIVE_PENDING_SECTIONS[navId]
  if (pendingDesc) {
    return <NativePendingPanel label={label} description={pendingDesc} />
  }
  // Fallback for any future nav items not yet categorised
  return (
    <div style={{ overflow: 'auto', padding: '28px 36px', flex: 1 }}>
      <h1 style={{ margin: '0 0 8px', fontSize: 26, fontWeight: 700, letterSpacing: '-0.025em', color: T.ink, lineHeight: 1.15 }}>
        {label}
      </h1>
    </div>
  )
}

// ── Root component ─────────────────────────────────────────────────────────

export default function WindowsSettings() {
  const [activeNav, setActiveNav] = useState<NavId>('sync')
  const regionCity = useRegionCity()
  const [status, setStatus] = useState<SyncStatus | null>(null)
  const [storage, setStorage] = useState<StorageSummary | null>(null)
  const [config, setConfig] = useState<DesktopConfig>(DEFAULT_CONFIG)
  // Synchronous source of truth for the freshest config. `config` (state) lags
  // by a render; `configRef.current` is updated the instant a patch is applied,
  // so two same-tick toggle clicks merge cumulatively instead of each starting
  // from the same stale closure. Kept in sync wherever config is authoritatively
  // set: the initial load-from-disk below, and every patch in handleConfigChange.
  const configRef = useRef<DesktopConfig>(DEFAULT_CONFIG)
  const [notice, setNotice] = useState<string | null>(null)
  const [searchQuery, setSearchQuery] = useState('')

  // Load status
  useEffect(() => {
    let cancelled = false
    const refresh = async () => {
      const next = await loadSyncStatus()
      if (!cancelled) setStatus(next)
    }
    void refresh()
    const id = window.setInterval(refresh, 5000)
    return () => { cancelled = true; window.clearInterval(id) }
  }, [])

  // Load storage summary when engine running
  useEffect(() => {
    if (!status?.logged_in || status.engine !== 'running') return
    let cancelled = false
    command<StorageSummary>('desktop_storage_summary').then((result) => {
      if (cancelled) return
      if (result.ok) setStorage(result.value)
    })
    return () => { cancelled = true }
  }, [status?.logged_in, status?.engine])

  // Load config — desktop_config returns null on first run (no config file yet)
  useEffect(() => {
    command<DesktopConfig | null>('desktop_config').then((result) => {
      if (result.ok && result.value != null) {
        // Seed both the ref (synchronous source of truth) and the state so the
        // first patch merges against the on-disk config, not DEFAULT_CONFIG.
        configRef.current = result.value
        setConfig(result.value)
      }
    })
  }, [])

  const handleConfigChange = async (patch: Partial<DesktopConfig>) => {
    setNotice(null)
    // Merge against configRef.current (the freshest value), NOT the closed-over
    // `config` state, then advance the ref synchronously BEFORE awaiting the IPC.
    // This is what makes two same-tick toggle clicks both persist: the second
    // call runs after the first has already written its merged result into the
    // ref, so it merges on top of that — both patches survive. (Reading the
    // lagging `config` state instead would make the second click clobber the
    // first.) We update the ref outside any setConfig updater so StrictMode's
    // double-invocation of updaters can't fire the IPC twice.
    const next: DesktopConfig = { ...configRef.current, ...patch }
    configRef.current = next
    setConfig(next)
    const result = await command<void>('set_desktop_config', { config: next })
    if (!result.ok) {
      setNotice(result.reason)
    }
  }

  const loggedIn = status?.logged_in ?? false

  // When the user signs out, reset to a always-accessible nav item so
  // auth-gated panels are never left on screen.
  useEffect(() => {
    if (!loggedIn && !ALWAYS_ACCESSIBLE.has(activeNav)) {
      setActiveNav('launch')
    }
  }, [loggedIn])

  // Filter nav by search query AND auth state.
  const query = searchQuery.toLowerCase()
  const filteredSections = NAV_SECTIONS.map((section) => ({
    ...section,
    items: section.items.filter(
      (item) =>
        // Auth gate: hide items that require login when signed out
        (loggedIn || ALWAYS_ACCESSIBLE.has(item.id)) &&
        // Search filter
        (
          !query ||
          item.label.toLowerCase().includes(query) ||
          section.heading.toLowerCase().includes(query)
        ),
    ),
  })).filter((section) => section.items.length > 0)

  return (
    <div style={{
      display: 'flex',
      flexDirection: 'column',
      height: '100vh',
      fontFamily: T.fontSans,
      color: T.ink,
      background: T.paper,
      overflow: 'hidden',
    }}>
      <UpdateBanner />
      <div style={{
        display: 'grid',
        gridTemplateColumns: '240px 1fr',
        flex: 1,
        overflow: 'hidden',
        minHeight: 0,
      }}>
      {/* Sidebar */}
      <div style={{
        background: T.paper2,
        borderRight: `1px solid ${T.line}`,
        padding: '16px 10px',
        overflow: 'auto',
        display: 'flex',
        flexDirection: 'column',
        gap: 0,
      }}>
        {/* Logo row */}
        <div style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          padding: '4px 8px 14px',
        }}>
          <div style={{
            width: 24,
            height: 24,
            borderRadius: 6,
            background: T.amber,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
          }}>
            <span style={{ fontWeight: 800, fontSize: 12, color: T.ink, lineHeight: 1 }}>b</span>
          </div>
          <span style={{ fontSize: 13, fontWeight: 600, letterSpacing: '-0.01em', color: T.ink }}>
            beebeeb.io
          </span>
        </div>

        {/* Search */}
        <div style={{
          display: 'flex',
          alignItems: 'center',
          gap: 7,
          padding: '6px 10px',
          background: T.paper,
          border: `1px solid ${T.line2}`,
          borderRadius: 6,
          marginBottom: 14,
        }}>
          <NavIcon name="search" size={11} color={T.ink4} />
          <input
            placeholder="Find a setting…"
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.currentTarget.value)}
            style={{
              border: 'none',
              background: 'transparent',
              outline: 'none',
              fontSize: 11.5,
              color: T.ink,
              fontFamily: T.fontSans,
              width: '100%',
            }}
          />
        </div>

        {/* Nav sections */}
        {filteredSections.map((section) => (
          <div key={section.heading} style={{ marginBottom: 10 }}>
            <div style={{
              fontSize: 9.5,
              fontFamily: T.fontMono,
              textTransform: 'uppercase' as const,
              letterSpacing: '0.07em',
              color: T.ink3,
              padding: '4px 10px',
              marginBottom: 2,
            }}>
              {section.heading}
            </div>
            {section.items.map((item) => {
              const active = item.id === activeNav
              return (
                <button
                  key={item.id}
                  onClick={() => setActiveNav(item.id)}
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 10,
                    width: '100%',
                    padding: '7px 10px',
                    borderRadius: 6,
                    fontSize: 12.5,
                    fontWeight: active ? 600 : 400,
                    color: active ? T.ink : T.ink2,
                    background: active ? T.paper : 'transparent',
                    border: active ? `1px solid ${T.line}` : '1px solid transparent',
                    borderLeft: active ? `3px solid ${T.amberDeep}` : '3px solid transparent',
                    cursor: 'pointer',
                    textAlign: 'left' as const,
                    fontFamily: T.fontSans,
                    marginBottom: 2,
                    transition: 'background 100ms',
                  }}
                >
                  <NavIcon
                    name={item.icon}
                    size={12}
                    color={active ? T.amberDeep : T.ink3}
                  />
                  {item.label}
                </button>
              )
            })}
          </div>
        ))}

        {/* Footer */}
        <div style={{
          marginTop: 'auto',
          paddingTop: 16,
          borderTop: `1px solid ${T.line}`,
          paddingLeft: 10,
          paddingRight: 10,
        }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
            <NavIcon name="shield" size={11} color={T.amberDeep} />
            <span style={{ fontSize: 10.5, fontFamily: T.fontMono, color: T.ink3, textTransform: 'uppercase' as const, letterSpacing: '0.04em' }}>
              {regionCity} · E2E encrypted
            </span>
          </div>
        </div>
      </div>

      {/* Main content area */}
      {activeNav === 'sync' && !loggedIn ? (
        <SignedOutGate />
      ) : activeNav === 'sync' ? (
        <SyncPanel
          status={status}
          storage={storage}
          config={config}
          onConfigChange={(patch) => void handleConfigChange(patch)}
          notice={notice}
        />
      ) : activeNav === 'launch' ? (
        <LaunchPanel />
      ) : (
        <PanelRouter navId={activeNav} />
      )}
      </div>
    </div>
  )
}
