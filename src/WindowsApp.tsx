/**
 * WindowsApp — the main Beebeeb app window for Windows (PKG-SHELL).
 *
 * Mounted when ?window=main-app&platform=windows (see main.tsx). Opened by the
 * Rust `show_main_app_window` command — the tray "Open Beebeeb" item/button and
 * the onboarding "Open control center" button both route here, and it's the
 * window shown automatically on launch once the PC is configured.
 *
 * This is the SHELL: a left sidebar (brand mark + nav groups + a live storage
 * widget) and a content area with a state-based router. It hosts the data-backed views that
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
 * Design tokens + idioms are shared through `src/windows/ui.tsx`. Brand: amber
 * for encryption state + the active nav icon accent only; Inter for humans,
 * JetBrains Mono for ids/sizes; name the city only ("Falkenstein"), never the
 * storage provider; honest voice; no emojis.
 */

import { useEffect, useRef, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import {
  command,
  commandUnavailableLabel,
  formatBytes,
  loadSyncStatus,
  openUrl,
  accountUsage,
  accountProfile,
  accountActivity,
  desktopFileOverview,
  getKnownFolderBackup,
  setKnownFolderBackup,
  type KnownFolderStatus,
  type BillingUsage,
  type StorageSummary,
  type SyncStatus,
  type AccountProfile,
  type AccountActivity,
  type AccountActivityEvent,
  type FileOverview,
  type StatusBucket,
  type RecentFile,
} from './desktopApi'
import UpdateBanner from './UpdateBanner'
import { T, NavIcon, Chip, PrimaryBtn, Skeleton, PageHeader, Card } from './windows/ui'
import KnownFolderOnboarding from './windows/KnownFolderOnboarding'
import AccountView from './windows/views/AccountView'
import InsightsView from './windows/views/InsightsView'
import BandwidthView from './windows/views/BandwidthView'
import SelectiveSyncView from './windows/views/SelectiveSyncView'
import DevicesView from './windows/views/DevicesView'
import SecurityView from './windows/views/SecurityView'
import ActivityView from './windows/views/ActivityView'
import SettingsView from './windows/views/SettingsView'
import { useRegionLabel } from './windows/useRegion'

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
const WINDOWS_APP_NAV_EVENT = 'windows-app:navigate'

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

function isNavId(value: unknown): value is NavId {
  return (
    value === 'home' ||
    value === 'files' ||
    value === 'account' ||
    value === 'insights' ||
    value === 'bandwidth' ||
    value === 'selective-sync' ||
    value === 'devices' ||
    value === 'security' ||
    value === 'activity' ||
    value === 'settings'
  )
}

function initialNav(): NavId {
  const nav = new URLSearchParams(window.location.search).get('nav')
  return isNavId(nav) ? nav : 'home'
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

// ── HomeView helpers ─────────────────────────────────────────────────────────

/** Compact relative time — same logic as ActivityView, kept local so HomeView
 *  has no shared-module dep; both files stay standalone. */
function homeRelativeTime(value?: string | null): string {
  if (!value) return '—'
  const d = new Date(value)
  if (Number.isNaN(d.getTime())) return '—'
  const sec = Math.round((Date.now() - d.getTime()) / 1000)
  if (sec < 0) return 'just now'
  if (sec < 45) return 'just now'
  const min = Math.round(sec / 60)
  if (min < 60) return `${min} min ago`
  const hr = Math.round(min / 60)
  if (hr < 24) return `${hr}h ago`
  const day = Math.round(hr / 24)
  if (day < 7) return `${day}d ago`
  return d.toLocaleDateString(undefined, { day: '2-digit', month: 'short', year: 'numeric' })
}

/** Icon + accent dot for a recent-activity row. Amber for security events;
 *  neutral otherwise — matches ActivityView's convention so nothing looks random. */
function homeEventVisual(type: string): { icon: string; dot: string } {
  const t = (type || '').toLowerCase()
  if (t.includes('login') || t.includes('signin') || t.includes('sign_in') || t.includes('device'))
    return { icon: 'device', dot: T.amber }
  if (t.includes('key') || t.includes('recovery') || t.includes('password') || t.includes('security') || t.includes('totp'))
    return { icon: 'shield', dot: T.amberDeep }
  if (t.includes('session') || t.includes('revoke') || t.includes('lock'))
    return { icon: 'lock', dot: T.ink }
  if (t.includes('share'))
    return { icon: 'cloud', dot: T.green }
  if (t.includes('upload') || t.includes('backup') || t.includes('sync') || t.includes('file'))
    return { icon: 'folder', dot: T.green }
  return { icon: 'clock', dot: T.ink3 }
}

function homeHumanizeType(type: string): string {
  const raw = (type || '').replace(/[._]+/g, ' ').trim()
  if (!raw) return 'Activity'
  return raw.charAt(0).toUpperCase() + raw.slice(1)
}

// HOME — a status dashboard. Metric strip wired to live sync status. Body
// self-fetches accountProfile + accountActivity on mount. Degrades gracefully
// when the API is unavailable (login down in this build): metric strip stays,
// account body shows a calm signed-out prompt.
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

  // ── Self-fetch: profile + activity ────────────────────────────────────────
  type HomeDataState =
    | { phase: 'loading' }
    | { phase: 'unavailable' }   // unsupported build OR auth error — degrade quietly
    | { phase: 'loaded'; profile: AccountProfile; activity: AccountActivity | null }

  const [dataState, setDataState] = useState<HomeDataState>({ phase: 'loading' })

  useEffect(() => {
    let cancelled = false
    setDataState({ phase: 'loading' })
    void (async () => {
      const [p, a] = await Promise.all([accountProfile(), accountActivity()])
      if (cancelled) return
      if (!p.ok) {
        // Profile failed — not signed in or API unavailable. Degrade quietly.
        setDataState({ phase: 'unavailable' })
        return
      }
      setDataState({
        phase: 'loaded',
        profile: p.value,
        activity: a.ok ? a.value : null,
      })
    })()
    return () => { cancelled = true }
  }, [])

  // ── Body sections rendered below the live strips ──────────────────────────

  let accountBody: React.ReactNode

  if (dataState.phase === 'loading') {
    // Skeleton rows matching the two cards we'll render
    accountBody = (
      <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
        {/* Welcome skeleton */}
        <Card style={{ padding: 18 }}>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
            <Skeleton width="38%" height={12} />
            <Skeleton width="55%" height={14} />
          </div>
        </Card>
        {/* Stats + activity skeleton */}
        <Card style={{ padding: 0, overflow: 'hidden' }}>
          <div style={{ padding: '13px 18px', borderBottom: `1px solid ${T.line}` }}>
            <Skeleton width={110} height={12} />
          </div>
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 0 }}>
            {[0, 1, 2, 3].map((i) => (
              <div key={i} style={{ padding: '16px 18px', borderRight: i < 3 ? `1px solid ${T.line}` : 'none' }}>
                <div style={{ marginBottom: 8 }}><Skeleton width="60%" height={10} /></div>
                <Skeleton width="40%" height={14} />
              </div>
            ))}
          </div>
        </Card>
        <Card style={{ padding: 0, overflow: 'hidden' }}>
          <div style={{ padding: '13px 18px', borderBottom: `1px solid ${T.line}` }}>
            <Skeleton width={120} height={12} />
          </div>
          {[0, 1, 2, 3].map((i) => (
            <div key={i} style={{ padding: '12px 18px', display: 'flex', alignItems: 'flex-start', gap: 12, borderBottom: i < 3 ? `1px solid ${T.line}` : 'none' }}>
              <Skeleton width={28} height={28} radius={999} />
              <div style={{ flex: 1, display: 'flex', flexDirection: 'column', gap: 6 }}>
                <Skeleton width={`${58 - i * 8}%`} height={12} />
                <Skeleton width={`${42 - i * 6}%`} height={10} />
              </div>
              <Skeleton width={52} height={10} />
            </div>
          ))}
        </Card>
      </div>
    )
  } else if (dataState.phase === 'unavailable') {
    // API down / not signed in. Don't show an error — just a calm prompt.
    // The metric strip above already tells the sync story.
    accountBody = (
      <Card style={{ padding: 20 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
          <div style={{ width: 32, height: 32, borderRadius: 8, background: T.paper2, border: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
            <NavIcon name="user" size={15} color={T.ink3} />
          </div>
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ fontSize: 13, fontWeight: 600, color: T.ink, marginBottom: 3 }}>Sign in to see your account activity</div>
            <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.6 }}>
              Active sessions, recent sign-ins, and security events appear here once you're signed in.
            </div>
          </div>
        </div>
      </Card>
    )
  } else {
    // Loaded — render welcome + stats + recent activity
    const { profile, activity } = dataState
    const summary = activity?.summary ?? null
    const events: AccountActivityEvent[] = (activity?.events ?? []).slice(0, 4)

    accountBody = (
      <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>

        {/* Welcome line */}
        <Card style={{ padding: 18 }}>
          <div style={{ fontSize: 11.5, color: T.ink3, marginBottom: 4 }}>Welcome back</div>
          <div style={{
            fontSize: 13.5,
            fontFamily: T.fontMono,
            color: T.ink,
            fontWeight: 600,
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap' as const,
          }}>
            {profile.email}
          </div>
        </Card>

        {/* At-a-glance security stats */}
        {summary && (
          <Card style={{ padding: 0, overflow: 'hidden' }}>
            <div style={{ padding: '12px 18px', borderBottom: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', gap: 8 }}>
              <NavIcon name="shield" size={13} color={T.ink2} />
              <span style={{ fontSize: 12.5, fontWeight: 600, color: T.ink }}>Account at a glance</span>
            </div>
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, minmax(0, 1fr))' }}>
              {[
                { k: 'Sessions', v: String(summary.active_sessions) },
                { k: 'Shares', v: String(summary.active_shares) },
                { k: 'Security', v: summary.security_score },
                {
                  k: 'Last sign-in',
                  v: summary.last_login_at ? homeRelativeTime(summary.last_login_at) : '—',
                  sub: summary.last_login_device ?? null,
                },
              ].map(({ k, v, sub }, i) => (
                <div key={k} style={{
                  padding: '16px 18px',
                  borderRight: i < 3 ? `1px solid ${T.line}` : 'none',
                }}>
                  <div style={{ fontSize: 9.5, fontFamily: T.fontMono, textTransform: 'uppercase' as const, letterSpacing: '0.07em', color: T.ink3, marginBottom: 5 }}>
                    {k}
                  </div>
                  <div style={{ fontSize: 13, fontWeight: 600, fontFamily: T.fontMono, color: T.ink }}>
                    {v}
                  </div>
                  {sub && (
                    <div style={{ fontSize: 10.5, fontFamily: T.fontMono, color: T.ink4, marginTop: 3, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' as const }}>
                      {sub}
                    </div>
                  )}
                </div>
              ))}
            </div>
          </Card>
        )}

        {/* Recent activity */}
        <Card style={{ padding: 0, overflow: 'hidden' }}>
          <div style={{ padding: '12px 18px', borderBottom: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
              <NavIcon name="clock" size={13} color={T.ink2} />
              <span style={{ fontSize: 12.5, fontWeight: 600, color: T.ink }}>Recent activity</span>
            </div>
            <span style={{ fontSize: 11, color: T.ink3 }}>Activity tab for the full log</span>
          </div>

          {events.length === 0 ? (
            <div style={{ padding: '22px 18px', display: 'flex', alignItems: 'center', gap: 10 }}>
              <NavIcon name="clock" size={14} color={T.ink4} />
              <span style={{ fontSize: 12, color: T.ink3 }}>No recent activity</span>
            </div>
          ) : events.map((ev, i) => {
            const vis = homeEventVisual(ev.type)
            const last = i === events.length - 1
            return (
              <div
                key={ev.id || `ev-${i}`}
                style={{ padding: '11px 18px', display: 'flex', alignItems: 'center', gap: 12, borderBottom: last ? 'none' : `1px solid ${T.line}` }}
              >
                <div style={{ width: 26, height: 26, borderRadius: '50%', background: T.paper2, border: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0, position: 'relative' }}>
                  <NavIcon name={vis.icon} size={12} color={T.ink2} />
                  <span style={{ position: 'absolute', top: -1, right: -1, width: 6, height: 6, borderRadius: '50%', background: vis.dot, border: `1.5px solid ${T.paper}` }} />
                </div>
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ fontSize: 12.5, color: T.ink, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' as const }}>
                    {ev.description?.trim() || homeHumanizeType(ev.type)}
                  </div>
                </div>
                <span style={{ fontSize: 10.5, fontFamily: T.fontMono, color: T.ink4, flexShrink: 0, whiteSpace: 'nowrap' as const }}>
                  {homeRelativeTime(ev.created_at)}
                </span>
              </div>
            )
          })}
        </Card>

      </div>
    )
  }

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

      {/* Account body: welcome + stats + recent activity (or quiet degraded state) */}
      {accountBody}
    </div>
  )
}

// FILES — the per-PC SYNC-STATE dashboard. The device lens: how many files live
// on this PC vs online-only vs pinned vs conflicting, the most-recently-changed
// files, and the Open-in-Explorer card (Explorer stays the real file surface on
// Windows). COMPLEMENTS InsightsView, which is the by-TYPE / billing lens — this
// view never duplicates that by-type breakdown.
//
// Data: desktopFileOverview() (local, from the SQLite mirror). All four states
// (loading / error / empty / loaded) handled — pattern copied from InsightsView.

const FILES_RECENT_LIMIT = 15
const FILES_DANGER = 'oklch(0.5 0.18 25)'

// Stable display order + human labels for the sync statuses. The DTO emits a
// bucket only for statuses that have files, so the UI owns the legend and treats
// a missing status as zero.
const STATUS_ORDER: RecentFile['status'][] = [
  'local',
  'cloud_only',
  'downloading',
  'uploading',
  'conflict',
  'error',
]
const STATUS_LABEL: Record<string, string> = {
  local: 'On this PC',
  cloud_only: 'Online-only',
  downloading: 'Downloading',
  uploading: 'Uploading',
  conflict: 'Conflict',
  error: 'Error',
}
// Status colors are NEUTRAL/GREEN/RED only — amber is reserved for encryption
// state + primary actions (file-header brand note). Synced/local reads green,
// conflict/error red, everything in-flight or cloud-only stays neutral ink.
const STATUS_COLOR: Record<string, string> = {
  local: T.green,
  cloud_only: T.ink3,
  downloading: T.ink3,
  uploading: T.ink3,
  conflict: FILES_DANGER,
  error: FILES_DANGER,
}

function statusLabel(status: string): string {
  return STATUS_LABEL[status] ?? status
}
function statusColor(status: string): string {
  return STATUS_COLOR[status] ?? T.ink3
}

// basename of a forward- or back-slash path.
function baseName(path: string): string {
  const parts = path.split(/[/\\]/)
  return parts[parts.length - 1] || path
}

// Relative time from a Unix-epoch (seconds) timestamp. Coarse on purpose —
// "just now" / "3m ago" / "2h ago" / "5d ago" / a short date for older.
function relativeTime(epochSecs: number): string {
  if (!Number.isFinite(epochSecs) || epochSecs <= 0) return '—'
  const deltaSecs = Math.max(0, Math.floor(Date.now() / 1000 - epochSecs))
  if (deltaSecs < 45) return 'just now'
  const mins = Math.floor(deltaSecs / 60)
  if (mins < 60) return `${mins}m ago`
  const hours = Math.floor(mins / 60)
  if (hours < 24) return `${hours}h ago`
  const days = Math.floor(hours / 24)
  if (days < 7) return `${days}d ago`
  return new Date(epochSecs * 1000).toLocaleDateString('en-US', { month: 'short', day: 'numeric' })
}

// A small status pill (mirrors InsightsView's category Chip idiom, but colored
// by sync status, never amber). A leading dot carries the status color so the
// pill text can stay legible neutral ink.
function StatusPill({ status }: { status: string }) {
  return (
    <Chip tone="neutral">
      <span style={{ display: 'inline-block', width: 6, height: 6, borderRadius: 999, background: statusColor(status) }} />
      {statusLabel(status)}
    </Chip>
  )
}

// ── Sub-card: Open-in-Explorer (kept from the previous FilesView) ─────────────

function SyncFolderCard() {
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
    <Card style={{ padding: 18, marginBottom: 14 }}>
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
      {error && <div style={{ marginTop: 12, fontSize: 11.5, color: FILES_DANGER }}>{error}</div>}
    </Card>
  )
}

// ── Section: metric strip ─────────────────────────────────────────────────────

function bucketFor(buckets: StatusBucket[], status: string): StatusBucket | undefined {
  return buckets.find((b) => b.status === status)
}

function FilesMetricStrip({ overview }: { overview: FileOverview }) {
  const local = bucketFor(overview.by_status, 'local')
  const cloud = bucketFor(overview.by_status, 'cloud_only')
  const conflict = bucketFor(overview.by_status, 'conflict')

  const metrics: { label: string; value: string; sub: string; color: string }[] = [
    {
      label: 'On this PC',
      value: (local?.count ?? 0).toLocaleString('en-US'),
      sub: formatBytes(local?.bytes ?? 0),
      color: T.green,
    },
    {
      label: 'Online-only',
      value: (cloud?.count ?? 0).toLocaleString('en-US'),
      sub: formatBytes(cloud?.bytes ?? 0),
      color: T.ink3,
    },
    {
      label: 'Pinned',
      value: overview.pinned_count.toLocaleString('en-US'),
      sub: formatBytes(overview.pinned_bytes),
      color: T.ink2,
    },
    {
      label: 'Conflicts',
      value: (conflict?.count ?? 0).toLocaleString('en-US'),
      sub: conflict && conflict.count > 0 ? 'needs attention' : 'none',
      color: conflict && conflict.count > 0 ? FILES_DANGER : T.ink3,
    },
    {
      label: 'In vault',
      value: overview.total_files.toLocaleString('en-US'),
      sub: formatBytes(overview.total_bytes),
      color: T.ink,
    },
  ]

  return (
    <Card style={{ padding: 0, marginBottom: 14 }}>
      <div style={{ display: 'grid', gridTemplateColumns: `repeat(${metrics.length}, 1fr)` }}>
        {metrics.map((m, i) => (
          <div
            key={m.label}
            style={{
              padding: '16px 18px',
              borderLeft: i === 0 ? 'none' : `1px solid ${T.line}`,
              minWidth: 0,
            }}
          >
            <div style={{ fontSize: 9.5, fontFamily: T.fontMono, textTransform: 'uppercase' as const, letterSpacing: '0.07em', color: T.ink3, marginBottom: 8 }}>
              {m.label}
            </div>
            <div style={{ fontSize: 22, fontWeight: 700, fontFamily: T.fontMono, letterSpacing: '-0.02em', color: m.color, lineHeight: 1.05 }}>
              {m.value}
            </div>
            <div style={{ fontSize: 10.5, fontFamily: T.fontMono, color: T.ink3, marginTop: 4 }}>
              {m.sub}
            </div>
          </div>
        ))}
      </div>
    </Card>
  )
}

// ── Section: by-status proportional bar ──────────────────────────────────────

function ByStatusBar({ overview }: { overview: FileOverview }) {
  const total = overview.total_files
  // Render in stable order; only statuses with files contribute a segment.
  const segments = STATUS_ORDER.map((status) => {
    const bucket = bucketFor(overview.by_status, status)
    return { status, count: bucket?.count ?? 0 }
  }).filter((s) => s.count > 0)

  return (
    <Card style={{ padding: 22, marginBottom: 14 }}>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 14 }}>
        <div style={{ fontSize: 13, fontWeight: 600, color: T.ink }}>By sync state</div>
        <span style={{ fontSize: 11, fontFamily: T.fontMono, color: T.ink3 }}>
          {total.toLocaleString('en-US')} {total === 1 ? 'file' : 'files'}
        </span>
      </div>

      <div style={{ display: 'flex', width: '100%', height: 10, borderRadius: 999, overflow: 'hidden', background: T.paper3 }}>
        {segments.map((s) => (
          <div
            key={s.status}
            title={`${statusLabel(s.status)} · ${s.count.toLocaleString('en-US')}`}
            style={{
              width: `${total > 0 ? (s.count / total) * 100 : 0}%`,
              height: '100%',
              background: statusColor(s.status),
              transition: 'width 200ms ease',
            }}
          />
        ))}
      </div>

      <div style={{ display: 'flex', flexWrap: 'wrap' as const, gap: '8px 16px', marginTop: 14 }}>
        {segments.map((s) => (
          <div key={s.status} style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
            <span style={{ display: 'inline-block', width: 8, height: 8, borderRadius: 999, background: statusColor(s.status) }} />
            <span style={{ fontSize: 11.5, color: T.ink2 }}>{statusLabel(s.status)}</span>
            <span style={{ fontSize: 11, fontFamily: T.fontMono, color: T.ink3 }}>
              {s.count.toLocaleString('en-US')}
            </span>
          </div>
        ))}
      </div>
    </Card>
  )
}

// ── Section: recently changed ─────────────────────────────────────────────────

function RecentlyChanged({ files }: { files: RecentFile[] }) {
  return (
    <Card style={{ padding: 0 }}>
      <div style={{ padding: '16px 22px 12px', display: 'flex', alignItems: 'center', justifyContent: 'space-between', borderBottom: `1px solid ${T.line}` }}>
        <div style={{ fontSize: 13, fontWeight: 600, color: T.ink }}>Recently changed</div>
        <span style={{ fontSize: 11, fontFamily: T.fontMono, color: T.ink3 }}>last {files.length}</span>
      </div>
      <div>
        {files.map((f, i) => (
          <div
            key={f.path || i}
            style={{
              display: 'grid',
              gridTemplateColumns: '1fr auto auto auto',
              gap: 14,
              alignItems: 'center',
              padding: '11px 22px',
              borderBottom: i === files.length - 1 ? 'none' : `1px solid ${T.line}`,
            }}
          >
            <span
              title={f.path}
              style={{
                fontSize: 12,
                fontFamily: T.fontMono,
                color: T.ink2,
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                whiteSpace: 'nowrap' as const,
                minWidth: 0,
                display: 'inline-flex',
                alignItems: 'center',
                gap: 7,
              }}
            >
              {f.pinned && <NavIcon name="lock" size={11} color={T.ink3} />}
              {baseName(f.path)}
            </span>
            <StatusPill status={f.status} />
            <span style={{ fontSize: 11, fontFamily: T.fontMono, color: T.ink3, textAlign: 'right' as const, minWidth: 56 }}>
              {relativeTime(f.modified_at)}
            </span>
            <span style={{ fontSize: 12, fontFamily: T.fontMono, fontWeight: 600, color: T.ink, textAlign: 'right' as const, minWidth: 56 }}>
              {formatBytes(f.size_bytes)}
            </span>
          </div>
        ))}
      </div>
    </Card>
  )
}

// ── States: loading / error / empty (pattern from InsightsView) ───────────────

function FilesLoadingState() {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
      <Card style={{ padding: 18 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 14 }}>
          <Skeleton width={36} height={36} radius={9} />
          <div style={{ flex: 1 }}>
            <Skeleton width={130} height={12} />
            <div style={{ marginTop: 8 }}><Skeleton width="60%" height={11} /></div>
          </div>
          <Skeleton width={130} height={32} radius={6} />
        </div>
      </Card>
      <Card style={{ padding: 0 }}>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)' }}>
          {[0, 1, 2, 3].map((i) => (
            <div key={i} style={{ padding: '16px 18px', borderLeft: i === 0 ? 'none' : `1px solid ${T.line}` }}>
              <Skeleton width={70} height={10} />
              <div style={{ marginTop: 10 }}><Skeleton width={48} height={20} /></div>
              <div style={{ marginTop: 6 }}><Skeleton width={60} height={10} /></div>
            </div>
          ))}
        </div>
      </Card>
      <Card style={{ padding: 22 }}>
        <div style={{ marginBottom: 14 }}><Skeleton width={100} height={12} /></div>
        <Skeleton height={10} radius={999} />
      </Card>
      <Card style={{ padding: 0 }}>
        <div style={{ padding: '16px 22px 12px', borderBottom: `1px solid ${T.line}` }}>
          <Skeleton width={120} height={12} />
        </div>
        {[0, 1, 2, 3].map((i) => (
          <div
            key={i}
            style={{
              display: 'grid',
              gridTemplateColumns: '1fr auto auto auto',
              gap: 14,
              alignItems: 'center',
              padding: '11px 22px',
              borderBottom: i === 3 ? 'none' : `1px solid ${T.line}`,
            }}
          >
            <Skeleton width={`${64 - i * 8}%`} height={12} />
            <Skeleton width={72} height={16} radius={999} />
            <Skeleton width={44} height={12} />
            <Skeleton width={50} height={12} />
          </div>
        ))}
      </Card>
    </div>
  )
}

function FilesErrorState({ err, onRetry }: { err: { reason: string; unsupported: boolean }; onRetry: () => void }) {
  if (err.unsupported) {
    return (
      <>
        <SyncFolderCard />
        <Card style={{ padding: 22 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
            <div style={{ width: 36, height: 36, borderRadius: 9, background: T.paper2, border: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
              <NavIcon name="cloud" size={16} color={T.ink3} />
            </div>
            <div style={{ minWidth: 0 }}>
              <div style={{ fontSize: 13, fontWeight: 600, color: T.ink, marginBottom: 2 }}>Sync details not available in this build</div>
              <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.5 }}>
                The per-file sync state needs the desktop sync engine, which this build doesn’t ship yet. Explorer is still your file surface.
              </div>
            </div>
          </div>
        </Card>
      </>
    )
  }
  return (
    <>
      <SyncFolderCard />
      <Card style={{ padding: 22 }}>
        <div style={{ display: 'flex', alignItems: 'flex-start', gap: 12 }}>
          <div style={{ width: 36, height: 36, borderRadius: 9, background: T.paper2, border: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
            <NavIcon name="cloud" size={16} color={FILES_DANGER} />
          </div>
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ fontSize: 13, fontWeight: 600, color: T.ink, marginBottom: 2 }}>Couldn’t load your sync state</div>
            <div style={{ fontSize: 11.5, fontFamily: T.fontMono, color: T.ink3, lineHeight: 1.5, wordBreak: 'break-word' as const }}>
              {err.reason}
            </div>
            <div style={{ marginTop: 14 }}>
              <PrimaryBtn onClick={onRetry}>Retry</PrimaryBtn>
            </div>
          </div>
        </div>
      </Card>
    </>
  )
}

function FilesEmptyState() {
  return (
    <>
      <SyncFolderCard />
      <Card style={{ padding: 0 }}>
        <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', padding: '48px 36px', gap: 10 }}>
          <NavIcon name="cloud" size={26} color={T.ink4} />
          <div style={{ fontSize: 14, fontWeight: 600, color: T.ink, marginTop: 4 }}>No files on this PC yet</div>
          <div style={{ fontSize: 12, color: T.ink3, textAlign: 'center' as const, lineHeight: 1.6, maxWidth: 360 }}>
            Once files sync to this PC, you’ll see what’s local, what’s online-only, what’s pinned, and anything that needs attention here.
          </div>
        </div>
      </Card>
    </>
  )
}

// ── Known-folder backup onboarding re-open bus (task 0804) ────────────────────
//
// The first-run "Back up these folders?" prompt is mounted ONCE at the app root
// (so its overlay covers the whole window). The Manage-backup panel — rendered
// several layers deep inside FilesView — needs to RE-OPEN that same prompt when
// the user clicks "Set up backup" after having skipped it. Rather than thread a
// callback down through FilesView → ManageBackupCard, we fire a tiny window
// CustomEvent the root listens for. Show-once still governs the AUTOMATIC
// first-run appearance; this manual path bypasses it on explicit request.
const KF_ONBOARDING_REOPEN_EVENT = 'bb:kf-onboarding-reopen'
// Fired after the onboarding prompt enables one or more folders, so the
// Manage-backup panel (if mounted) re-reads the now-updated backup state.
const KF_BACKUP_CHANGED_EVENT = 'bb:kf-backup-changed'

function requestKnownFolderOnboarding() {
  window.dispatchEvent(new CustomEvent(KF_ONBOARDING_REOPEN_EVENT))
}

// ── Section: Manage backup (known-folder backup, task 0797) ───────────────────
//
// OneDrive-style "Manage backup": one-way mirror of the user's Windows known
// folders (Desktop / Documents / Pictures / Music / Videos / Downloads) into the
// vault. Model 2 — the real folders are NOT redirected; we copy their contents
// into the sync root and the engine uploads them. Honest copy, no emojis. The
// toggle is NEUTRAL (ink), not amber — amber is reserved for encryption state.

/** A small on/off switch matching the Windows app's neutral palette. */
function BackupToggle({
  on,
  busy,
  onChange,
  label,
}: {
  on: boolean
  busy: boolean
  onChange: () => void
  label: string
}) {
  return (
    <button
      role="switch"
      aria-checked={on}
      aria-label={label}
      disabled={busy}
      onClick={onChange}
      style={{
        position: 'relative' as const,
        width: 36,
        height: 20,
        flexShrink: 0,
        borderRadius: 999,
        border: `1px solid ${on ? T.ink : T.line2}`,
        background: on ? T.ink : T.paper2,
        cursor: busy ? 'wait' : 'pointer',
        opacity: busy ? 0.6 : 1,
        padding: 0,
        transition: 'background 120ms ease, border-color 120ms ease',
      }}
    >
      <span
        style={{
          position: 'absolute' as const,
          top: 2,
          left: on ? 18 : 2,
          width: 14,
          height: 14,
          borderRadius: 999,
          background: T.paper,
          boxShadow: '0 1px 2px rgba(0,0,0,0.2)',
          transition: 'left 120ms ease',
        }}
      />
    </button>
  )
}

/** Status line under each folder name: Backed up / Off / Syncing / unavailable. */
function backupStatus(f: KnownFolderStatus, pending: boolean): { text: string; tone: 'on' | 'off' | 'busy' | 'na' } {
  if (f.source_path == null) return { text: 'Not available on this platform', tone: 'na' }
  if (pending) return { text: f.enabled ? 'Starting backup…' : 'Updating…', tone: 'busy' }
  if (f.enabled) return { text: 'Backed up to your vault', tone: 'on' }
  return { text: 'Not backed up', tone: 'off' }
}

function ManageBackupCard() {
  const [folders, setFolders] = useState<KnownFolderStatus[] | null>(null)
  const [unsupported, setUnsupported] = useState(false)
  const [error, setError] = useState<string | null>(null)
  // Keys with an in-flight set_known_folder_backup call.
  const [pending, setPending] = useState<Set<string>>(new Set())

  const load = async () => {
    const res = await getKnownFolderBackup()
    if (!res.ok) {
      if (res.unsupported) setUnsupported(true)
      else setError(res.reason)
      return
    }
    setFolders(res.value)
  }

  useEffect(() => {
    let cancelled = false
    void (async () => {
      const res = await getKnownFolderBackup()
      if (cancelled) return
      if (!res.ok) {
        if (res.unsupported) setUnsupported(true)
        else setError(res.reason)
        return
      }
      setFolders(res.value)
    })()
    return () => { cancelled = true }
  }, [])

  // Re-read backup state when the onboarding prompt enables folders (task 0804)
  // so the panel reflects the change without a manual refresh.
  useEffect(() => {
    const onChanged = () => { void load() }
    window.addEventListener(KF_BACKUP_CHANGED_EVENT, onChanged)
    return () => window.removeEventListener(KF_BACKUP_CHANGED_EVENT, onChanged)
  }, [])

  const toggle = async (f: KnownFolderStatus) => {
    if (f.source_path == null) return
    const next = !f.enabled
    setError(null)
    setPending((p) => new Set(p).add(f.key))
    // Optimistic flip.
    setFolders((prev) => prev?.map((x) => (x.key === f.key ? { ...x, enabled: next } : x)) ?? prev)
    const res = await setKnownFolderBackup(f.key, next)
    setPending((p) => {
      const n = new Set(p)
      n.delete(f.key)
      return n
    })
    if (!res.ok) {
      // Roll back the optimistic flip and surface the error.
      setFolders((prev) => prev?.map((x) => (x.key === f.key ? { ...x, enabled: f.enabled } : x)) ?? prev)
      setError(res.unsupported ? commandUnavailableLabel('set_known_folder_backup') : res.reason)
      return
    }
    // Refresh to pick up updated item counts after a mirror pass.
    void load()
  }

  // Windows-only feature — render nothing on macOS/Linux rather than an empty card.
  if (unsupported) return null
  if (folders != null && folders.every((f) => f.source_path == null)) return null

  // No folders backed up yet → offer the guided "Set up backup" prompt (task
  // 0804). This is how a user who dismissed the first-run prompt with "Not now"
  // can return to it: the affordance re-opens the SAME prompt (pre-checked
  // Desktop/Documents/Pictures). Only shown when at least one folder resolves
  // here AND none are currently backed up.
  const noneBackedUp =
    folders != null && folders.some((f) => f.source_path != null) && folders.every((f) => !f.enabled)

  return (
    <Card style={{ padding: 0, marginBottom: 14 }}>
      <div style={{ padding: '16px 18px 12px', borderBottom: `1px solid ${T.line}` }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12 }}>
          <div style={{ fontSize: 13, fontWeight: 600, color: T.ink }}>Manage backup</div>
          {noneBackedUp ? (
            <button
              type="button"
              onClick={() => requestKnownFolderOnboarding()}
              style={{
                padding: '5px 11px',
                fontSize: 11.5,
                fontFamily: T.fontSans,
                fontWeight: 600,
                borderRadius: 6,
                border: `1px solid ${T.amberDeep}`,
                background: T.amber,
                color: T.ink,
                cursor: 'pointer',
                whiteSpace: 'nowrap' as const,
                letterSpacing: '-0.005em',
              }}
            >
              Set up backup
            </button>
          ) : (
            <Chip tone="neutral">One-way</Chip>
          )}
        </div>
        <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.55, marginTop: 4, maxWidth: 520 }}>
          Back up your Windows folders to your encrypted vault. Files are copied in — your
          original folders stay exactly where they are, so this works alongside OneDrive. Turning a
          folder off stops new backups; what’s already in your vault stays.
        </div>
      </div>

      {folders == null ? (
        <div style={{ padding: 18, display: 'grid', gap: 12 }}>
          <Skeleton height={40} />
          <Skeleton height={40} />
          <Skeleton height={40} />
        </div>
      ) : (
        <div>
          {folders.map((f, i) => {
            const isPending = pending.has(f.key)
            const status = backupStatus(f, isPending)
            const statusColor =
              status.tone === 'on' ? T.green : status.tone === 'busy' ? T.ink2 : T.ink3
            const disabled = f.source_path == null
            return (
              <div
                key={f.key}
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 14,
                  padding: '12px 18px',
                  borderTop: i === 0 ? 'none' : `1px solid ${T.line}`,
                  opacity: disabled ? 0.55 : 1,
                }}
              >
                <div style={{ width: 30, height: 30, borderRadius: 8, background: T.paper2, border: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
                  <NavIcon name="folder" size={14} color={T.ink3} />
                </div>
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ display: 'flex', alignItems: 'baseline', gap: 8 }}>
                    <span style={{ fontSize: 12.5, fontWeight: 600, color: T.ink }}>{f.display_name}</span>
                    {f.item_count != null && f.item_count > 0 && (
                      <span style={{ fontSize: 10.5, fontFamily: T.fontMono, color: T.ink4 }}>
                        {f.item_count.toLocaleString('en-US')}
                        {f.item_count >= 10000 ? '+' : ''} {f.item_count === 1 ? 'file' : 'files'}
                      </span>
                    )}
                  </div>
                  {f.source_path && (
                    <div
                      title={f.source_path}
                      style={{ fontSize: 11, fontFamily: T.fontMono, color: T.ink4, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' as const, marginTop: 1 }}
                    >
                      {f.source_path}
                    </div>
                  )}
                  <div style={{ fontSize: 11, color: statusColor, marginTop: 2 }}>{status.text}</div>
                </div>
                <BackupToggle
                  on={f.enabled}
                  busy={isPending}
                  onChange={() => void toggle(f)}
                  label={`${f.enabled ? 'Stop backing up' : 'Back up'} ${f.display_name}`}
                />
              </div>
            )
          })}
        </div>
      )}

      {error && <div style={{ padding: '0 18px 14px', fontSize: 11.5, color: FILES_DANGER }}>{error}</div>}
    </Card>
  )
}

// ── Root: FilesView ───────────────────────────────────────────────────────────

function FilesView() {
  const [loading, setLoading] = useState(true)
  const [err, setErr] = useState<{ reason: string; unsupported: boolean } | null>(null)
  const [overview, setOverview] = useState<FileOverview | null>(null)
  const [reloadKey, setReloadKey] = useState(0)

  useEffect(() => {
    let cancelled = false
    setLoading(true)
    setErr(null)

    void (async () => {
      const res = await desktopFileOverview(FILES_RECENT_LIMIT)
      if (cancelled) return
      if (!res.ok) {
        setErr({ reason: res.reason, unsupported: res.unsupported })
        setLoading(false)
        return
      }
      setOverview(res.value)
      setLoading(false)
    })()

    return () => { cancelled = true }
  }, [reloadKey])

  const retry = () => setReloadKey((k) => k + 1)
  const isEmpty = overview != null && overview.total_files === 0 && overview.total_bytes === 0

  return (
    <div style={{ overflow: 'auto', padding: '28px 36px', flex: 1 }}>
      <PageHeader
        title="Files"
        subtitle="What’s synced to this PC — local vs online-only, anything pinned or in conflict, and your most recent changes. Explorer stays your file surface; this is the device view."
      />

      {loading ? (
        <FilesLoadingState />
      ) : err ? (
        <FilesErrorState err={err} onRetry={retry} />
      ) : isEmpty ? (
        <>
          <SyncFolderCard />
          {/* Manage backup is useful even before any files exist — it's how the
              user opts their Windows folders INTO the vault in the first place. */}
          <ManageBackupCard />
          <FilesEmptyState />
        </>
      ) : overview ? (
        <>
          <SyncFolderCard />
          <ManageBackupCard />
          <FilesMetricStrip overview={overview} />
          <ByStatusBar overview={overview} />
          {overview.recent.length > 0 && <RecentlyChanged files={overview.recent} />}
        </>
      ) : (
        // Defensive fallback — never white-screen.
        <FilesEmptyState />
      )}
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
  const regionLabel = useRegionLabel()
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
          {regionLabel}
        </span>
      </div>
    </div>
  )
}

// ── Root component ──────────────────────────────────────────────────────────

export default function WindowsApp() {
  const [activeNav, setActiveNav] = useState<NavId>(() => initialNav())
  const [status, setStatus] = useState<SyncStatus | null>(null)
  const [usage, setUsage] = useState<BillingUsage | null>(null)
  const [storage, setStorage] = useState<StorageSummary | null>(null)
  // Avoid refetching billing usage every poll once we have it.
  const usageLoaded = useRef(false)

  // Known-folder backup onboarding (task 0804). `mode` drives the one mounted
  // instance: 'auto' = self-gated first-run (shows once when never seen + no
  // folders backed up), 'forced' = a manual "Set up backup" re-open, 'closed' =
  // not mounted. We start in 'auto'; the component self-closes (via onClose)
  // when it's not a genuine first-run, so the overlay never flashes off-Windows
  // or for returning users. A bumping `key` remounts it on a forced re-open so
  // its internal load effect re-runs.
  const [kfOnboarding, setKfOnboarding] = useState<'auto' | 'forced' | 'closed'>('auto')
  const kfOnboardingKey = useRef(0)

  // Manual re-open from the Manage-backup panel ("Set up backup").
  useEffect(() => {
    const onReopen = () => {
      kfOnboardingKey.current += 1
      setKfOnboarding('forced')
    }
    window.addEventListener(KF_ONBOARDING_REOPEN_EVENT, onReopen)
    return () => window.removeEventListener(KF_ONBOARDING_REOPEN_EVENT, onReopen)
  }, [])

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

  useEffect(() => {
    let cancelled = false
    const unlisten = listen<string>(WINDOWS_APP_NAV_EVENT, (event) => {
      if (!cancelled && isNavId(event.payload)) {
        setActiveNav(event.payload)
      }
    })
    return () => {
      cancelled = true
      void unlisten.then((callback) => callback())
    }
  }, [])

  const loggedIn = status?.logged_in ?? false

  // Load billing usage once signed in (storage widget + Home). On transient
  // failure we retry with EXPONENTIAL BACKOFF rather than a tight 4s loop — the
  // engine may not have a valid session ready on the very first
  // `logged_in: true` tick, so a single fire-and-forget call can strand the
  // widget at the skeleton state, but a fixed 4s loop hammers the shared billing
  // endpoint (which 429s under load) for as long as the session stays unready.
  //
  // Backoff: 4s → 8s → 16s → … capped at 60s. We STOP entirely once usage loads
  // (usageLoaded short-circuits the effect) and after a bounded number of
  // attempts, so a persistently-failing endpoint isn't polled forever. The
  // local-mirror fallback (desktop_storage_summary) still fills in a partial
  // signal on the way.
  useEffect(() => {
    if (!loggedIn) return
    // If we already have data, don't re-fetch on every status poll re-render.
    if (usageLoaded.current) return

    let cancelled = false
    let retryTimer: ReturnType<typeof window.setTimeout> | undefined
    let tries = 0

    const BASE_DELAY_MS = 4000
    const MAX_DELAY_MS = 60_000
    const MAX_TRIES = 8 // ~4+8+16+32+60+60+60s of backoff, then give up quietly

    const attempt = async () => {
      const r = await accountUsage()
      if (cancelled) return
      if (r.ok) {
        setUsage(r.value)
        usageLoaded.current = true
        return // loaded — stop retrying
      }
      // accountUsage() failed — try the local mirror for a partial signal.
      const s = await command<StorageSummary>('desktop_storage_summary')
      if (cancelled) return
      if (s.ok) {
        setStorage(s.value)
        // storage is a partial fallback; keep retrying accountUsage() (with
        // backoff) so the progress bar eventually shows accurate billing quota.
      }
      // Both failed (engine session not ready / endpoint rate-limited) —
      // schedule a backed-off retry so the widget recovers without hammering.
      tries += 1
      if (tries >= MAX_TRIES) return // give up quietly rather than loop forever
      const delay = Math.min(BASE_DELAY_MS * 2 ** (tries - 1), MAX_DELAY_MS)
      retryTimer = window.setTimeout(attempt, delay)
    }

    void attempt()

    return () => {
      cancelled = true
      if (retryTimer !== undefined) window.clearTimeout(retryTimer)
    }
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
        return <SettingsView status={status} onOpenSignIn={openSignIn} />
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
      {/* Keyframes for the skeleton shimmer + the refresh-icon spin (global,
          inline — so any view can use them without mounting UpdateBanner). */}
      <style>{'@keyframes bb-shimmer { 0% { background-position: 200% 0; } 100% { background-position: -200% 0; } } @keyframes bb-spin { to { transform: rotate(360deg); } }'}</style>
      <UpdateBanner />

      {/* Known-folder backup onboarding (task 0804). Only meaningful once signed
          in (it enables backup into the signed-in vault). 'auto' self-gates on
          the seen flag + empty backup set inside the component; 'forced' is an
          explicit re-open from the Manage-backup panel. */}
      {loggedIn && kfOnboarding !== 'closed' && (
        <KnownFolderOnboarding
          key={kfOnboardingKey.current}
          forced={kfOnboarding === 'forced'}
          onClose={(enabledAny) => {
            setKfOnboarding('closed')
            if (enabledAny) window.dispatchEvent(new CustomEvent(KF_BACKUP_CHANGED_EVENT))
          }}
        />
      )}
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
                Falkenstein
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
