/**
 * DevicesView — every device holding a copy of your vault key, and the sync
 * sessions running on each (PKG-DATA bind for the `devices` nav slot).
 *
 * Self-fetching on mount: accountDevices() + accountClientSessions(). Sessions
 * are grouped under their device (session.device_id → device.id, falling back to
 * device_hostname → hostname). Handles all four states — loading (skeleton rows
 * in the real card shape), error (unsupported → calm note; otherwise reason +
 * Retry), empty ("No devices registered yet"), and loaded.
 *
 * Brand: amber only for the live-encryption signal and primary action; Inter for
 * human text, JetBrains Mono for every machine value (ids, versions, byte sizes,
 * speeds, timestamps); name the city only ("Falkenstein"), never the storage provider; honest voice; no emojis. We do
 * NOT render a "This device" chip — no IPC gives us a trustworthy local-device
 * signal here, so guessing would be dishonest.
 *
 * Design grounding: design/hifi/hifi-security.jsx (Trusted devices) +
 * design/hifi/hifi-settings.jsx (HiSettingsDevices), adapted to the desktop idiom.
 */

import { useEffect, useState } from 'react'
import {
  accountDevices,
  accountClientSessions,
  formatBytes,
  type ClientDevice,
  type ClientSyncSession,
} from '../../desktopApi'
import { T, Card, PageHeader, Chip, Skeleton, NavIcon, PrimaryBtn } from '../ui'

// ── Date formatting (RFC3339 → human, shown in mono) ─────────────────────────

function relativeTime(iso: string | null | undefined): string | null {
  if (!iso) return null
  const then = Date.parse(iso)
  if (Number.isNaN(then)) return null
  const diffMs = Date.now() - then
  const sec = Math.round(diffMs / 1000)
  if (sec < 0) return 'just now'
  if (sec < 45) return 'just now'
  const min = Math.round(sec / 60)
  if (min < 60) return `${min} min ago`
  const hr = Math.round(min / 60)
  if (hr < 24) return `${hr}h ago`
  const day = Math.round(hr / 24)
  if (day < 14) return `${day}d ago`
  // Older than two weeks — show a short absolute date.
  const d = new Date(then)
  return d.toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' })
}

function shortPlatform(platform: string | null | undefined): string {
  if (!platform) return 'Unknown platform'
  const p = platform.trim()
  if (!p) return 'Unknown platform'
  const lower = p.toLowerCase()
  if (lower === 'macos' || lower === 'darwin') return 'macOS'
  if (lower === 'windows' || lower === 'win32') return 'Windows'
  if (lower === 'linux') return 'Linux'
  if (lower === 'ios') return 'iOS'
  if (lower === 'android') return 'Android'
  return p
}

// ── Status → tone mapping (matches the brand: amber only for active work) ────

type Tone = 'green' | 'amber' | 'neutral' | 'red'

function statusMeta(status: string | null | undefined): { label: string; tone: Tone } {
  const s = (status ?? '').trim().toLowerCase()
  switch (s) {
    case 'watching':
      return { label: 'Watching', tone: 'green' }
    case 'syncing':
      return { label: 'Syncing', tone: 'amber' }
    case 'paused':
      return { label: 'Paused', tone: 'amber' }
    case 'idle':
      return { label: 'Idle', tone: 'neutral' }
    case 'error':
    case 'failed':
      return { label: s === 'failed' ? 'Failed' : 'Error', tone: 'red' }
    case '':
      return { label: 'Unknown', tone: 'neutral' }
    default:
      // Title-case an unexpected status rather than dropping it.
      return { label: s.charAt(0).toUpperCase() + s.slice(1), tone: 'neutral' }
  }
}

const RED = 'oklch(0.5 0.18 25)'
const RED_BG = 'oklch(0.97 0.02 25)'
const RED_BORDER = 'oklch(0.85 0.08 25)'

function StatusBadge({ status }: { status: string | null | undefined }) {
  const { label, tone } = statusMeta(status)
  if (tone === 'green' || tone === 'amber' || tone === 'neutral') {
    const chipTone = tone === 'green' ? 'green' : tone === 'amber' ? 'amber' : 'neutral'
    return (
      <Chip tone={chipTone}>
        {tone !== 'neutral' && (
          <span style={{ width: 6, height: 6, borderRadius: '50%', background: tone === 'green' ? T.green : T.amber, display: 'inline-block' }} />
        )}
        {label}
      </Chip>
    )
  }
  // red-ish — built inline since Chip has no red tone.
  return (
    <span style={{
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
    }}>
      <span style={{ width: 6, height: 6, borderRadius: '50%', background: RED, display: 'inline-block' }} />
      {label}
    </span>
  )
}

// ── Session row ──────────────────────────────────────────────────────────────

function SessionRow({ session, last }: { session: ClientSyncSession; last: boolean }) {
  const meta = statusMeta(session.status)
  const isSyncing = meta.tone === 'amber' && (session.status ?? '').toLowerCase() === 'syncing'

  const filesSynced = session.files_synced
  const filesTotal = session.files_total
  const hasFileProgress = typeof filesSynced === 'number' && typeof filesTotal === 'number' && filesTotal > 0

  const bytesSynced = session.bytes_synced
  const bytesTotal = session.bytes_total
  const hasByteProgress = typeof bytesSynced === 'number' && typeof bytesTotal === 'number'

  const speed = session.speed_bps
  const hasSpeed = typeof speed === 'number' && speed > 0

  const pct = hasByteProgress && (bytesTotal as number) > 0
    ? Math.min(100, Math.max(0, ((bytesSynced as number) / (bytesTotal as number)) * 100))
    : hasFileProgress
      ? Math.min(100, Math.max(0, ((filesSynced as number) / (filesTotal as number)) * 100))
      : null

  const heartbeat = relativeTime(session.last_heartbeat)
  const name = session.name?.trim() || session.session_type?.trim() || 'Sync session'

  return (
    <div style={{
      padding: '12px 16px',
      borderBottom: last ? 'none' : `1px solid ${T.line}`,
      display: 'flex',
      flexDirection: 'column',
      gap: 8,
    }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
        <NavIcon name="folder" size={13} color={T.ink3} />
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ fontSize: 12.5, fontWeight: 500, color: T.ink, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' as const }}>
            {name}
          </div>
          {session.remote_path && (
            <div style={{ fontSize: 11, fontFamily: T.fontMono, color: T.ink3, marginTop: 2, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' as const }}>
              {session.remote_path}
            </div>
          )}
        </div>
        <StatusBadge status={session.status} />
      </div>

      {/* Progress bar (when we have any progress signal) */}
      {pct != null && (
        <div className="progress-track">
          <div className="progress-fill" style={{ width: `${pct}%` }} />
        </div>
      )}

      {/* Progress numbers + speed — all machine values in mono */}
      {(hasFileProgress || hasByteProgress || hasSpeed || heartbeat) && (
        <div style={{ display: 'flex', alignItems: 'center', gap: 14, flexWrap: 'wrap' as const, fontSize: 11, fontFamily: T.fontMono, color: T.ink3 }}>
          {hasFileProgress && (
            <span>{(filesSynced as number).toLocaleString()} / {(filesTotal as number).toLocaleString()} files</span>
          )}
          {hasByteProgress && (
            <span>{formatBytes(bytesSynced as number)} / {formatBytes(bytesTotal as number)}</span>
          )}
          {hasSpeed && (
            <span style={{ color: T.ink2 }}>{formatBytes(speed as number)}/s</span>
          )}
          {heartbeat && (
            <span style={{ color: T.ink4, marginLeft: 'auto' }}>heartbeat {heartbeat}</span>
          )}
        </div>
      )}

      {/* Current file — only when actively syncing */}
      {isSyncing && session.current_file && (
        <div style={{
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
        }}>
          {session.current_file}
        </div>
      )}
    </div>
  )
}

// ── Device card ──────────────────────────────────────────────────────────────

function DeviceCard({ device, sessions }: { device: ClientDevice; sessions: ClientSyncSession[] }) {
  const lastSeen = relativeTime(device.last_seen)
  const created = relativeTime(device.created_at)
  const sessionCount = device.session_count

  return (
    <Card style={{ overflow: 'hidden' }}>
      {/* Device header */}
      <div style={{ padding: '16px 18px', display: 'flex', alignItems: 'center', gap: 14, borderBottom: sessions.length > 0 ? `1px solid ${T.line}` : 'none' }}>
        <div style={{
          width: 40,
          height: 40,
          borderRadius: 9,
          background: T.paper2,
          border: `1px solid ${T.line}`,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          flexShrink: 0,
          color: T.ink2,
        }}>
          <NavIcon name="device" size={17} color={T.ink2} />
        </div>

        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' as const }}>
            <span style={{ fontSize: 14, fontWeight: 600, color: T.ink, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' as const }}>
              {device.hostname?.trim() || 'Unnamed device'}
            </span>
            {device.bb_version && (
              <Chip>v{device.bb_version}</Chip>
            )}
          </div>
          <div style={{ fontSize: 11.5, color: T.ink3, marginTop: 3, display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' as const }}>
            <span>{shortPlatform(device.platform)}</span>
            <span style={{ color: T.ink4 }}>·</span>
            <span style={{ fontFamily: T.fontMono }}>
              {lastSeen ? `last seen ${lastSeen}` : 'last seen unknown'}
            </span>
            {created && (
              <>
                <span style={{ color: T.ink4 }}>·</span>
                <span style={{ fontFamily: T.fontMono, color: T.ink4 }}>registered {created}</span>
              </>
            )}
          </div>
        </div>

        <span style={{ fontSize: 11, fontFamily: T.fontMono, color: T.ink3, whiteSpace: 'nowrap' as const, flexShrink: 0 }}>
          {sessionCount} {sessionCount === 1 ? 'session' : 'sessions'}
        </span>
      </div>

      {/* Sync sessions running on this device */}
      {sessions.length > 0 ? (
        <div>
          {sessions.map((s, i) => (
            <SessionRow key={s.id} session={s} last={i === sessions.length - 1} />
          ))}
        </div>
      ) : sessionCount > 0 ? (
        <div style={{ padding: '12px 18px', fontSize: 11.5, color: T.ink3, lineHeight: 1.5 }}>
          {sessionCount} active {sessionCount === 1 ? 'session' : 'sessions'} on this device — live sync detail isn't reported for them.
        </div>
      ) : null}
    </Card>
  )
}

// ── State scaffolds ──────────────────────────────────────────────────────────

function LoadingState() {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
      {Array.from({ length: 2 }).map((_, i) => (
        <Card key={i} style={{ overflow: 'hidden' }}>
          <div style={{ padding: '16px 18px', display: 'flex', alignItems: 'center', gap: 14, borderBottom: `1px solid ${T.line}` }}>
            <Skeleton width={40} height={40} radius={9} />
            <div style={{ flex: 1, display: 'flex', flexDirection: 'column', gap: 7 }}>
              <Skeleton width={`${42 - i * 8}%`} height={13} />
              <Skeleton width={`${64 - i * 6}%`} height={10} />
            </div>
            <Skeleton width={64} height={11} />
          </div>
          <div style={{ padding: '12px 16px', display: 'flex', flexDirection: 'column', gap: 8 }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
              <Skeleton width={13} height={13} radius={4} />
              <div style={{ flex: 1 }}><Skeleton width="48%" height={11} /></div>
              <Skeleton width={56} height={14} radius={999} />
            </div>
            <Skeleton height={8} radius={999} />
          </div>
        </Card>
      ))}
    </div>
  )
}

function ErrorState({ unsupported, reason, onRetry }: { unsupported: boolean; reason: string; onRetry: () => void }) {
  if (unsupported) {
    return (
      <Card style={{ padding: '28px 24px', display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 8, textAlign: 'center' as const }}>
        <NavIcon name="device" size={22} color={T.ink4} />
        <div style={{ fontSize: 14, fontWeight: 600, color: T.ink, marginTop: 4 }}>Not available in this build</div>
        <div style={{ fontSize: 12, color: T.ink3, lineHeight: 1.6, maxWidth: 340 }}>
          The device registry isn't wired into this desktop build yet. It will appear here once the client reports its registered devices.
        </div>
      </Card>
    )
  }
  return (
    <Card style={{ padding: '24px', display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 10, textAlign: 'center' as const }}>
      <NavIcon name="device" size={22} color={RED} />
      <div style={{ fontSize: 14, fontWeight: 600, color: T.ink, marginTop: 4 }}>Couldn't load your devices</div>
      <div style={{ fontSize: 11.5, fontFamily: T.fontMono, color: T.ink3, lineHeight: 1.5, maxWidth: 380, wordBreak: 'break-word' as const }}>
        {reason}
      </div>
      <div style={{ marginTop: 4 }}>
        <PrimaryBtn onClick={onRetry}>Retry</PrimaryBtn>
      </div>
    </Card>
  )
}

function EmptyState() {
  return (
    <Card style={{ padding: '32px 24px', display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 8, textAlign: 'center' as const }}>
      <div style={{
        width: 46,
        height: 46,
        borderRadius: 11,
        background: T.paper2,
        border: `1px solid ${T.line}`,
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
      }}>
        <NavIcon name="device" size={20} color={T.ink3} />
      </div>
      <div style={{ fontSize: 14, fontWeight: 600, color: T.ink, marginTop: 4 }}>No devices registered yet</div>
      <div style={{ fontSize: 12, color: T.ink3, lineHeight: 1.6, maxWidth: 340 }}>
        Sign in on another device to see it here. Every device holds a copy of your vault key — revoking one re-encrypts your data against a new key.
      </div>
    </Card>
  )
}

// ── Root view ────────────────────────────────────────────────────────────────

type LoadState =
  | { phase: 'loading' }
  | { phase: 'error'; unsupported: boolean; reason: string }
  | { phase: 'ready'; devices: ClientDevice[]; sessions: ClientSyncSession[] }

export default function DevicesView() {
  const [state, setState] = useState<LoadState>({ phase: 'loading' })

  const load = () => {
    let cancelled = false
    setState({ phase: 'loading' })
    void (async () => {
      const [devRes, sessRes] = await Promise.all([accountDevices(), accountClientSessions()])
      if (cancelled) return

      // Devices are the spine — if they fail, the view is an error. Sessions are
      // additive; if only sessions fail we still render devices (without detail).
      if (!devRes.ok) {
        setState({ phase: 'error', unsupported: devRes.unsupported, reason: devRes.reason })
        return
      }

      const devices = Array.isArray(devRes.value.devices) ? devRes.value.devices : []
      const sessions = sessRes.ok && Array.isArray(sessRes.value.sessions) ? sessRes.value.sessions : []
      setState({ phase: 'ready', devices, sessions })
    })()
    return () => { cancelled = true }
  }

  useEffect(() => {
    const cancel = load()
    return cancel
  }, [])

  // Group sessions under their device: device_id → id, else device_hostname → hostname.
  const sessionsForDevice = (device: ClientDevice, sessions: ClientSyncSession[]): ClientSyncSession[] =>
    sessions.filter((s) => {
      if (s.device_id && device.id && s.device_id === device.id) return true
      if (s.device_hostname && device.hostname && s.device_hostname === device.hostname) return true
      return false
    })

  const activeDeviceCount = state.phase === 'ready' ? state.devices.length : null

  return (
    <div style={{ overflow: 'auto', padding: '28px 36px', flex: 1 }}>
      <PageHeader
        title="Devices"
        subtitle="Every device holding a copy of your vault key, and the sync sessions running on each. Revoke a device to re-encrypt your data against a new key."
        aside={
          activeDeviceCount != null && activeDeviceCount > 0 ? (
            <Chip tone="green">
              <span style={{ width: 6, height: 6, borderRadius: '50%', background: T.green, display: 'inline-block' }} />
              {activeDeviceCount} {activeDeviceCount === 1 ? 'device' : 'devices'}
            </Chip>
          ) : undefined
        }
      />

      {state.phase === 'loading' && <LoadingState />}

      {state.phase === 'error' && (
        <ErrorState unsupported={state.unsupported} reason={state.reason} onRetry={() => load()} />
      )}

      {state.phase === 'ready' && state.devices.length === 0 && <EmptyState />}

      {state.phase === 'ready' && state.devices.length > 0 && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
          {state.devices.map((device) => (
            <DeviceCard
              key={device.id}
              device={device}
              sessions={sessionsForDevice(device, state.sessions)}
            />
          ))}
        </div>
      )}
    </div>
  )
}
