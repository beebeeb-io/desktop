/**
 * WindowsApp — the main Beebeeb app window for Windows (PKG-SHELL).
 *
 * Mounted when ?window=main-app&platform=windows (see main.tsx). Opened by the
 * Rust `show_main_app_window` command — the tray "Open Beebeeb" item/button and
 * the onboarding "Open control center" button both route here, and it's the
 * window shown automatically on launch once the PC is configured.
 *
 * This is the SHELL: a left sidebar (brand mark + nav groups + a live storage
 * widget) and a content area with a state-based router (mirroring
 * WindowsSettings' `activeNav` pattern). It hosts the data-backed views that
 * later packages will fill — TODAY most views are honest placeholders / skeleton
 * slots. What IS finished here: the window, nav + active states, routing,
 * responsive content area, the auth gate, loading/skeleton scaffolding, and the
 * live storage widget (wired to account_usage with a desktop_storage_summary
 * fallback).
 *
 * Data wrappers for the 15 account/billing/devices/activity IPCs live in
 * desktopApi.ts (accountProfile, accountUsage, …). The placeholder views below
 * call almost none of them yet — that's the next packages' job. Each slot is
 * marked with a `DATA SLOT:` note naming the wrapper(s) it should consume.
 *
 * Design tokens + idioms are shared with WindowsSettings.tsx (the `T` map and
 * NavIcon set are intentionally kept parallel so the two Windows windows feel
 * like one product). Brand: amber for encryption state + the active nav icon
 * accent only; Inter for humans, JetBrains Mono for ids/sizes; "Falkenstein ·
 * Hetzner"; honest voice; no emojis.
 */

import { useEffect, useRef, useState } from 'react'
import {
  command,
  commandUnavailableLabel,
  formatBytes,
  loadSyncStatus,
  openUrl,
  accountUsage,
  type BillingUsage,
  type StorageSummary,
  type SyncStatus,
} from './desktopApi'
import UpdateBanner from './UpdateBanner'

// ── Design tokens (parallel to WindowsSettings.tsx) ─────────────────────────

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

// ── Nav structure ───────────────────────────────────────────────────────────

type NavId =
  | 'home'
  | 'files'
  | 'account'
  | 'insights'
  | 'bandwidth'
  | 'selective-sync'
  | 'devices'
  | 'security'
  | 'activity'
  | 'settings'

// Nav items reachable whether or not the user is signed in. Everything else is
// auth-gated: hidden from the sidebar and replaced by the sign-in prompt.
const ALWAYS_ACCESSIBLE: ReadonlySet<NavId> = new Set(['home', 'settings'])

interface NavItem {
  id: NavId
  label: string
  icon: string
}

const NAV_SECTIONS: Array<{ heading: string; items: NavItem[] }> = [
  {
    heading: 'Beebeeb',
    items: [
      { id: 'home', label: 'Home', icon: 'home' },
      { id: 'files', label: 'Files', icon: 'folder' },
      { id: 'account', label: 'Account', icon: 'user' },
    ],
  },
  {
    heading: 'Usage',
    items: [
      { id: 'insights', label: 'Insights', icon: 'chart' },
      { id: 'bandwidth', label: 'Bandwidth', icon: 'bolt' },
      { id: 'selective-sync', label: 'Selective sync', icon: 'folder' },
    ],
  },
  {
    heading: 'Security',
    items: [
      { id: 'devices', label: 'Devices', icon: 'device' },
      { id: 'security', label: 'Security', icon: 'shield' },
      { id: 'activity', label: 'Activity', icon: 'clock' },
    ],
  },
  {
    heading: 'System',
    items: [{ id: 'settings', label: 'Settings', icon: 'cog' }],
  },
]

// ── Mini icon set (superset of WindowsSettings' NavIcon) ─────────────────────

function NavIcon({ name, size = 13, color = 'currentColor' }: { name: string; size?: number; color?: string }) {
  const s = size
  switch (name) {
    case 'home':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4" strokeLinejoin="round">
          <path d="M2.5 7 L8 2.5 L13.5 7 L13.5 13 L9.5 13 L9.5 9.5 L6.5 9.5 L6.5 13 L2.5 13 Z" />
        </svg>
      )
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
    case 'chart':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4" strokeLinecap="round">
          <path d="M2 13 L14 13" />
          <rect x="3.5" y="8" width="2.2" height="4" rx="0.5" fill={color} stroke="none" />
          <rect x="7" y="5" width="2.2" height="7" rx="0.5" fill={color} stroke="none" />
          <rect x="10.5" y="9.5" width="2.2" height="2.5" rx="0.5" fill={color} stroke="none" />
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
    case 'shield':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4" strokeLinejoin="round">
          <path d="M8 2 L13 4 L13 8 C13 11.5 10.5 13.5 8 14.5 C5.5 13.5 3 11.5 3 8 L3 4 Z" />
        </svg>
      )
    case 'clock':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4" strokeLinecap="round">
          <circle cx="8" cy="8" r="6" />
          <path d="M8 5 L8 8 L10.5 9.5" />
        </svg>
      )
    case 'cog':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4">
          <circle cx="8" cy="8" r="2.5" />
          <path d="M8 1.5 L8 3.5 M8 12.5 L8 14.5 M1.5 8 L3.5 8 M12.5 8 L14.5 8 M3.5 3.5 L5 5 M11 11 L12.5 12.5 M12.5 3.5 L11 5 M5 11 L3.5 12.5" strokeLinecap="round" />
        </svg>
      )
    case 'lock':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4">
          <rect x="3" y="7" width="10" height="7" rx="2" />
          <path d="M5 7 L5 5 C5 3.3 11 3.3 11 5 L11 7" />
        </svg>
      )
    case 'external':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round">
          <path d="M9 2 L14 2 L14 7 M14 2 L7.5 8.5" />
          <path d="M12 9.5 L12 13 L3 13 L3 4 L6.5 4" />
        </svg>
      )
    default:
      return <span style={{ display: 'block', width: s, height: s }} />
  }
}

// ── Shared primitives ───────────────────────────────────────────────────────

function Chip({ children, tone = 'neutral' }: { children: React.ReactNode; tone?: 'neutral' | 'amber' | 'green' }) {
  const palette = {
    neutral: { bg: T.paper2, border: T.line2, color: T.ink3 },
    amber: { bg: T.amberBg, border: 'oklch(0.86 0.07 90)', color: 'oklch(0.4 0.08 72)' },
    green: { bg: 'oklch(0.96 0.04 155)', border: 'oklch(0.87 0.08 155)', color: 'oklch(0.4 0.1 155)' },
  }[tone]
  return (
    <span style={{
      display: 'inline-flex',
      alignItems: 'center',
      gap: 5,
      padding: '1px 7px',
      fontSize: 10,
      fontFamily: T.fontMono,
      border: `1px solid ${palette.border}`,
      borderRadius: 999,
      background: palette.bg,
      color: palette.color,
      whiteSpace: 'nowrap' as const,
    }}>
      {children}
    </span>
  )
}

function PrimaryBtn({ children, onClick, disabled }: { children: React.ReactNode; onClick?: () => void; disabled?: boolean }) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 6,
        padding: '8px 14px',
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

// A shimmering skeleton block — the canonical loading state for the data slots.
function Skeleton({ width = '100%', height = 14, radius = 6 }: { width?: number | string; height?: number; radius?: number }) {
  return (
    <span
      aria-hidden
      style={{
        display: 'block',
        width,
        height,
        borderRadius: radius,
        background: `linear-gradient(90deg, ${T.paper3} 0%, ${T.paper2} 50%, ${T.paper3} 100%)`,
        backgroundSize: '200% 100%',
        animation: 'bb-shimmer 1.3s ease-in-out infinite',
      }}
    />
  )
}

// Reusable page header (title + subtitle).
function PageHeader({ title, subtitle, aside }: { title: string; subtitle?: string; aside?: React.ReactNode }) {
  return (
    <div style={{ display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between', gap: 16, marginBottom: 20 }}>
      <div style={{ minWidth: 0 }}>
        <h1 style={{ margin: '0 0 6px', fontSize: 26, fontWeight: 700, letterSpacing: '-0.025em', color: T.ink, lineHeight: 1.15 }}>
          {title}
        </h1>
        {subtitle && <p style={{ margin: 0, fontSize: 12, color: T.ink3, lineHeight: 1.6, maxWidth: 560 }}>{subtitle}</p>}
      </div>
      {aside}
    </div>
  )
}

function Card({ children, style }: { children: React.ReactNode; style?: React.CSSProperties }) {
  return (
    <div style={{ background: T.paper, border: `1px solid ${T.line}`, borderRadius: 10, boxShadow: 'var(--shadow-1)', ...style }}>
      {children}
    </div>
  )
}

// The honest placeholder body shown inside a not-yet-bound view. It clearly
// states the view is scaffolded and shows skeleton rows so the layout reads as
// "loading the shape" rather than "broken / empty".
function PlaceholderBody({ note, dataSlot, rows = 3 }: { note: string; dataSlot: string; rows?: number }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
      <Card style={{ padding: 18 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 12 }}>
          <Chip>Coming together</Chip>
          <span style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.5 }}>{note}</span>
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
          {Array.from({ length: rows }).map((_, i) => (
            <div key={i} style={{ display: 'grid', gridTemplateColumns: '32px 1fr 90px', gap: 12, alignItems: 'center' }}>
              <Skeleton width={32} height={32} radius={8} />
              <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
                <Skeleton width={`${60 - i * 8}%`} height={12} />
                <Skeleton width={`${40 - i * 5}%`} height={10} />
              </div>
              <Skeleton width={70} height={12} />
            </div>
          ))}
        </div>
      </Card>
      <div style={{ fontSize: 11, fontFamily: T.fontMono, color: T.ink4, letterSpacing: '0.02em' }}>
        DATA SLOT — {dataSlot}
      </div>
    </div>
  )
}

// ── Signed-out gate ─────────────────────────────────────────────────────────

function SignedOutGate({ onOpenSignIn }: { onOpenSignIn: () => void }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', flex: 1, padding: '28px 36px', gap: 10 }}>
      <NavIcon name="lock" size={26} color={T.ink3} />
      <div style={{ fontSize: 16, fontWeight: 600, color: T.ink, marginTop: 6 }}>Sign in to Beebeeb</div>
      <div style={{ fontSize: 12, color: T.ink3, textAlign: 'center' as const, lineHeight: 1.6, maxWidth: 320 }}>
        Your files are encrypted on this PC before they leave for Falkenstein. Sign in to see your vault, devices, and account.
      </div>
      <div style={{ marginTop: 8 }}>
        <PrimaryBtn onClick={onOpenSignIn}>Sign in</PrimaryBtn>
      </div>
    </div>
  )
}

// ── Views ───────────────────────────────────────────────────────────────────

// HOME — a status dashboard. The metric strip is wired to the live sync status;
// the rest is scaffolded. DATA SLOT: accountProfile, accountUsage,
// accountActivity for the welcome line + at-a-glance security/recent rows.
function HomeView({ status, usage, storage }: { status: SyncStatus | null; usage: BillingUsage | null; storage: StorageSummary | null }) {
  const stateLabel =
    status == null ? '…'
    : !status.logged_in ? 'Signed out'
    : status.engine === 'running' ? (status.syncing > 0 ? 'Syncing' : 'Up to date')
    : 'Paused'

  const usedBytes = usage?.used_bytes ?? storage?.used_bytes ?? null
  const quotaBytes = usage?.quota_bytes ?? storage?.quota_bytes ?? null

  const metrics: Array<{ k: string; v: string }> = [
    { k: 'State', v: stateLabel },
    { k: 'Syncing', v: status ? String(status.syncing) : '…' },
    { k: 'Conflicts', v: status ? String(status.conflicts) : '…' },
    { k: 'In vault', v: usedBytes != null ? formatBytes(usedBytes) : '…' },
  ]

  return (
    <div style={{ overflow: 'auto', padding: '28px 36px', flex: 1 }}>
      <PageHeader
        title="Home"
        subtitle="Everything on this PC, at a glance. Files are encrypted on your device before upload."
        aside={<Chip tone="green"><span style={{ width: 6, height: 6, borderRadius: '50%', background: T.green, display: 'inline-block' }} /> End-to-end encrypted</Chip>}
      />

      {/* Live status strip */}
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
        {metrics.map(({ k, v }) => (
          <div key={k}>
            <div style={{ fontSize: 9.5, fontFamily: T.fontMono, textTransform: 'uppercase' as const, letterSpacing: '0.07em', color: T.ink3, marginBottom: 4 }}>{k}</div>
            <div style={{ fontSize: 16, fontWeight: 600, fontFamily: T.fontMono, color: T.ink }}>{v}</div>
          </div>
        ))}
      </div>

      {/* Storage line (live when available) */}
      <Card style={{ padding: 18, marginBottom: 14 }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 10 }}>
          <div style={{ fontSize: 13, fontWeight: 600, color: T.ink }}>Storage</div>
          {quotaBytes != null && usedBytes != null && (
            <span style={{ fontSize: 11.5, fontFamily: T.fontMono, color: T.ink3 }}>
              {formatBytes(usedBytes)} / {formatBytes(quotaBytes)}
            </span>
          )}
        </div>
        {quotaBytes != null && usedBytes != null ? (
          <div className="progress-track">
            <div className="progress-fill" style={{ width: `${Math.min(100, quotaBytes > 0 ? (usedBytes / quotaBytes) * 100 : 0)}%` }} />
          </div>
        ) : (
          <Skeleton height={8} radius={999} />
        )}
      </Card>

      {/* Recent activity placeholder */}
      <PlaceholderBody
        note="Your recent activity and security at-a-glance land here."
        dataSlot="accountActivity() · accountSecurityScore()"
        rows={3}
      />
    </div>
  )
}

// FILES — the link to File Explorer (the real file surface on Windows). Wired to
// open the sync-folder location. DATA SLOT (later): a richer in-app browser may
// consume desktop vault listing IPCs; for now Explorer is the surface.
function FilesView() {
  const [root, setRoot] = useState<string | null>(null)
  const [opening, setOpening] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    command<string | null>('default_sync_root').then((r) => {
      if (!cancelled && r.ok) setRoot(r.value)
    })
    return () => { cancelled = true }
  }, [])

  const openInExplorer = async () => {
    setOpening(true)
    setError(null)
    const r = await command<void>('open_finder_location', { path: root })
    setOpening(false)
    if (!r.ok) setError(r.unsupported ? commandUnavailableLabel('open_finder_location') : r.reason)
  }

  return (
    <div style={{ overflow: 'auto', padding: '28px 36px', flex: 1 }}>
      <PageHeader
        title="Files"
        subtitle="On Windows, File Explorer is your file surface — Beebeeb appears there as a sync folder. Explorer only ever shows the decrypted view."
      />
      <Card style={{ padding: 18 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 14 }}>
          <div style={{ width: 36, height: 36, borderRadius: 9, background: T.amberBg, border: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
            <NavIcon name="folder" size={16} color={T.amberDeep} />
          </div>
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ fontSize: 13, fontWeight: 600, color: T.ink, marginBottom: 2 }}>Beebeeb sync folder</div>
            <div style={{ fontSize: 11.5, fontFamily: T.fontMono, color: T.ink3, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' as const }}>
              {root ?? 'Not configured on this PC yet'}
            </div>
          </div>
          <PrimaryBtn onClick={() => void openInExplorer()} disabled={opening || root == null}>
            <NavIcon name="external" size={13} color={T.paper} /> {opening ? 'Opening…' : 'Open in Explorer'}
          </PrimaryBtn>
        </div>
        {error && <div style={{ marginTop: 12, fontSize: 11.5, color: 'oklch(0.5 0.18 25)' }}>{error}</div>}
      </Card>
    </div>
  )
}

// SETTINGS — open the dedicated Windows settings window (keeps Settings reachable
// from inside the app). The settings window holds sync, explorer integration,
// launch, updates, etc.
function SettingsView() {
  const [error, setError] = useState<string | null>(null)
  const open = async () => {
    setError(null)
    const r = await command<void>('show_settings_window')
    if (!r.ok) setError(r.unsupported ? commandUnavailableLabel('show_settings_window') : r.reason)
  }
  return (
    <div style={{ overflow: 'auto', padding: '28px 36px', flex: 1 }}>
      <PageHeader
        title="Settings"
        subtitle="Sync rules, Explorer integration, launch-at-login, and updates live in the Settings window."
      />
      <Card style={{ padding: 18 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 14 }}>
          <div style={{ width: 36, height: 36, borderRadius: 9, background: T.paper2, border: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
            <NavIcon name="cog" size={16} color={T.ink2} />
          </div>
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ fontSize: 13, fontWeight: 600, color: T.ink, marginBottom: 2 }}>Open Settings</div>
            <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.4 }}>Sync, bandwidth, Explorer integration, launch, and updates.</div>
          </div>
          <PrimaryBtn onClick={() => void open()}>Open Settings</PrimaryBtn>
        </div>
        {error && <div style={{ marginTop: 12, fontSize: 11.5, color: 'oklch(0.5 0.18 25)' }}>{error}</div>}
      </Card>
    </div>
  )
}

// Placeholder views — each declares the data wrapper(s) the next package binds.
const PLACEHOLDER_VIEWS: Record<string, { title: string; subtitle: string; note: string; dataSlot: string }> = {
  account: {
    title: 'Account',
    subtitle: 'Your profile, plan, and subscription.',
    note: 'Email, plan, and subscription details land here.',
    dataSlot: 'accountProfile() · accountSubscription()',
  },
  insights: {
    title: 'Insights',
    subtitle: 'Where your storage goes — by content type and largest files.',
    note: 'Storage breakdown by Media / Documents / Other plus your largest files.',
    dataSlot: 'accountUsage() · accountStorageBreakdown()',
  },
  bandwidth: {
    title: 'Bandwidth',
    subtitle: 'Upload and download activity and per-transfer limits.',
    note: 'Live transfer rates and limits land here.',
    dataSlot: 'accountClientSessions() (speed_bps) · desktop_config',
  },
  'selective-sync': {
    title: 'Selective sync',
    subtitle: 'Pick which folders live on this PC.',
    note: 'The folder tree with online-only / pinned toggles lands here.',
    dataSlot: 'list_vault_folders · set_recursive_pin',
  },
  devices: {
    title: 'Devices',
    subtitle: 'Every device with access to your vault, and its sync sessions.',
    note: 'Registered devices and per-device sync sessions land here.',
    dataSlot: 'accountDevices() · accountClientSessions()',
  },
  security: {
    title: 'Security',
    subtitle: 'Your security score, sessions, and recovery posture.',
    note: 'Security score, active sessions, and factors land here.',
    dataSlot: 'accountSecurityScore() · accountSessionList()',
  },
  activity: {
    title: 'Activity',
    subtitle: 'A full audit log of what happened on your account.',
    note: 'The paginated audit-log feed lands here.',
    dataSlot: 'accountActivityFeed()',
  },
}

function PlaceholderView({ navId }: { navId: NavId }) {
  const meta = PLACEHOLDER_VIEWS[navId]
  if (!meta) {
    return (
      <div style={{ overflow: 'auto', padding: '28px 36px', flex: 1 }}>
        <PageHeader title={navId} />
      </div>
    )
  }
  return (
    <div style={{ overflow: 'auto', padding: '28px 36px', flex: 1 }}>
      <PageHeader title={meta.title} subtitle={meta.subtitle} />
      <PlaceholderBody note={meta.note} dataSlot={meta.dataSlot} rows={navId === 'activity' || navId === 'devices' ? 5 : 3} />
    </div>
  )
}

// ── Sidebar storage widget ──────────────────────────────────────────────────

function StorageWidget({ usage, storage, onUpgrade }: { usage: BillingUsage | null; storage: StorageSummary | null; onUpgrade: () => void }) {
  const usedBytes = usage?.used_bytes ?? storage?.used_bytes ?? null
  const quotaBytes = usage?.quota_bytes ?? storage?.quota_bytes ?? null
  const pct = usage?.percentage != null
    ? usage.percentage * 100
    : (usedBytes != null && quotaBytes != null && quotaBytes > 0 ? (usedBytes / quotaBytes) * 100 : null)

  return (
    <div style={{ marginTop: 'auto', padding: '14px 12px 10px', borderTop: `1px solid ${T.line}` }}>
      <div style={{ fontSize: 9.5, fontFamily: T.fontMono, textTransform: 'uppercase' as const, letterSpacing: '0.07em', color: T.ink3, marginBottom: 8 }}>
        Storage
      </div>
      {pct != null ? (
        <div className="progress-track" style={{ marginBottom: 6 }}>
          <div className="progress-fill" style={{ width: `${Math.min(100, Math.max(0, pct))}%` }} />
        </div>
      ) : (
        <div style={{ marginBottom: 6 }}><Skeleton height={8} radius={999} /></div>
      )}
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', fontSize: 11 }}>
        <span style={{ fontFamily: T.fontMono, color: T.ink3 }}>
          {usedBytes != null && quotaBytes != null ? `${formatBytes(usedBytes)} / ${formatBytes(quotaBytes)}` : '…'}
        </span>
        <button
          onClick={onUpgrade}
          style={{ background: 'transparent', border: 'none', padding: 0, cursor: 'pointer', color: T.amberDeep, fontWeight: 500, fontSize: 11, fontFamily: T.fontSans }}
        >
          Upgrade
        </button>
      </div>
      <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginTop: 12 }}>
        <NavIcon name="shield" size={11} color={T.amberDeep} />
        <span style={{ fontSize: 10.5, fontFamily: T.fontMono, color: T.ink3, textTransform: 'uppercase' as const, letterSpacing: '0.04em' }}>
          Falkenstein · Hetzner
        </span>
      </div>
    </div>
  )
}

// ── Root component ──────────────────────────────────────────────────────────

export default function WindowsApp() {
  const [activeNav, setActiveNav] = useState<NavId>('home')
  const [status, setStatus] = useState<SyncStatus | null>(null)
  const [usage, setUsage] = useState<BillingUsage | null>(null)
  const [storage, setStorage] = useState<StorageSummary | null>(null)
  // Avoid refetching billing usage every poll once we have it.
  const usageLoaded = useRef(false)

  // Poll sync status (drives the auth gate + Home metrics).
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

  const loggedIn = status?.logged_in ?? false

  // Load billing usage once signed in (storage widget + Home). Fall back to the
  // local desktop_storage_summary so the widget is never blank when the billing
  // endpoint is unavailable in this build.
  useEffect(() => {
    if (!loggedIn || usageLoaded.current) return
    let cancelled = false
    void (async () => {
      const r = await accountUsage()
      if (cancelled) return
      if (r.ok) {
        setUsage(r.value)
        usageLoaded.current = true
        return
      }
      // Fallback: local mirror summary.
      const s = await command<StorageSummary>('desktop_storage_summary')
      if (!cancelled && s.ok) setStorage(s.value)
    })()
    return () => { cancelled = true }
  }, [loggedIn])

  // When signed out, snap back to an always-accessible view.
  useEffect(() => {
    if (!loggedIn && !ALWAYS_ACCESSIBLE.has(activeNav)) setActiveNav('home')
  }, [loggedIn, activeNav])

  const openSignIn = () => { void command<void>('open_onboarding_window') }
  const openUpgrade = () => { void openUrl('https://app.beebeeb.io/billing') }

  const filteredSections = NAV_SECTIONS.map((section) => ({
    ...section,
    items: section.items.filter((item) => loggedIn || ALWAYS_ACCESSIBLE.has(item.id)),
  })).filter((section) => section.items.length > 0)

  const renderContent = () => {
    if (!loggedIn && !ALWAYS_ACCESSIBLE.has(activeNav)) {
      return <SignedOutGate onOpenSignIn={openSignIn} />
    }
    switch (activeNav) {
      case 'home':
        return loggedIn ? <HomeView status={status} usage={usage} storage={storage} /> : <SignedOutGate onOpenSignIn={openSignIn} />
      case 'files':
        return <FilesView />
      case 'settings':
        return <SettingsView />
      default:
        return <PlaceholderView navId={activeNav} />
    }
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100vh', fontFamily: T.fontSans, color: T.ink, background: T.paper, overflow: 'hidden' }}>
      {/* Keyframes for the skeleton shimmer (scoped, inline). */}
      <style>{'@keyframes bb-shimmer { 0% { background-position: 200% 0; } 100% { background-position: -200% 0; } }'}</style>
      <UpdateBanner />
      <div style={{ display: 'grid', gridTemplateColumns: '232px 1fr', flex: 1, overflow: 'hidden' }}>
        {/* Sidebar */}
        <div style={{ background: T.paper2, borderRight: `1px solid ${T.line}`, padding: '16px 10px 0', overflow: 'auto', display: 'flex', flexDirection: 'column' }}>
          {/* Brand */}
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '4px 8px 14px' }}>
            <div style={{ width: 24, height: 24, borderRadius: 6, background: T.amber, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
              <span style={{ fontWeight: 800, fontSize: 12, color: T.ink, lineHeight: 1 }}>b</span>
            </div>
            <span style={{ fontSize: 13, fontWeight: 600, letterSpacing: '-0.01em', color: T.ink }}>beebeeb.io</span>
          </div>

          {/* Nav sections */}
          {filteredSections.map((section) => (
            <div key={section.heading} style={{ marginBottom: 10 }}>
              <div style={{ fontSize: 9.5, fontFamily: T.fontMono, textTransform: 'uppercase' as const, letterSpacing: '0.07em', color: T.ink3, padding: '4px 10px', marginBottom: 2 }}>
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
                    <NavIcon name={item.icon} size={12} color={active ? T.amberDeep : T.ink3} />
                    {item.label}
                  </button>
                )
              })}
            </div>
          ))}

          {/* Storage widget (auth-gated content; shown only when signed in) */}
          {loggedIn ? (
            <StorageWidget usage={usage} storage={storage} onUpgrade={openUpgrade} />
          ) : (
            <div style={{ marginTop: 'auto', padding: '14px 12px 10px', borderTop: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', gap: 6 }}>
              <NavIcon name="shield" size={11} color={T.amberDeep} />
              <span style={{ fontSize: 10.5, fontFamily: T.fontMono, color: T.ink3, textTransform: 'uppercase' as const, letterSpacing: '0.04em' }}>
                Falkenstein · Hetzner
              </span>
            </div>
          )}
        </div>

        {/* Content */}
        {renderContent()}
      </div>
    </div>
  )
}
