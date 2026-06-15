/**
 * BandwidthView — live transfer telemetry from the sync engine heartbeat
 * (PKG-DATA bind for the `bandwidth` nav slot, wave-2).
 *
 * Self-fetches accountClientSessions() on mount, then polls every ~10 s so the
 * numbers stay current without competing for the account-data rate limit. This
 * is telemetry, not a live wire — the engine heartbeat itself only fires every
 * ~20 s, so polling faster than 10 s buys nothing but extra 429 pressure on the
 * shared account endpoints. The poll is SILENT (never resets to a skeleton) and
 * keeps the last-known data on a failed tick. Cleans up the interval on unmount.
 *
 * For each session we render:
 *   - Device line: device_hostname + device_platform, session name
 *   - Status badge (syncing→amber, watching/idle→green/neutral, paused→neutral,
 *     error→red). Amber ONLY for the genuinely-active 'syncing' state.
 *   - Progress: files_synced/files_total + bytes breakdown (mono)
 *   - Speed: speed_bps > 0 → formatBytes(speed_bps)+"/s" (mono, neutral — it's
 *     a machine value, NOT amber)
 *   - current_file: rendered only when present (often null — the engine leaves it
 *     null for privacy; we never show an empty "Current: " label)
 *   - last_heartbeat → "updated Ns ago" (mono)
 *
 * Aggregate header: total throughput (sum of speed_bps) + active session count.
 *
 * All four async states: loading (Skeleton rows in the real card shape), error
 * (unsupported → calm note; otherwise reason + Retry), empty (clear message),
 * loaded.
 *
 * FOLLOW-UP (not in scope): rate-limit pickers (upload/download kbps). There is
 * no config-write IPC registered for bandwidth limits yet — the DesktopConfig
 * type has upload_kbps_limit/download_kbps_limit but setting them requires a
 * `write_desktop_config` or equivalent Tauri command that hasn't been wired in
 * this build. Add the pickers once that IPC exists.
 *
 * Brand: amber only for 'syncing' badge + primary buttons. JetBrains Mono for
 * all machine values (bytes, speed, counts, timestamps, hostnames). Inter for
 * human copy. No emojis. Honest voice.
 *
 * Design grounding: design/hifi/hifi-billing.jsx / hifi-settings.jsx.
 * Sibling idiom: DevicesView.tsx + AccountView.tsx.
 */

import { useEffect, useState } from 'react'
import { accountClientSessions, formatBytes, type ClientSyncSession } from '../../desktopApi'
import { T, Card, PageHeader, Chip, Skeleton, NavIcon, PrimaryBtn } from '../ui'

// ── Constants ────────────────────────────────────────────────────────────────

// Telemetry refresh cadence. The engine heartbeat is ~20s and this poll shares
// the account-data rate limit, so 10s is the sweet spot: current enough to feel
// live, slow enough not to hammer. (Was 3s — that competed on the 429 budget.)
const POLL_INTERVAL_MS = 10_000

const RED = 'oklch(0.5 0.18 25)'
const RED_BG = 'oklch(0.97 0.02 25)'
const RED_BORDER = 'oklch(0.85 0.08 25)'

// ── Helpers ──────────────────────────────────────────────────────────────────

/** Relative "N ago" for a heartbeat timestamp (RFC3339 or null). */
function relativeSecsAgo(iso: string | null | undefined): string | null {
  if (!iso) return null
  const then = Date.parse(iso)
  if (Number.isNaN(then)) return null
  const diffMs = Date.now() - then
  const sec = Math.round(diffMs / 1000)
  if (sec < 0) return 'just now'
  if (sec < 5) return 'just now'
  if (sec < 60) return `${sec}s ago`
  const min = Math.round(sec / 60)
  if (min < 60) return `${min}m ago`
  const hr = Math.round(min / 60)
  if (hr < 24) return `${hr}h ago`
  const day = Math.round(hr / 24)
  return `${day}d ago`
}

function shortPlatform(platform: string | null | undefined): string {
  if (!platform) return ''
  const lower = platform.trim().toLowerCase()
  if (lower === 'macos' || lower === 'darwin') return 'macOS'
  if (lower === 'windows' || lower === 'win32') return 'Windows'
  if (lower === 'linux') return 'Linux'
  if (lower === 'ios') return 'iOS'
  if (lower === 'android') return 'Android'
  return platform.trim()
}

// ── Active-session predicate (single source of truth) ────────────────────────

/**
 * A session counts as "active" when the sync engine is either watching the
 * filesystem for changes OR actively transferring bytes.  Paused, idle, error,
 * and stopped sessions are NOT active.
 *
 * This predicate is used by BOTH the per-row StatusBadge (for the green/amber
 * visual indicator) and the ThroughputHeader aggregate counts, so the two can
 * never disagree about what "active" means.
 */
function isActiveSession(status: string | null | undefined): boolean {
  const s = (status ?? '').trim().toLowerCase()
  return s === 'watching' || s === 'syncing'
}

// ── Status badge ─────────────────────────────────────────────────────────────

type Tone = 'green' | 'amber' | 'neutral' | 'red'

function statusMeta(status: string | null | undefined): { label: string; tone: Tone } {
  const s = (status ?? '').trim().toLowerCase()
  switch (s) {
    case 'watching':
      return { label: 'Watching', tone: 'green' }
    case 'syncing':
      // Amber = live encryption-in-progress signal. ONLY for this state.
      return { label: 'Syncing', tone: 'amber' }
    case 'paused':
      return { label: 'Paused', tone: 'neutral' }
    case 'idle':
      return { label: 'Idle', tone: 'neutral' }
    case 'error':
    case 'failed':
      return { label: s === 'failed' ? 'Failed' : 'Error', tone: 'red' }
    case '':
      return { label: 'Unknown', tone: 'neutral' }
    default:
      return { label: s.charAt(0).toUpperCase() + s.slice(1), tone: 'neutral' }
  }
}

function StatusBadge({ status }: { status: string | null | undefined }) {
  const { label, tone } = statusMeta(status)

  if (tone === 'green' || tone === 'amber' || tone === 'neutral') {
    const chipTone = tone === 'green' ? 'green' : tone === 'amber' ? 'amber' : 'neutral'
    return (
      <Chip tone={chipTone}>
        {tone !== 'neutral' && (
          <span
            style={{
              width: 6,
              height: 6,
              borderRadius: '50%',
              background: tone === 'green' ? T.green : T.amber,
              display: 'inline-block',
            }}
          />
        )}
        {label}
      </Chip>
    )
  }

  // red — built inline; Chip has no red tone
  return (
    <span
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 5,
        padding: '1px 7px',
        fontSize: 10,
        fontFamily: T.fontMono,
        border: `1px solid ${RED_BORDER}`,
        borderRadius: 999,
        background: RED_BG,
        color: RED,
        whiteSpace: 'nowrap' as const,
      }}
    >
      <span
        style={{ width: 6, height: 6, borderRadius: '50%', background: RED, display: 'inline-block' }}
      />
      {label}
    </span>
  )
}

// ── Session card ─────────────────────────────────────────────────────────────

function SessionCard({ session, last }: { session: ClientSyncSession; last: boolean }) {
  const { tone } = statusMeta(session.status)
  const isSyncing = session.status?.toLowerCase() === 'syncing'

  const filesSynced = session.files_synced
  const filesTotal = session.files_total
  const hasFileProgress =
    typeof filesSynced === 'number' && typeof filesTotal === 'number' && filesTotal > 0

  const bytesSynced = session.bytes_synced
  const bytesTotal = session.bytes_total
  const hasByteProgress =
    typeof bytesSynced === 'number' && typeof bytesTotal === 'number'

  const speed = session.speed_bps
  const hasSpeed = typeof speed === 'number' && speed > 0

  const pct =
    hasByteProgress && (bytesTotal as number) > 0
      ? Math.min(100, Math.max(0, ((bytesSynced as number) / (bytesTotal as number)) * 100))
      : hasFileProgress
        ? Math.min(100, Math.max(0, ((filesSynced as number) / (filesTotal as number)) * 100))
        : null

  const heartbeatAgo = relativeSecsAgo(session.last_heartbeat)
  const sessionName = session.name?.trim() || session.session_type?.trim() || 'Sync session'
  const hostname = session.device_hostname?.trim()
  const platform = shortPlatform(session.device_platform)

  return (
    <div
      style={{
        padding: '14px 18px',
        borderBottom: last ? 'none' : `1px solid ${T.line}`,
        display: 'flex',
        flexDirection: 'column',
        gap: 9,
      }}
    >
      {/* Device + session name row */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
        <div
          style={{
            width: 32,
            height: 32,
            borderRadius: 8,
            background: tone === 'amber' ? T.amberBg : T.paper2,
            border: `1px solid ${tone === 'amber' ? 'oklch(0.86 0.07 90)' : T.line}`,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            flexShrink: 0,
          }}
        >
          <NavIcon name="device" size={14} color={tone === 'amber' ? T.amberDeep : T.ink3} />
        </div>

        <div style={{ flex: 1, minWidth: 0 }}>
          <div
            style={{
              fontSize: 13,
              fontWeight: 600,
              color: T.ink,
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap' as const,
            }}
          >
            {hostname
              ? (
                <>
                  <span style={{ fontFamily: T.fontMono, fontSize: 12 }}>{hostname}</span>
                  {platform && (
                    <span style={{ fontFamily: T.fontSans, fontSize: 11, color: T.ink3, fontWeight: 400, marginLeft: 6 }}>
                      · {platform}
                    </span>
                  )}
                </>
              )
              : sessionName}
          </div>
          {hostname && (
            <div
              style={{
                fontSize: 11.5,
                color: T.ink3,
                marginTop: 2,
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                whiteSpace: 'nowrap' as const,
              }}
            >
              {sessionName}
            </div>
          )}
        </div>

        <StatusBadge status={session.status} />
      </div>

      {/* Progress bar */}
      {pct != null && (
        <div className="progress-track">
          <div className="progress-fill" style={{ width: `${pct}%` }} />
        </div>
      )}

      {/* Machine values row: progress counts, byte sizes, speed, heartbeat */}
      {(hasFileProgress || hasByteProgress || hasSpeed || heartbeatAgo) && (
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 14,
            flexWrap: 'wrap' as const,
            fontSize: 11,
            fontFamily: T.fontMono,
            color: T.ink3,
          }}
        >
          {hasFileProgress && (
            <span>
              {(filesSynced as number).toLocaleString()} / {(filesTotal as number).toLocaleString()} files
            </span>
          )}
          {hasByteProgress && (
            <span>
              {formatBytes(bytesSynced as number)} / {formatBytes(bytesTotal as number)}
            </span>
          )}
          {hasSpeed && (
            // Speed is a machine value — neutral color, NOT amber.
            <span style={{ color: T.ink2 }}>{formatBytes(speed as number)}/s</span>
          )}
          {heartbeatAgo && (
            <span style={{ color: T.ink4, marginLeft: 'auto' }}>updated {heartbeatAgo}</span>
          )}
        </div>
      )}

      {/* current_file — only when actively syncing AND the field is non-null.
          The engine leaves it null for privacy most of the time; never show
          an empty label. */}
      {isSyncing && session.current_file && (
        <div
          style={{
            fontSize: 11,
            fontFamily: T.fontMono,
            color: T.ink3,
            background: T.paper2,
            border: `1px solid ${T.line}`,
            borderRadius: 6,
            padding: '6px 9px',
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap' as const,
          }}
        >
          {session.current_file}
        </div>
      )}
    </div>
  )
}

// ── Aggregate throughput header ───────────────────────────────────────────────

function ThroughputHeader({ sessions }: { sessions: ClientSyncSession[] }) {
  // Use the shared predicate so the header count always matches what the rows show.
  const activeSessions = sessions.filter((s) => isActiveSession(s.status))
  const totalSpeedBps = activeSessions.reduce(
    (sum, s) => sum + (typeof s.speed_bps === 'number' && s.speed_bps > 0 ? s.speed_bps : 0),
    0,
  )
  const hasThroughput = totalSpeedBps > 0
  // Amber "Syncing" chip only when at least one session is actively transferring bytes.
  const anySyncing = activeSessions.some((s) => s.status?.trim().toLowerCase() === 'syncing')

  return (
    <div
      style={{
        padding: '14px 20px',
        borderRadius: 10,
        background: hasThroughput
          ? `linear-gradient(135deg, ${T.amberBg}, ${T.paper2})`
          : T.paper2,
        border: `1px solid ${hasThroughput ? T.amberDeep : T.line}`,
        marginBottom: 20,
        display: 'flex',
        alignItems: 'center',
        gap: 32,
      }}
    >
      <div>
        <div
          style={{
            fontSize: 9.5,
            fontFamily: T.fontMono,
            textTransform: 'uppercase' as const,
            letterSpacing: '0.07em',
            color: T.ink3,
            marginBottom: 4,
          }}
        >
          Total throughput
        </div>
        <div
          style={{
            fontSize: 20,
            fontWeight: 700,
            fontFamily: T.fontMono,
            color: T.ink,
            letterSpacing: '-0.02em',
          }}
        >
          {hasThroughput ? `${formatBytes(totalSpeedBps)}/s` : '—'}
        </div>
      </div>

      <div>
        <div
          style={{
            fontSize: 9.5,
            fontFamily: T.fontMono,
            textTransform: 'uppercase' as const,
            letterSpacing: '0.07em',
            color: T.ink3,
            marginBottom: 4,
          }}
        >
          Active sessions
        </div>
        <div
          style={{
            fontSize: 20,
            fontWeight: 700,
            fontFamily: T.fontMono,
            color: T.ink,
            letterSpacing: '-0.02em',
          }}
        >
          {activeSessions.length}
        </div>
      </div>

      {anySyncing && (
        <Chip tone="amber">
          <span
            style={{
              width: 6,
              height: 6,
              borderRadius: '50%',
              background: T.amber,
              display: 'inline-block',
            }}
          />
          Syncing
        </Chip>
      )}
    </div>
  )
}

// ── Loading state ─────────────────────────────────────────────────────────────

function LoadingState() {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
      {/* Aggregate header skeleton */}
      <div
        style={{
          padding: '14px 20px',
          borderRadius: 10,
          background: T.paper2,
          border: `1px solid ${T.line}`,
          marginBottom: 6,
          display: 'flex',
          alignItems: 'center',
          gap: 32,
        }}
      >
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          <Skeleton width={80} height={9} />
          <Skeleton width={72} height={18} />
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          <Skeleton width={72} height={9} />
          <Skeleton width={28} height={18} />
        </div>
      </div>

      {/* Session card skeletons */}
      {Array.from({ length: 2 }).map((_, i) => (
        <Card key={i} style={{ overflow: 'hidden' }}>
          <div style={{ padding: '14px 18px', display: 'flex', flexDirection: 'column', gap: 10 }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
              <Skeleton width={32} height={32} radius={8} />
              <div style={{ flex: 1, display: 'flex', flexDirection: 'column', gap: 6 }}>
                <Skeleton width={`${48 - i * 8}%`} height={12} />
                <Skeleton width={`${34 - i * 5}%`} height={10} />
              </div>
              <Skeleton width={62} height={16} radius={999} />
            </div>
            <Skeleton height={8} radius={999} />
            <div style={{ display: 'flex', gap: 14 }}>
              <Skeleton width={90} height={10} />
              <Skeleton width={80} height={10} />
              <Skeleton width={56} height={10} />
            </div>
          </div>
        </Card>
      ))}
    </div>
  )
}

// ── Error state ───────────────────────────────────────────────────────────────

function ErrorState({
  unsupported,
  reason,
  onRetry,
}: {
  unsupported: boolean
  reason: string
  onRetry: () => void
}) {
  if (unsupported) {
    return (
      <Card
        style={{
          padding: '28px 24px',
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          gap: 8,
          textAlign: 'center' as const,
        }}
      >
        <NavIcon name="bolt" size={22} color={T.ink4} />
        <div style={{ fontSize: 14, fontWeight: 600, color: T.ink, marginTop: 4 }}>
          Not available in this build
        </div>
        <div style={{ fontSize: 12, color: T.ink3, lineHeight: 1.6, maxWidth: 340 }}>
          Live transfer telemetry isn't wired into this desktop build yet. It will appear here once
          the sync engine reports session heartbeats.
        </div>
      </Card>
    )
  }

  return (
    <Card
      style={{
        padding: '24px',
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        gap: 10,
        textAlign: 'center' as const,
      }}
    >
      <NavIcon name="bolt" size={22} color={RED} />
      <div style={{ fontSize: 14, fontWeight: 600, color: T.ink, marginTop: 4 }}>
        Couldn't load transfer sessions
      </div>
      <div
        style={{
          fontSize: 11.5,
          fontFamily: T.fontMono,
          color: T.ink3,
          lineHeight: 1.5,
          maxWidth: 380,
          wordBreak: 'break-word' as const,
        }}
      >
        {reason}
      </div>
      <div style={{ marginTop: 4 }}>
        <PrimaryBtn onClick={onRetry}>Retry</PrimaryBtn>
      </div>
    </Card>
  )
}

// ── Empty state ───────────────────────────────────────────────────────────────

function EmptyState() {
  return (
    <Card
      style={{
        padding: '32px 24px',
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        gap: 8,
        textAlign: 'center' as const,
      }}
    >
      <div
        style={{
          width: 46,
          height: 46,
          borderRadius: 11,
          background: T.paper2,
          border: `1px solid ${T.line}`,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
        }}
      >
        <NavIcon name="bolt" size={20} color={T.ink3} />
      </div>
      <div style={{ fontSize: 14, fontWeight: 600, color: T.ink, marginTop: 4 }}>
        No active sync sessions
      </div>
      <div style={{ fontSize: 12, color: T.ink3, lineHeight: 1.6, maxWidth: 340 }}>
        This device reports in once it's syncing. Start a sync session and it will appear here with
        live transfer telemetry.
      </div>
    </Card>
  )
}

// ── Loaded sessions list ──────────────────────────────────────────────────────

function SessionList({ sessions }: { sessions: ClientSyncSession[] }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
      <ThroughputHeader sessions={sessions} />
      <Card style={{ overflow: 'hidden' }}>
        {sessions.map((session, i) => (
          <SessionCard key={session.id} session={session} last={i === sessions.length - 1} />
        ))}
      </Card>
    </div>
  )
}

// ── Async state type ──────────────────────────────────────────────────────────

type LoadState =
  | { phase: 'loading' }
  | { phase: 'error'; unsupported: boolean; reason: string }
  | { phase: 'ready'; sessions: ClientSyncSession[] }

// ── Root view ─────────────────────────────────────────────────────────────────

export default function BandwidthView() {
  const [state, setState] = useState<LoadState>({ phase: 'loading' })

  const load = () => {
    setState({ phase: 'loading' })
    let cancelled = false
    void (async () => {
      const result = await accountClientSessions()
      if (cancelled) return
      if (!result.ok) {
        setState({ phase: 'error', unsupported: result.unsupported, reason: result.reason })
        return
      }
      const sessions = Array.isArray(result.value.sessions) ? result.value.sessions : []
      setState({ phase: 'ready', sessions })
    })()
    return () => { cancelled = true }
  }

  // Initial fetch + poll for live updates.
  useEffect(() => {
    const cancel = load()
    const id = window.setInterval(() => {
      // Poll silently: do not reset to loading on subsequent polls so the UI
      // never flickers to a skeleton when it already has data.
      void (async () => {
        const result = await accountClientSessions()
        setState((prev) => {
          if (prev.phase === 'loading') return prev // first fetch still in flight
          if (!result.ok) return prev // keep stale data on poll failure
          const sessions = Array.isArray(result.value.sessions) ? result.value.sessions : []
          return { phase: 'ready', sessions }
        })
      })()
    }, POLL_INTERVAL_MS)

    return () => {
      cancel?.()
      window.clearInterval(id)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const sessionCount = state.phase === 'ready' ? state.sessions.length : null

  return (
    <div style={{ overflow: 'auto', padding: '28px 36px', flex: 1 }}>
      <PageHeader
        title="Bandwidth"
        subtitle="Live transfer telemetry from the sync engine. Numbers update every few seconds as the engine reports heartbeats."
        aside={
          sessionCount != null && sessionCount > 0 ? (
            <Chip tone="neutral">
              <span style={{ fontFamily: T.fontMono }}>
                {sessionCount} {sessionCount === 1 ? 'session' : 'sessions'}
              </span>
            </Chip>
          ) : undefined
        }
      />

      {state.phase === 'loading' && <LoadingState />}

      {state.phase === 'error' && (
        <ErrorState
          unsupported={state.unsupported}
          reason={state.reason}
          onRetry={() => load()}
        />
      )}

      {state.phase === 'ready' && state.sessions.length === 0 && <EmptyState />}

      {state.phase === 'ready' && state.sessions.length > 0 && (
        <SessionList sessions={state.sessions} />
      )}
    </div>
  )
}
