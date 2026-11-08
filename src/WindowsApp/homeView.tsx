/**
 * Home: recent activity, sync metrics and the account summary the window opens on. The time and
 * event helpers translate raw activity rows into the product voice.
 */
import { useEffect, useState } from 'react'
import { command, formatBytes, accountProfile, accountActivity, type BillingUsage, type DesktopConfig, type StorageSummary, type SyncStatus, type AccountProfile, type AccountActivity, type AccountActivityEvent } from '../desktopApi'
import { T, NavIcon, Chip, Skeleton, PageHeader, Card } from '../windows/ui'
import { usePlatformName, thisDeviceNoun } from '../platform'
import { useSyncActivitySummary } from '../windows/syncActivityUi'
import { homeSyncMetrics, type HomeMetricLink, type HomeSyncMetric } from '../windows/homeSyncMetrics'

// ── Views ───────────────────────────────────────────────────────────────────

// ── HomeView helpers ─────────────────────────────────────────────────────────

/** Compact relative time — same logic as ActivityView, kept local so HomeView
 *  has no shared-module dep; both files stay standalone. */
export function homeRelativeTime(value?: string | null): string {
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
export function homeEventVisual(type: string): { icon: string; dot: string } {
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

export function homeHumanizeType(type: string): string {
  const raw = (type || '').replace(/[._]+/g, ' ').trim()
  if (!raw) return 'Activity'
  return raw.charAt(0).toUpperCase() + raw.slice(1)
}

// HOME — a status dashboard. Metric strip wired to live sync status. Body
// self-fetches accountProfile + accountActivity on mount. Degrades gracefully
// when the API is unavailable (login down in this build): metric strip stays,
// account body shows a calm signed-out prompt.
export function HomeView({
  status,
  usage,
  storage,
  onOpenList,
}: {
  status: SyncStatus | null
  usage: BillingUsage | null
  storage: StorageSummary | null
  /**
   * Task 2409: "Not sent N" opens Activity › Not sent, where those changes are listed; task 2414: "Not downloaded N"
   * opens Activity › Not downloaded.
   */
  onOpenList?: (section: HomeMetricLink) => void
}) {
  const platform = usePlatformName()
  // What sync is doing (0.8.15): the model Settings › Sync and the tray render, so Home never says "Up to date" over
  // held changes. Before this the strip read `status` alone and showed "Up to date · Syncing 0" next to 2,967 unsent
  // changes. The saved pause setting is the model's fallback only while the summary cannot be read (as in Settings).
  const { summary } = useSyncActivitySummary()
  const [pausedFallback, setPausedFallback] = useState<boolean | undefined>(undefined)
  useEffect(() => {
    let cancelled = false
    void command<DesktopConfig | null>('desktop_config').then((result) => {
      if (!cancelled && result.ok && result.value != null) setPausedFallback(result.value.pause_sync)
    })
    return () => { cancelled = true }
  }, [])

  const usedBytes = usage?.used_bytes ?? storage?.used_bytes ?? null
  const quotaBytes = usage?.quota_bytes ?? storage?.quota_bytes ?? null

  const metrics: HomeSyncMetric[] = [
    ...homeSyncMetrics(status, summary, { device: thisDeviceNoun(platform), pausedFallback }),
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
        subtitle={`Everything on ${thisDeviceNoun(platform)}, at a glance. Files are encrypted on your device before upload.`}
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
        // Four figures, five while "Not downloaded" shows (task 2414).
        gridTemplateColumns: `repeat(${metrics.length}, minmax(0, 1fr))`,
        gap: 20,
      }}>
        {metrics.map(({ k, v, link }) => (
          <div key={k}>
            <div style={{ fontSize: 9.5, fontFamily: T.fontMono, textTransform: 'uppercase' as const, letterSpacing: '0.07em', color: T.ink3, marginBottom: 4 }}>{k}</div>
            {link && onOpenList ? (
              // Task 2409: the count opens the list of what it counts. A link (it goes somewhere), underlined, in ink.
              <a
                href={`#${link}`}
                title={link === 'not-sent' ? 'See which changes' : 'See which files'}
                onClick={(event) => {
                  event.preventDefault()
                  onOpenList(link)
                }}
                style={{ fontSize: 16, fontWeight: 600, fontFamily: T.fontMono, color: T.ink, textDecoration: 'underline', textUnderlineOffset: 3, textDecorationColor: T.ink3 }}
              >
                {v}
              </a>
            ) : (
              <div style={{ fontSize: 16, fontWeight: 600, fontFamily: T.fontMono, color: T.ink }}>{v}</div>
            )}
          </div>
        ))}
        {/* Must-render row 10 (lead ruling FT-row10): why sync did not start, when the engine start
            was refused (on Windows: another account's local files on this PC). Rust's sentence,
            verbatim, and no action. */}
        {status?.engine_refusal && (
          <div role="status" data-engine-refusal={status.engine_refusal.code} style={{ gridColumn: '1 / -1', fontSize: 12.5, lineHeight: 1.5, color: T.ink }}>
            {status.engine_refusal.sentence}
          </div>
        )}
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
