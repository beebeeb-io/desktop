/**
 * ActivityView — the account's Activity surface for the Windows app (PKG-DATA).
 *
 * Three sections behind a lightweight in-view tab switch, each fully self-
 * fetching on mount and each handling all four states (loading / error / empty
 * / loaded) so a down login backend reads as crafted, never broken:
 *
 *   1. Timeline      — accountActivityFeed(1, 50) → events grouped by day.
 *   2. Notifications — accountNotifications() → list + unread_count (display
 *                      only; marking-read is not a wired IPC, so no fake button).
 *   3. Preferences   — accountNotificationPreferences() → the 5 booleans, each
 *                      an accessible switch; on change we PUT a single key via
 *                      accountUpdateNotificationPreferences and reflect the
 *                      RETURNED state (optimistic, reverting on failure).
 *
 * Design grounded on design/hifi/hifi-settings.jsx (HiActivity for the day-
 * grouped timeline with per-row type dot, and HiSettingsNotifications for the
 * preference rows). Adapted to the desktop idiom (T tokens, ~10-13px type,
 * 6-10px radii, 1px T.line borders, var(--shadow-1) via Card).
 *
 * Brand: amber only for the encryption state line + unread dot + active tab +
 * the live switch knob; Inter for human text, JetBrains Mono for every machine
 * value (ids, timestamps, ip/device, counts, percentages); "Falkenstein ·
 * Hetzner"; honest voice; no emojis.
 */

import { useEffect, useState } from 'react'
import {
  accountActivityFeed,
  accountNotifications,
  accountNotificationPreferences,
  accountUpdateNotificationPreferences,
  type ActivityFeed,
  type ActivityFeedEvent,
  type NotificationList,
  type NotificationPreferenceValues,
} from '../../desktopApi'
import { T, Card, PageHeader, Chip, Skeleton, NavIcon, PrimaryBtn } from '../ui'

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
                    {ev.subject?.trim() || humanizeType(ev.type)}
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
                {n.title?.trim() || humanizeType(n.type)}
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

// ── Preferences section ──────────────────────────────────────────────────────

type PrefKey = keyof NotificationPreferenceValues

const PREF_META: Array<{ key: PrefKey; label: string; hint: string }> = [
  { key: 'new_device_login', label: 'New device sign-in', hint: 'We can’t see your files, but we know when a new device signs in.' },
  { key: 'share_received', label: 'Share received', hint: 'When someone grants you access to a file or folder.' },
  { key: 'file_updated', label: 'File updated', hint: 'When a file in your vault changes on another device.' },
  { key: 'storage_warning', label: 'Storage warning', hint: 'When you’re close to your storage limit.' },
  { key: 'backup_complete', label: 'Backup complete', hint: 'When a scheduled backup finishes.' },
]

// Accessible switch built from T tokens (mirrors the Settings-window idiom).
function Switch({ on, busy, onToggle, label }: { on: boolean; busy: boolean; onToggle: () => void; label: string }) {
  return (
    <button
      role="switch"
      aria-checked={on}
      aria-label={label}
      aria-busy={busy}
      disabled={busy}
      onClick={onToggle}
      style={{
        width: 36,
        height: 20,
        borderRadius: 999,
        border: 'none',
        padding: 2,
        background: on ? T.amberDeep : T.line2,
        cursor: busy ? 'progress' : 'pointer',
        opacity: busy ? 0.6 : 1,
        transition: 'background 150ms ease, opacity 150ms ease',
        display: 'inline-flex',
        alignItems: 'center',
        flexShrink: 0,
      }}
    >
      <div
        style={{
          width: 16,
          height: 16,
          borderRadius: '50%',
          background: T.paper,
          transform: on ? 'translateX(16px)' : 'translateX(0)',
          transition: 'transform 150ms ease',
          boxShadow: '0 1px 3px rgba(0,0,0,0.25)',
        }}
      />
    </button>
  )
}

function PreferencesSection() {
  const [values, setValues] = useState<NotificationPreferenceValues | null>(null)
  const [err, setErr] = useState<{ reason: string; unsupported: boolean } | null>(null)
  const [loading, setLoading] = useState(true)
  // Per-key in-flight + a transient per-key save error.
  const [busyKey, setBusyKey] = useState<PrefKey | null>(null)
  const [saveError, setSaveError] = useState<string | null>(null)

  const load = () => {
    let cancelled = false
    setLoading(true)
    setErr(null)
    setSaveError(null)
    void (async () => {
      const r = await accountNotificationPreferences()
      if (cancelled) return
      if (r.ok) { setValues(r.value.preferences); setErr(null) }
      else { setErr({ reason: r.reason, unsupported: r.unsupported }); setValues(null) }
      setLoading(false)
    })()
    return () => { cancelled = true }
  }

  useEffect(() => {
    let active = true
    setLoading(true)
    setErr(null)
    void (async () => {
      const r = await accountNotificationPreferences()
      if (!active) return
      if (r.ok) setValues(r.value.preferences)
      else setErr({ reason: r.reason, unsupported: r.unsupported })
      setLoading(false)
    })()
    return () => { active = false }
  }, [])

  const toggle = (key: PrefKey) => {
    if (!values || busyKey) return
    const prev = values
    const next = !values[key]
    // Optimistic: flip immediately, confirm with the returned state, revert on fail.
    setValues({ ...values, [key]: next })
    setBusyKey(key)
    setSaveError(null)
    void (async () => {
      const r = await accountUpdateNotificationPreferences({ [key]: next })
      if (r.ok) {
        setValues(r.value.preferences)
      } else {
        setValues(prev)
        setSaveError(
          r.unsupported
            ? 'Changing this preference isn’t available in this build.'
            : `Couldn’t save: ${r.reason}`,
        )
      }
      setBusyKey(null)
    })()
  }

  if (loading) return <PreferencesSkeleton />
  if (err) return <ErrorBlock reason={err.reason} unsupported={err.unsupported} onRetry={load} />
  if (!values) {
    // Defensive: ok-but-no-values should never crash to a white screen.
    return (
      <EmptyBlock
        icon="cog"
        title="No preferences to show"
        body="Notification preferences couldn’t be read for this account."
      />
    )
  }

  return (
    <Card style={{ padding: 0, overflow: 'hidden' }}>
      <div style={{ padding: '13px 20px', borderBottom: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', gap: 10 }}>
        <NavIcon name="cog" size={14} color={T.ink2} />
        <div style={{ minWidth: 0 }}>
          <div style={{ fontSize: 12.5, fontWeight: 600, color: T.ink }}>Preferences</div>
          <div style={{ fontSize: 11, color: T.ink3 }}>Choose which events notify you.</div>
        </div>
      </div>

      {PREF_META.map((m, i) => {
        const last = i === PREF_META.length - 1
        const on = values[m.key]
        return (
          <div
            key={m.key}
            style={{ display: 'grid', gridTemplateColumns: '1fr auto', gap: 16, padding: '14px 20px', alignItems: 'center', borderBottom: last ? 'none' : `1px solid ${T.line}` }}
          >
            <div style={{ minWidth: 0 }}>
              <div style={{ fontSize: 13, fontWeight: 500, color: T.ink }}>{m.label}</div>
              <div style={{ fontSize: 11, color: T.ink3, marginTop: 3, lineHeight: 1.5 }}>{m.hint}</div>
            </div>
            <Switch on={on} busy={busyKey === m.key} onToggle={() => toggle(m.key)} label={m.label} />
          </div>
        )
      })}

      {saveError && (
        <div style={{ padding: '11px 20px', borderTop: `1px solid ${T.line}`, background: T.paper2, fontSize: 11.5, color: RED, lineHeight: 1.5 }}>
          {saveError}
        </div>
      )}
    </Card>
  )
}

function PreferencesSkeleton() {
  return (
    <Card style={{ padding: 0, overflow: 'hidden' }}>
      <div style={{ padding: '13px 20px', borderBottom: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', gap: 10 }}>
        <NavIcon name="cog" size={14} color={T.ink4} />
        <Skeleton width={110} height={12} />
      </div>
      {Array.from({ length: 5 }).map((_, i) => (
        <div key={i} style={{ display: 'grid', gridTemplateColumns: '1fr 36px', gap: 16, padding: '14px 20px', alignItems: 'center', borderBottom: i < 4 ? `1px solid ${T.line}` : 'none' }}>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
            <Skeleton width={`${44 - i * 4}%`} height={12} />
            <Skeleton width={`${68 - i * 4}%`} height={10} />
          </div>
          <Skeleton width={36} height={20} radius={999} />
        </div>
      ))}
    </Card>
  )
}

// ── Tab switch ───────────────────────────────────────────────────────────────

type Tab = 'timeline' | 'notifications' | 'preferences'

const TABS: Array<{ id: Tab; label: string }> = [
  { id: 'timeline', label: 'Timeline' },
  { id: 'notifications', label: 'Notifications' },
  { id: 'preferences', label: 'Preferences' },
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

  return (
    <div style={{ overflow: 'auto', padding: '28px 36px', flex: 1 }}>
      <PageHeader
        title="Activity"
        subtitle="Your account’s timeline, notifications, and what we’re allowed to tell you about. We can’t read your files — these are account-level events only."
        aside={
          <Chip tone="amber">
            <NavIcon name="shield" size={11} color={T.amberDeep} />
            Falkenstein · Hetzner
          </Chip>
        }
      />

      <TabBar active={tab} onChange={setTab} />

      {/* Each section self-fetches on mount; remounting on tab change keeps the
          four-state handling self-contained and the data fresh on return. */}
      {tab === 'timeline' && <TimelineSection key="timeline" />}
      {tab === 'notifications' && <NotificationsSection key="notifications" />}
      {tab === 'preferences' && <PreferencesSection key="preferences" />}
    </div>
  )
}
