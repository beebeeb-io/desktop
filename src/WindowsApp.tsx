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
import { T, NavIcon, Chip, PrimaryBtn, Skeleton, PageHeader, Card } from './windows/ui'
import AccountView from './windows/views/AccountView'
import InsightsView from './windows/views/InsightsView'
import BandwidthView from './windows/views/BandwidthView'
import SelectiveSyncView from './windows/views/SelectiveSyncView'
import DevicesView from './windows/views/DevicesView'
import SecurityView from './windows/views/SecurityView'
import ActivityView from './windows/views/ActivityView'

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
      case 'account':
        return <AccountView />
      case 'insights':
        return <InsightsView />
      case 'devices':
        return <DevicesView />
      case 'security':
        return <SecurityView />
      case 'activity':
        return <ActivityView />
      case 'bandwidth':
        return <BandwidthView />
      case 'selective-sync':
        return <SelectiveSyncView />
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
