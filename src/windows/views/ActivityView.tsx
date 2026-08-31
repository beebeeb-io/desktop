/**
 * ActivityView — the account's Activity surface for the Windows app (PKG-DATA).
 *
 * Two sections behind a lightweight in-view tab switch, each fully self-
 * fetching on mount and each handling all four states (loading / error / empty
 * / loaded) so a down login backend reads as crafted, never broken:
 *
 *   1. Timeline      — accountActivityFeed(1, 50) → events grouped by day.
 *   2. Notifications — accountNotifications() → list + unread_count (display
 *                      only; marking-read is not a wired IPC, so no fake button).
 *
 * Notification preferences now live under the main app Settings area so account
 * and desktop settings have one coherent home.
 *
 * Design grounded on design/hifi/hifi-settings.jsx (HiActivity for the day-
 * grouped timeline with per-row type dot, and HiSettingsNotifications for the
 * preference rows). Adapted to the desktop idiom (T tokens, ~10-13px type,
 * 6-10px radii, 1px T.line borders, var(--shadow-1) via Card).
 *
 * Brand: amber only for the encryption state line + unread dot + active tab +
 * the live switch knob; Inter for human text, JetBrains Mono for every machine
 * value (ids, timestamps, ip/device, counts, percentages); name the city only
 * ("Falkenstein"), never the storage provider; honest voice; no emojis.
 */

import { useEffect, useState } from 'react'
import {
  accountActivityFeed,
  accountNotifications,
  type ActivityFeed,
  type ActivityFeedEvent,
  type NotificationList,
} from '../../desktopApi'
import { T, Card, PageHeader, Chip, Skeleton, NavIcon, PrimaryBtn } from '../ui'
import { useRegionLabel } from '../useRegion'

// ── Date helpers ─────────────────────────────────────────────────────────────
// All wire timestamps are RFC3339 strings; guard null/undefined + unparseable.

function parseDate(value?: string | null): Date | null {
  if (!value) return null
  const d = new Date(value)
  return Number.isNaN(d.getTime()) ? null : d
}

// Compact relative time for recent events, short absolute for older ones. Mono.
function relativeTime(value?: string | null): string {
  const d = parseDate(value)
  if (!d) return '—'
  const diffMs = Date.now() - d.getTime()
  const sec = Math.round(diffMs / 1000)
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

// Day-header key + label for grouping (Today / Yesterday / short absolute date).
function dayLabel(value?: string | null): { key: string; label: string } {
  const d = parseDate(value)
  if (!d) return { key: 'unknown', label: 'Unknown date' }
  const startOfDay = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime()
  const today = startOfDay(new Date())
  const that = startOfDay(d)
  const dayMs = 86_400_000
  const key = new Date(that).toISOString().slice(0, 10)
  if (that === today) return { key, label: 'Today' }
  if (that === today - dayMs) return { key, label: 'Yesterday' }
  return { key, label: d.toLocaleDateString(undefined, { weekday: 'long', day: '2-digit', month: 'short' }) }
}

// ── Event presentation ───────────────────────────────────────────────────────
// Map the audit event `type` to an icon + a dot accent. Amber is reserved for
// security-sensitive events (sign-ins, key/recovery); everything else is neutral
// or green so amber stays meaningful, never decorative.

function eventVisual(type: string): { icon: string; dot: string } {
  const t = (type || '').toLowerCase()
  if (t.includes('login') || t.includes('signin') || t.includes('sign_in') || t.includes('device'))
    return { icon: 'device', dot: T.amber }
  if (t.includes('session') || t.includes('revoke') || t.includes('lock'))
    return { icon: 'lock', dot: T.ink }
  if (t.includes('key') || t.includes('recovery') || t.includes('password') || t.includes('security') || t.includes('totp'))
    return { icon: 'shield', dot: T.amberDeep }
  if (t.includes('share'))
    return { icon: 'cloud', dot: T.green }
  if (t.includes('upload') || t.includes('backup') || t.includes('sync') || t.includes('file'))
    return { icon: 'folder', dot: T.green }
  return { icon: 'clock', dot: T.ink3 }
}

// Turn an audit event name into something a human reads, when no subject given.
// e.g. "auth.login.success" → "Auth login success". Falls back gracefully.
function humanizeType(type: string): string {
  const raw = (type || '').replace(/[._]+/g, ' ').trim()
  if (!raw) return 'Activity'
  return raw.charAt(0).toUpperCase() + raw.slice(1)
}

// Returns true iff every comma-separated segment looks like a UUID v4 hex string.
// Used to suppress raw UUIDs that the server sets as event subjects (zero-knowledge
// events where the server never sees filenames).
const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i
function looksLikeId(s: string): boolean {
  const parts = s.split(',').map(p => p.trim()).filter(Boolean)
  return parts.length > 0 && parts.every(p => UUID_RE.test(p))
}

// Clean human-readable labels for event types where the subject is always an ID
// or where the subject is empty. file.trash.bulk is handled inline (needs count).
const TYPE_LABELS: Record<string, string> = {
  'file.trash':      'Moved an item to trash',
  'file.downloaded': 'Downloaded an item',
  'auth.logout':     'Signed out',
  'user.logout':     'Signed out',
}

// Derives the title to display for a timeline event, never rendering a raw UUID.
// Priority: (1) known type labels; (2) file.trash.bulk with count; (3) any
// non-UUID subject; (4) humanized type name as catch-all.
function displayTitle(ev: ActivityFeedEvent): string {
  if (ev.type === 'file.trash.bulk') {
    const n = (ev.subject ?? '').split(',').map(s => s.trim()).filter(Boolean).length
    return n === 1 ? 'Moved an item to trash' : `Moved ${n} items to trash`
  }
  if (ev.type in TYPE_LABELS) return TYPE_LABELS[ev.type]
  const subj = ev.subject?.trim()
  if (subj && !looksLikeId(subj)) return subj
  return humanizeType(ev.type)
}

// ── Shared state shells ──────────────────────────────────────────────────────

const RED = 'oklch(0.5 0.18 25)'

function ErrorBlock({ reason, unsupported, onRetry }: { reason: string; unsupported: boolean; onRetry: () => void }) {
  if (unsupported) {
    return (
      <Card style={{ padding: 22, display: 'flex', alignItems: 'center', gap: 12 }}>
        <NavIcon name="clock" size={16} color={T.ink4} />
        <div>
          <div style={{ fontSize: 12.5, fontWeight: 600, color: T.ink2 }}>Not available in this build</div>
          <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.5, marginTop: 2 }}>
            This view binds to a command that isn’t wired into the current desktop build.
          </div>
        </div>
      </Card>
    )
  }
  return (
    <Card style={{ padding: 22 }}>
      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 12 }}>
        <NavIcon name="external" size={16} color={RED} />
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ fontSize: 12.5, fontWeight: 600, color: T.ink }}>Couldn’t load</div>
          <div style={{ fontSize: 11.5, fontFamily: T.fontSans, color: T.ink3, lineHeight: 1.5, marginTop: 4, wordBreak: 'break-word' as const }}>
            {reason}
          </div>
        </div>
      </div>
      <div style={{ marginTop: 14 }}>
        <PrimaryBtn onClick={onRetry}>Retry</PrimaryBtn>
      </div>
    </Card>
  )
}

function EmptyBlock({ icon, title, body }: { icon: string; title: string; body: string }) {
  return (
    <Card style={{ padding: '30px 24px', display: 'flex', flexDirection: 'column', alignItems: 'center', textAlign: 'center' as const, gap: 8 }}>
      <div style={{ width: 38, height: 38, borderRadius: 10, background: T.paper2, border: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
        <NavIcon name={icon} size={16} color={T.ink4} />
      </div>
      <div style={{ fontSize: 13, fontWeight: 600, color: T.ink }}>{title}</div>
      <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.6, maxWidth: 360 }}>{body}</div>
    </Card>
  )
}

// ── Timeline section ─────────────────────────────────────────────────────────

function TimelineSection() {
  const [data, setData] = useState<ActivityFeed | null>(null)
  const [err, setErr] = useState<{ reason: string; unsupported: boolean } | null>(null)
  const [loading, setLoading] = useState(true)

  const load = () => {
    let cancelled = false
    setLoading(true)
    setErr(null)
    void (async () => {
      const r = await accountActivityFeed(1, 50)
      if (cancelled) return
      if (r.ok) {
        setData(r.value)
        setErr(null)
      } else {
        // SPLIT PENDING — NO OWNER YET (flagged to the lead 2026-08-31): toast this action failure, leave the load failure inline.
        // eslint-disable-next-line beebeeb/no-ad-hoc-error-surface
        setErr({ reason: r.reason, unsupported: r.unsupported })
        setData(null)
      }
      setLoading(false)
    })()
    return () => { cancelled = true }
  }

  useEffect(() => {
    let active = true
    setLoading(true)
    setErr(null)
    void (async () => {
      const r = await accountActivityFeed(1, 50)
      if (!active) return
      if (r.ok) setData(r.value)
      else setErr({ reason: r.reason, unsupported: r.unsupported })
      setLoading(false)
    })()
    return () => { active = false }
  }, [])

  if (loading) return <TimelineSkeleton />
  if (err) return <ErrorBlock reason={err.reason} unsupported={err.unsupported} onRetry={load} />

  const events = data?.events ?? []
  if (events.length === 0) {
    return (
      <EmptyBlock
        icon="clock"
        title="No activity yet"
        body="Sign-ins, shares, and sync events will appear here as they happen. Each entry is encrypted per-entry — only you can read it."
      />
    )
  }

  // Group by calendar day, preserving wire order (newest first as delivered).
  const groups: Array<{ key: string; label: string; items: ActivityFeedEvent[] }> = []
  const indexByKey = new Map<string, number>()
  for (const ev of events) {
    const { key, label } = dayLabel(ev.created_at)
    let idx = indexByKey.get(key)
    if (idx === undefined) {
      idx = groups.length
      indexByKey.set(key, idx)
      groups.push({ key, label, items: [] })
    }
    groups[idx].items.push(ev)
  }

  const total = data?.total ?? events.length
  const hasMore = total > events.length

  return (
    <Card style={{ padding: 0, overflow: 'hidden' }}>
      {/* Honest header: events are per-entry encrypted. */}
      <div style={{ padding: '13px 20px', borderBottom: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', gap: 10 }}>
        <NavIcon name="clock" size={14} color={T.ink2} />
        <div style={{ minWidth: 0 }}>
          <div style={{ fontSize: 12.5, fontWeight: 600, color: T.ink }}>Timeline</div>
          <div style={{ fontSize: 11, color: T.ink3 }}>What happened on your account — only you can read it.</div>
        </div>
      </div>

      {groups.map((g) => (
        <div key={g.key}>
          <div style={{ padding: '9px 20px 5px', background: T.paper2, borderBottom: `1px solid ${T.line}` }}>
            <span style={{ fontSize: 9.5, fontFamily: T.fontMono, textTransform: 'uppercase' as const, letterSpacing: '0.07em', color: T.ink3 }}>
              {g.label}
            </span>
          </div>
          {g.items.map((ev, i) => {
            const vis = eventVisual(ev.type)
            const last = i === g.items.length - 1
            return (
              <div
                key={ev.id || `${g.key}-${i}`}
                style={{ padding: '12px 20px', display: 'flex', alignItems: 'flex-start', gap: 14, borderBottom: last ? 'none' : `1px solid ${T.line}` }}
              >
                <div style={{ width: 28, height: 28, borderRadius: '50%', background: T.paper2, border: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0, position: 'relative' }}>
                  <NavIcon name={vis.icon} size={13} color={T.ink2} />
                  <span style={{ position: 'absolute', top: -1, right: -1, width: 7, height: 7, borderRadius: '50%', background: vis.dot, border: `1.5px solid ${T.paper}` }} />
                </div>
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ fontSize: 13, color: T.ink, fontWeight: 500, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' as const }}>
                    {displayTitle(ev)}
                  </div>
                  {(ev.details || ev.where) && (
                    <div style={{ fontSize: 11, color: T.ink3, marginTop: 2, lineHeight: 1.5, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' as const }}>
                      {ev.details && <span style={{ fontFamily: T.fontMono }}>{ev.details}</span>}
                      {ev.details && ev.where && <span style={{ color: T.ink4 }}> · </span>}
                      {ev.where && <span style={{ fontFamily: T.fontMono, color: T.ink3 }}>{ev.where}</span>}
                    </div>
                  )}
                </div>
                <span style={{ fontSize: 11, fontFamily: T.fontMono, color: T.ink4, flexShrink: 0, whiteSpace: 'nowrap' as const }} title={parseDate(ev.created_at)?.toISOString() ?? undefined}>
                  {relativeTime(ev.created_at)}
                </span>
              </div>
            )
          })}
        </div>
      ))}

      {hasMore && (
        <div style={{ padding: '11px 20px', background: T.paper2, borderTop: `1px solid ${T.line}`, textAlign: 'center' as const }}>
          <span style={{ fontSize: 10.5, fontFamily: T.fontMono, color: T.ink3 }}>
            Showing {events.length} of {total}
          </span>
        </div>
      )}
    </Card>
  )
}

function TimelineSkeleton() {
  return (
    <Card style={{ padding: 0, overflow: 'hidden' }}>
      <div style={{ padding: '13px 20px', borderBottom: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', gap: 10 }}>
        <NavIcon name="clock" size={14} color={T.ink4} />
        <Skeleton width={120} height={12} />
      </div>
      <div style={{ padding: '9px 20px 5px', background: T.paper2, borderBottom: `1px solid ${T.line}` }}>
        <Skeleton width={56} height={9} />
      </div>
      {Array.from({ length: 4 }).map((_, i) => (
        <div key={i} style={{ padding: '12px 20px', display: 'flex', alignItems: 'flex-start', gap: 14, borderBottom: i < 3 ? `1px solid ${T.line}` : 'none' }}>
          <Skeleton width={28} height={28} radius={999} />
          <div style={{ flex: 1, display: 'flex', flexDirection: 'column', gap: 6 }}>
            <Skeleton width={`${62 - i * 8}%`} height={12} />
            <Skeleton width={`${44 - i * 6}%`} height={10} />
          </div>
          <Skeleton width={56} height={10} />
        </div>
      ))}
    </Card>
  )
}

// ── Notifications section (display only — marking-read is not a wired IPC) ────

function NotificationsSection() {
  const [data, setData] = useState<NotificationList | null>(null)
  const [err, setErr] = useState<{ reason: string; unsupported: boolean } | null>(null)
  const [loading, setLoading] = useState(true)

  const load = () => {
    let cancelled = false
    setLoading(true)
    setErr(null)
    void (async () => {
      const r = await accountNotifications()
      if (cancelled) return
      if (r.ok) { setData(r.value); setErr(null) }
      // SPLIT PENDING — NO OWNER YET (flagged to the lead 2026-08-31): toast this action failure, leave the load failure inline.
      // eslint-disable-next-line beebeeb/no-ad-hoc-error-surface
      else { setErr({ reason: r.reason, unsupported: r.unsupported }); setData(null) }
      setLoading(false)
    })()
    return () => { cancelled = true }
  }

  useEffect(() => {
    let active = true
    setLoading(true)
    setErr(null)
    void (async () => {
      const r = await accountNotifications()
      if (!active) return
      if (r.ok) setData(r.value)
      else setErr({ reason: r.reason, unsupported: r.unsupported })
      setLoading(false)
    })()
    return () => { active = false }
  }, [])

  if (loading) return <NotificationsSkeleton />
  if (err) return <ErrorBlock reason={err.reason} unsupported={err.unsupported} onRetry={load} />

  const notifications = data?.notifications ?? []
  const unread = data?.unread_count ?? 0

  if (notifications.length === 0) {
    return (
      <EmptyBlock
        icon="cloud"
        title="No notifications"
        body="When something needs your attention — a new device sign-in, a share, a storage warning — it shows up here."
      />
    )
  }

  return (
    <Card style={{ padding: 0, overflow: 'hidden' }}>
      <div style={{ padding: '13px 20px', borderBottom: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', gap: 10 }}>
        <NavIcon name="cloud" size={14} color={T.ink2} />
        <div style={{ minWidth: 0, flex: 1 }}>
          <div style={{ fontSize: 12.5, fontWeight: 600, color: T.ink }}>Notifications</div>
          <div style={{ fontSize: 11, color: T.ink3 }}>Recent alerts from your account.</div>
        </div>
        {unread > 0 && (
          <Chip tone="amber">
            <span style={{ width: 6, height: 6, borderRadius: '50%', background: T.amber, display: 'inline-block' }} />
            {unread} unread
          </Chip>
        )}
      </div>

      {notifications.map((n, i) => {
        const last = i === notifications.length - 1
        return (
          <div
            key={n.id || `n-${i}`}
            style={{ padding: '12px 20px', display: 'flex', alignItems: 'flex-start', gap: 12, borderBottom: last ? 'none' : `1px solid ${T.line}`, background: n.read ? 'transparent' : T.amberBg }}
          >
            <span
              aria-hidden
              style={{ width: 7, height: 7, borderRadius: '50%', marginTop: 5, flexShrink: 0, background: n.read ? 'transparent' : T.amber, border: n.read ? `1px solid ${T.line2}` : 'none' }}
            />
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ fontSize: 13, fontWeight: n.read ? 400 : 600, color: T.ink, lineHeight: 1.4 }}>
                {n.title && !looksLikeId(n.title) ? n.title : humanizeType(n.type)}
              </div>
              {n.body && (
                <div style={{ fontSize: 11.5, color: T.ink3, marginTop: 3, lineHeight: 1.55 }}>{n.body}</div>
              )}
            </div>
            <span style={{ fontSize: 11, fontFamily: T.fontMono, color: T.ink4, flexShrink: 0, whiteSpace: 'nowrap' as const }} title={parseDate(n.created_at)?.toISOString() ?? undefined}>
              {relativeTime(n.created_at)}
            </span>
          </div>
        )
      })}
    </Card>
  )
}

function NotificationsSkeleton() {
  return (
    <Card style={{ padding: 0, overflow: 'hidden' }}>
      <div style={{ padding: '13px 20px', borderBottom: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', gap: 10 }}>
        <NavIcon name="cloud" size={14} color={T.ink4} />
        <Skeleton width={140} height={12} />
      </div>
      {Array.from({ length: 3 }).map((_, i) => (
        <div key={i} style={{ padding: '12px 20px', display: 'flex', alignItems: 'flex-start', gap: 12, borderBottom: i < 2 ? `1px solid ${T.line}` : 'none' }}>
          <Skeleton width={7} height={7} radius={999} />
          <div style={{ flex: 1, display: 'flex', flexDirection: 'column', gap: 6 }}>
            <Skeleton width={`${56 - i * 8}%`} height={12} />
            <Skeleton width={`${72 - i * 6}%`} height={10} />
          </div>
          <Skeleton width={50} height={10} />
        </div>
      ))}
    </Card>
  )
}

// ── Tab switch ───────────────────────────────────────────────────────────────

type Tab = 'timeline' | 'notifications'

const TABS: Array<{ id: Tab; label: string }> = [
  { id: 'timeline', label: 'Timeline' },
  { id: 'notifications', label: 'Notifications' },
]

function TabBar({ active, onChange }: { active: Tab; onChange: (t: Tab) => void }) {
  return (
    <div role="tablist" aria-label="Activity sections" style={{ display: 'inline-flex', gap: 4, padding: 3, borderRadius: 8, background: T.paper2, border: `1px solid ${T.line}`, marginBottom: 18 }}>
      {TABS.map((t) => {
        const isActive = t.id === active
        return (
          <button
            key={t.id}
            role="tab"
            aria-selected={isActive}
            onClick={() => onChange(t.id)}
            style={{
              padding: '6px 14px',
              fontSize: 12,
              fontWeight: isActive ? 600 : 400,
              fontFamily: T.fontSans,
              color: isActive ? T.ink : T.ink2,
              background: isActive ? T.paper : 'transparent',
              border: isActive ? `1px solid ${T.line}` : '1px solid transparent',
              borderRadius: 6,
              cursor: 'pointer',
              transition: 'background 100ms, color 100ms',
            }}
          >
            {t.label}
          </button>
        )
      })}
    </div>
  )
}

// ── Root ─────────────────────────────────────────────────────────────────────

export default function ActivityView() {
  const [tab, setTab] = useState<Tab>('timeline')
  const regionLabel = useRegionLabel()

  return (
    <div style={{ overflow: 'auto', padding: '28px 36px', flex: 1 }}>
      <PageHeader
        title="Activity"
        subtitle="Your account’s timeline and notifications. Notification preferences now live under Settings."
        aside={
          <Chip tone="amber">
            <NavIcon name="shield" size={11} color={T.amberDeep} />
            {regionLabel}
          </Chip>
        }
      />

      <TabBar active={tab} onChange={setTab} />

      {/* Each section self-fetches on mount; remounting on tab change keeps the
          four-state handling self-contained and the data fresh on return. */}
      {tab === 'timeline' && <TimelineSection key="timeline" />}
      {tab === 'notifications' && <NotificationsSection key="notifications" />}
    </div>
  )
}
