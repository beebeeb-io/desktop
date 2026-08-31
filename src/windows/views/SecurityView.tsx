/**
 * SecurityView — the Security page inside the Windows main app window.
 *
 * Self-fetches on mount (no props). Binds three read IPCs + two destructive
 * IPCs from desktopApi.ts:
 *   - accountSecurityScore()        → SecurityScore { score, max?, label, factors[] }
 *   - accountSessionList()          → AccountSessionList { sessions: AccountSession[] }
 *   - accountRevokeSession(id)      → void           (per non-current session)
 *   - accountRevokeOtherSessions()  → RevokeAllResult { revoked }
 *
 * Layout grounded on design/hifi/hifi-security.jsx (score card + checklist,
 * trusted-device list, sign-in methods) — adapted to the desktop idiom (shared
 * `T` tokens, ~10–13px fonts, 6–10px radii, 1px T.line borders, Card/Chip/
 * PrimaryBtn/Skeleton/PageHeader). No props; the four states (loading / error /
 * empty / loaded) are each rendered intentionally.
 *
 * Honesty: enabling 2FA is NOT a wired desktop IPC, so the two-factor row never
 * renders an in-app enable/disable toggle — it states the truth and links to
 * beebeeb.io via openUrl. Destructive session revokes are gated behind an inline
 * confirm (not window.confirm); failures surface the server reason.
 *
 * Brand: amber is reserved for encryption state + primary/destructive-confirm
 * actions; Inter for human text, JetBrains Mono for ids/codes/timestamps;
 * name the city only ("Falkenstein"), never the storage provider; no emojis.
 */

import { useEffect, useState } from 'react'
import {
  accountSecurityScore,
  accountSessionList,
  accountRevokeSession,
  accountRevokeOtherSessions,
  openUrl,
  type SecurityScore,
  type SecurityFactor,
  type AccountSession,
} from '../../desktopApi'
import { T, Card, PageHeader, Chip, Skeleton, NavIcon, PrimaryBtn } from '../ui'

// ── Local palette extensions (warning/danger reuse WindowsApp's error tone) ──

const WARN = 'oklch(0.5 0.18 25)'
const WARN_BG = 'oklch(0.97 0.02 25)'
const WARN_LINE = 'oklch(0.85 0.08 25)'

const MANAGE_2FA_URL = 'https://app.beebeeb.io/settings/security'

// ── Factor humanization ──────────────────────────────────────────────────────
// Map the server's snake_case factor keys to a human label + the action implied
// when the factor is unsatisfied. Unknown keys fall back to a Title Case of the
// key so a new server factor still reads sensibly instead of as a raw token.

const FACTOR_COPY: Record<string, { label: string; action: string }> = {
  email_verified: { label: 'Email verified', action: 'Verify your email address' },
  two_factor_enabled: { label: 'Two-factor authentication', action: 'Add a second factor' },
  totp_enabled: { label: 'Two-factor authentication', action: 'Add a second factor' },
  strong_password: { label: 'Strong master password', action: 'Strengthen your master password' },
  recovery_kit_saved: { label: 'Recovery kit saved', action: 'Save your recovery kit' },
  recovery_phrase_saved: { label: 'Recovery phrase saved', action: 'Save your recovery phrase' },
  passkey_added: { label: 'Passkey added', action: 'Add a passkey' },
  no_breached_password: { label: 'Password not in any known breach', action: 'Change your master password' },
  single_active_session: { label: 'One active session', action: 'Review your active sessions' },
}

function humanizeFactor(key: string): { label: string; action: string } {
  const known = FACTOR_COPY[key]
  if (known) return known
  const label = key
    .split(/[_\s]+/)
    .filter(Boolean)
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join(' ')
  return { label: label || 'Security factor', action: `Set up ${label || 'this factor'}` }
}

// ── Score label → tone ───────────────────────────────────────────────────────
// Strong labels read amber (the encryption/trust accent); weak labels read in a
// calm warning tone — never alarming red, just honest.

function labelTone(label: string): 'amber' | 'green' | 'neutral' | 'warn' {
  switch ((label || '').toLowerCase()) {
    case 'excellent':
    case 'strong':
      return 'green'
    case 'good':
      return 'amber'
    case 'basic':
      return 'neutral'
    case 'vulnerable':
      return 'warn'
    default:
      return 'neutral'
  }
}

const TONE_COLOR: Record<'amber' | 'green' | 'neutral' | 'warn', string> = {
  amber: T.amberDeep,
  green: T.green,
  neutral: T.ink2,
  warn: WARN,
}

// ── Date formatting (RFC3339 → human, shown in mono) ─────────────────────────

function formatWhen(iso: string | null | undefined): string {
  if (!iso) return 'Unknown'
  const t = Date.parse(iso)
  if (Number.isNaN(t)) return 'Unknown'
  const diffMs = Date.now() - t
  const sec = Math.round(diffMs / 1000)
  if (sec < 0) return 'just now'
  if (sec < 45) return 'just now'
  const min = Math.round(sec / 60)
  if (min < 60) return `${min}m ago`
  const hr = Math.round(min / 60)
  if (hr < 24) return `${hr}h ago`
  const day = Math.round(hr / 24)
  if (day < 30) return `${day}d ago`
  // Older than a month — short absolute date.
  return new Date(t).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' })
}

// ── Small atoms ──────────────────────────────────────────────────────────────

function SectionCard({ icon, title, aside, children }: {
  icon: string
  title: string
  aside?: React.ReactNode
  children: React.ReactNode
}) {
  return (
    <Card style={{ overflow: 'hidden' }}>
      <div style={{ padding: '13px 16px', borderBottom: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', gap: 8 }}>
        <NavIcon name={icon} size={14} color={T.ink2} />
        <div style={{ fontSize: 13.5, fontWeight: 600, color: T.ink }}>{title}</div>
        {aside && <div style={{ marginLeft: 'auto', display: 'flex', alignItems: 'center', gap: 8 }}>{aside}</div>}
      </div>
      {children}
    </Card>
  )
}

function deviceIconFor(kind: string | null | undefined): string {
  const k = (kind || '').toLowerCase()
  if (k.includes('phone') || k.includes('mobile') || k.includes('ios') || k.includes('android')) return 'device'
  if (k.includes('web') || k.includes('browser')) return 'cloud'
  if (k.includes('desktop') || k.includes('windows') || k.includes('mac') || k.includes('linux')) return 'home'
  return 'device'
}

// ── Loading state — the real layout shape, not a spinner ─────────────────────

function LoadingBody() {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 18 }}>
      {/* Score card skeleton */}
      <Card style={{ padding: 18 }}>
        <div style={{ display: 'grid', gridTemplateColumns: 'auto 1fr', gap: 18, alignItems: 'center' }}>
          <Skeleton width={72} height={72} radius={999} />
          <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
            <Skeleton width="42%" height={14} />
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '10px 24px' }}>
              {Array.from({ length: 4 }).map((_, i) => (
                <div key={i} style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
                  <Skeleton width={14} height={14} radius={4} />
                  <Skeleton width={`${70 - i * 6}%`} height={11} />
                </div>
              ))}
            </div>
          </div>
        </div>
      </Card>
      {/* Sessions skeleton */}
      <Card style={{ overflow: 'hidden' }}>
        <div style={{ padding: '13px 16px', borderBottom: `1px solid ${T.line}` }}>
          <Skeleton width={140} height={13} />
        </div>
        {Array.from({ length: 3 }).map((_, i) => (
          <div
            key={i}
            style={{
              padding: '13px 16px',
              borderBottom: i < 2 ? `1px solid ${T.line}` : 'none',
              display: 'grid',
              gridTemplateColumns: '30px 1fr 90px',
              gap: 12,
              alignItems: 'center',
            }}
          >
            <Skeleton width={30} height={30} radius={7} />
            <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
              <Skeleton width={`${50 - i * 6}%`} height={12} />
              <Skeleton width={`${34 - i * 4}%`} height={10} />
            </div>
            <Skeleton width={70} height={11} />
          </div>
        ))}
      </Card>
    </div>
  )
}

// ── Error state ──────────────────────────────────────────────────────────────

function ErrorBody({ unsupported, reason, onRetry }: { unsupported: boolean; reason: string; onRetry: () => void }) {
  if (unsupported) {
    return (
      <Card style={{ padding: 22, display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 8, textAlign: 'center' as const }}>
        <NavIcon name="shield" size={22} color={T.ink4} />
        <div style={{ fontSize: 13.5, fontWeight: 600, color: T.ink }}>Not available in this build</div>
        <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.55, maxWidth: 360 }}>
          Security details aren't wired into this desktop build yet. You can manage sessions and two-factor on beebeeb.io.
        </div>
        <div style={{ marginTop: 4 }}>
          <PrimaryBtn onClick={() => void openUrl(MANAGE_2FA_URL)}>
            <NavIcon name="external" size={12} color={T.paper} /> Open on beebeeb.io
          </PrimaryBtn>
        </div>
      </Card>
    )
  }
  return (
    <Card style={{ padding: 22, display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 8, textAlign: 'center' as const }}>
      <NavIcon name="shield" size={22} color={WARN} />
      <div style={{ fontSize: 13.5, fontWeight: 600, color: T.ink }}>Couldn't load your security details</div>
      <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.55, maxWidth: 420, wordBreak: 'break-word' as const }}>
        {reason}
      </div>
      <div style={{ marginTop: 4 }}>
        <PrimaryBtn onClick={onRetry}>Retry</PrimaryBtn>
      </div>
    </Card>
  )
}

// ── Score card ───────────────────────────────────────────────────────────────

function ScoreCard({ data }: { data: SecurityScore }) {
  const max = data.max ?? 5
  const score = data.score
  const pct = max > 0 ? Math.min(100, Math.max(0, (score / max) * 100)) : 0
  const tone = labelTone(data.label)
  const ringColor = tone === 'warn' ? WARN : tone === 'green' ? T.green : T.amber
  const labelColor = TONE_COLOR[tone]

  // Ring geometry (matches the hifi mockup's stroked progress ring).
  const r = 28
  const c = 2 * Math.PI * r
  const offset = c - (pct / 100) * c

  const factors: SecurityFactor[] = Array.isArray(data.factors) ? data.factors : []
  const satisfiedCount = factors.filter((f) => f.satisfied).length

  return (
    <Card style={{ padding: 18 }}>
      <div style={{ display: 'grid', gridTemplateColumns: 'auto 1fr', gap: 20, alignItems: 'center' }}>
        {/* Score ring */}
        <div style={{ position: 'relative', width: 72, height: 72, flexShrink: 0 }}>
          <svg width="72" height="72" viewBox="0 0 72 72">
            <circle cx="36" cy="36" r={r} fill="none" stroke={T.paper3} strokeWidth="5" />
            <circle
              cx="36"
              cy="36"
              r={r}
              fill="none"
              stroke={ringColor}
              strokeWidth="5"
              strokeDasharray={c}
              strokeDashoffset={offset}
              strokeLinecap="round"
              transform="rotate(-90 36 36)"
            />
          </svg>
          <div
            style={{
              position: 'absolute',
              inset: 0,
              display: 'flex',
              flexDirection: 'column',
              alignItems: 'center',
              justifyContent: 'center',
              fontFamily: T.fontMono,
              lineHeight: 1,
            }}
          >
            <span style={{ fontSize: 17, fontWeight: 600, color: T.ink }}>{score}</span>
            <span style={{ fontSize: 9.5, color: T.ink4, marginTop: 2 }}>/ {max}</span>
          </div>
        </div>

        {/* Label + factor checklist */}
        <div style={{ minWidth: 0 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 4 }}>
            <span style={{ fontSize: 9.5, fontFamily: T.fontMono, textTransform: 'uppercase' as const, letterSpacing: '0.07em', color: T.ink3 }}>
              Security posture
            </span>
            <span style={{ fontSize: 9.5, fontFamily: T.fontMono, color: T.ink4 }}>
              {satisfiedCount}/{factors.length || 0} done
            </span>
          </div>
          <div style={{ fontSize: 16, fontWeight: 700, color: labelColor, letterSpacing: '-0.01em', marginBottom: 14 }}>
            {data.label || 'Unknown'}
          </div>

          {factors.length > 0 ? (
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '10px 24px' }}>
              {factors.map((f) => {
                const { label, action } = humanizeFactor(f.key)
                return (
                  <div key={f.key} style={{ display: 'flex', alignItems: 'flex-start', gap: 9 }}>
                    <span
                      style={{
                        width: 16,
                        height: 16,
                        borderRadius: 5,
                        flexShrink: 0,
                        marginTop: 1,
                        display: 'flex',
                        alignItems: 'center',
                        justifyContent: 'center',
                        background: f.satisfied ? T.amberBg : T.paper2,
                        border: `1px solid ${f.satisfied ? 'oklch(0.86 0.07 90)' : T.line2}`,
                        color: f.satisfied ? T.amberDeep : T.ink4,
                      }}
                    >
                      {f.satisfied ? (
                        <svg width="9" height="9" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                          <path d="M2.5 6.5 L5 9 L9.5 3.5" />
                        </svg>
                      ) : (
                        <span style={{ width: 4, height: 4, borderRadius: '50%', background: T.ink4 }} />
                      )}
                    </span>
                    <div style={{ minWidth: 0, flex: 1 }}>
                      <div style={{ fontSize: 12, lineHeight: 1.35, color: f.satisfied ? T.ink2 : T.ink, fontWeight: f.satisfied ? 400 : 500 }}>
                        {label}
                      </div>
                      {!f.satisfied && (
                        <div style={{ fontSize: 10.5, color: T.ink3, marginTop: 1, lineHeight: 1.35 }}>{action}</div>
                      )}
                    </div>
                  </div>
                )
              })}
            </div>
          ) : (
            <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.5 }}>
              No security factors reported for your account yet.
            </div>
          )}
        </div>
      </div>
    </Card>
  )
}

// ── Two-factor row (honest: links out, never a fake in-app toggle) ───────────

function TwoFactorRow({ enabled }: { enabled: boolean | null }) {
  // enabled === null → we couldn't determine state from the score factors; say so.
  const known = enabled !== null
  return (
    <SectionCard icon="lock" title="Two-factor authentication">
      <div style={{ padding: '14px 16px', display: 'flex', alignItems: 'center', gap: 14 }}>
        <div
          style={{
            width: 30,
            height: 30,
            borderRadius: 8,
            flexShrink: 0,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            background: enabled ? T.amberBg : T.paper2,
            border: `1px solid ${enabled ? 'oklch(0.86 0.07 90)' : T.line}`,
            color: enabled ? T.amberDeep : T.ink3,
          }}
        >
          <NavIcon name="shield" size={14} color={enabled ? T.amberDeep : T.ink3} />
        </div>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ fontSize: 12.5, fontWeight: 500, color: T.ink, display: 'flex', alignItems: 'center', gap: 8 }}>
            Authenticator app (TOTP)
            {known ? (
              enabled ? <Chip tone="amber">On</Chip> : <Chip>Off</Chip>
            ) : (
              <Chip>Unknown</Chip>
            )}
          </div>
          <div style={{ fontSize: 11, color: T.ink3, marginTop: 2, lineHeight: 1.45 }}>
            {enabled
              ? 'A second factor is required when you sign in. Manage or reset it on beebeeb.io.'
              : known
                ? 'Add a second factor so a stolen password alone can’t open your vault. Set it up on beebeeb.io.'
                : 'Manage two-factor authentication on beebeeb.io.'}
          </div>
        </div>
        <button
          onClick={() => void openUrl(MANAGE_2FA_URL)}
          style={{
            display: 'inline-flex',
            alignItems: 'center',
            gap: 6,
            padding: '7px 12px',
            fontSize: 12,
            fontFamily: T.fontSans,
            fontWeight: 500,
            borderRadius: 6,
            border: `1px solid ${T.line2}`,
            background: T.paper,
            color: T.ink2,
            cursor: 'pointer',
            whiteSpace: 'nowrap' as const,
            flexShrink: 0,
          }}
        >
          <NavIcon name="external" size={12} color={T.ink3} />
          {enabled ? 'Manage' : 'Set up'} on beebeeb.io
        </button>
      </div>
    </SectionCard>
  )
}

// ── Session row ──────────────────────────────────────────────────────────────

function SessionRow({
  session,
  isLast,
  pendingConfirm,
  busy,
  onAskRevoke,
  onCancelRevoke,
  onConfirmRevoke,
}: {
  session: AccountSession
  isLast: boolean
  pendingConfirm: boolean
  busy: boolean
  onAskRevoke: () => void
  onCancelRevoke: () => void
  onConfirmRevoke: () => void
}) {
  const icon = deviceIconFor(session.device_kind)
  return (
    <div
      style={{
        padding: '13px 16px',
        borderBottom: isLast ? 'none' : `1px solid ${T.line}`,
        display: 'flex',
        alignItems: 'center',
        gap: 14,
        background: pendingConfirm ? WARN_BG : 'transparent',
      }}
    >
      <div
        style={{
          width: 30,
          height: 30,
          borderRadius: 7,
          flexShrink: 0,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          background: session.is_current ? T.ink : T.paper2,
          border: `1px solid ${session.is_current ? T.ink : T.line}`,
          color: session.is_current ? T.paper : T.ink3,
        }}
      >
        <NavIcon name={icon} size={14} color={session.is_current ? T.paper : T.ink3} />
      </div>

      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' as const }}>
          <span style={{ fontSize: 12.5, fontWeight: 500, color: T.ink }}>
            {session.device_name || 'Unknown device'}
          </span>
          {session.is_current && <Chip tone="neutral">This session</Chip>}
          {session.country_code && (
            <span style={{ fontSize: 10.5, fontFamily: T.fontMono, color: T.ink3, textTransform: 'uppercase' as const, letterSpacing: '0.04em' }}>
              {session.country_code}
            </span>
          )}
        </div>
        <div style={{ fontSize: 10.5, color: T.ink3, marginTop: 2, display: 'flex', gap: 8, alignItems: 'center', flexWrap: 'wrap' as const }}>
          <span style={{ fontFamily: T.fontMono }}>{session.device_kind || 'unknown'}</span>
          <span style={{ color: T.ink4 }}>·</span>
          <span style={{ fontFamily: T.fontMono }}>
            Active {formatWhen(session.last_active_at ?? session.created_at)}
          </span>
        </div>
      </div>

      {!session.is_current && (
        <div style={{ flexShrink: 0, display: 'flex', alignItems: 'center', gap: 8 }}>
          {pendingConfirm ? (
            <>
              <span style={{ fontSize: 11, color: WARN, fontWeight: 500, whiteSpace: 'nowrap' as const }}>
                Sign out this session?
              </span>
              <button
                onClick={onConfirmRevoke}
                disabled={busy}
                style={{
                  padding: '6px 11px',
                  fontSize: 11.5,
                  fontFamily: T.fontSans,
                  fontWeight: 500,
                  borderRadius: 6,
                  border: `1px solid ${WARN}`,
                  background: WARN,
                  color: T.paper,
                  cursor: busy ? 'not-allowed' : 'pointer',
                  opacity: busy ? 0.6 : 1,
                  whiteSpace: 'nowrap' as const,
                }}
              >
                {busy ? 'Revoking…' : 'Revoke'}
              </button>
              <button
                onClick={onCancelRevoke}
                disabled={busy}
                style={{
                  padding: '6px 11px',
                  fontSize: 11.5,
                  fontFamily: T.fontSans,
                  borderRadius: 6,
                  border: `1px solid ${T.line2}`,
                  background: T.paper,
                  color: T.ink2,
                  cursor: busy ? 'not-allowed' : 'pointer',
                  whiteSpace: 'nowrap' as const,
                }}
              >
                Keep
              </button>
            </>
          ) : (
            <button
              onClick={onAskRevoke}
              style={{
                padding: '6px 11px',
                fontSize: 11.5,
                fontFamily: T.fontSans,
                fontWeight: 500,
                borderRadius: 6,
                border: `1px solid ${WARN_LINE}`,
                background: T.paper,
                color: WARN,
                cursor: 'pointer',
                whiteSpace: 'nowrap' as const,
              }}
            >
              Revoke
            </button>
          )}
        </div>
      )}
    </div>
  )
}

// ── Root view ────────────────────────────────────────────────────────────────

type LoadState =
  | { phase: 'loading' }
  | { phase: 'error'; unsupported: boolean; reason: string }
  | { phase: 'loaded'; score: SecurityScore | null; sessions: AccountSession[] }

export default function SecurityView() {
  const [state, setState] = useState<LoadState>({ phase: 'loading' })

  // Per-session revoke UI state.
  const [confirmId, setConfirmId] = useState<string | null>(null)
  const [revokingId, setRevokingId] = useState<string | null>(null)
  const [confirmAllOthers, setConfirmAllOthers] = useState(false)
  const [revokingAll, setRevokingAll] = useState(false)
  // UNTRIAGED (task 1319 notes). Coupled to actionNote via `actionNote && !actionError`, so migrating
  // it changes that interaction — needs its own decision.
  // eslint-disable-next-line beebeeb/no-ad-hoc-error-surface
  const [actionError, setActionError] = useState<string | null>(null)
  const [actionNote, setActionNote] = useState<string | null>(null)

  const load = async () => {
    setState({ phase: 'loading' })
    setActionError(null)
    setActionNote(null)
    setConfirmId(null)
    setConfirmAllOthers(false)

    const [scoreRes, sessionsRes] = await Promise.all([accountSecurityScore(), accountSessionList()])

    // If BOTH endpoints fail, this is a genuine load error — show the error
    // state (and prefer the unsupported signal so we render the calm copy).
    if (!scoreRes.ok && !sessionsRes.ok) {
      const unsupported = scoreRes.unsupported && sessionsRes.unsupported
      const reason = sessionsRes.reason || scoreRes.reason
      setState({ phase: 'error', unsupported, reason })
      return
    }

    setState({
      phase: 'loaded',
      score: scoreRes.ok ? scoreRes.value : null,
      sessions: sessionsRes.ok ? (sessionsRes.value.sessions ?? []) : [],
    })
  }

  useEffect(() => {
    void load()
  }, [])

  const doRevoke = async (id: string) => {
    setRevokingId(id)
    setActionError(null)
    setActionNote(null)
    const r = await accountRevokeSession(id)
    setRevokingId(null)
    setConfirmId(null)
    if (!r.ok) {
      setActionError(
        r.unsupported ? 'Revoking sessions isn’t available in this build.' : r.reason,
      )
      return
    }
    setActionNote('Session signed out.')
    await load()
  }

  const doRevokeAllOthers = async () => {
    setRevokingAll(true)
    setActionError(null)
    setActionNote(null)
    const r = await accountRevokeOtherSessions()
    setRevokingAll(false)
    setConfirmAllOthers(false)
    if (!r.ok) {
      setActionError(
        r.unsupported ? 'Signing out other sessions isn’t available in this build.' : r.reason,
      )
      return
    }
    const n = r.value?.revoked ?? 0
    setActionNote(n === 1 ? 'Signed out 1 other session.' : `Signed out ${n} other sessions.`)
    await load()
  }

  // ── Render ──────────────────────────────────────────────────────────────

  const header = (
    <PageHeader
      title="Security"
      subtitle="What we can see, reset, or recover on your behalf: nothing. Here is what you control."
      aside={
        <Chip tone="amber">
          <NavIcon name="shield" size={10} color={T.amberDeep} /> Zero-knowledge
        </Chip>
      }
    />
  )

  let body: React.ReactNode
  if (state.phase === 'loading') {
    body = <LoadingBody />
  } else if (state.phase === 'error') {
    body = <ErrorBody unsupported={state.unsupported} reason={state.reason} onRetry={() => void load()} />
  } else {
    const sessions = state.sessions
    const otherCount = sessions.filter((s) => !s.is_current).length

    // Derive 2FA state from the score factors (the only honest source we have
    // here). null when no totp/2fa factor is present at all.
    let twoFactor: boolean | null = null
    if (state.score) {
      const f = state.score.factors?.find(
        (x) => x.key === 'two_factor_enabled' || x.key === 'totp_enabled',
      )
      if (f) twoFactor = f.satisfied
    }

    body = (
      <div style={{ display: 'flex', flexDirection: 'column', gap: 18 }}>
        {/* 1) Security score */}
        {state.score ? (
          <ScoreCard data={state.score} />
        ) : (
          <Card style={{ padding: 16, display: 'flex', alignItems: 'center', gap: 10 }}>
            <NavIcon name="shield" size={16} color={T.ink4} />
            <span style={{ fontSize: 12, color: T.ink3, lineHeight: 1.5 }}>
              Your security score isn’t available right now. Sessions are shown below.
            </span>
          </Card>
        )}

        {/* Action feedback (revoke results / errors) */}
        {actionError && (
          <div
            style={{
              padding: '10px 14px',
              borderRadius: 8,
              border: `1px solid ${WARN_LINE}`,
              background: WARN_BG,
              fontSize: 11.5,
              fontFamily: T.fontSans,
              color: WARN,
              wordBreak: 'break-word' as const,
            }}
          >
            {actionError}
          </div>
        )}
        {actionNote && !actionError && (
          <div
            style={{
              padding: '10px 14px',
              borderRadius: 8,
              border: `1px solid ${T.line2}`,
              background: T.paper2,
              fontSize: 11.5,
              color: T.ink2,
            }}
          >
            {actionNote}
          </div>
        )}

        {/* 2) Active sessions */}
        <SectionCard
          icon="device"
          title="Active sessions"
          aside={
            <span style={{ fontSize: 10.5, fontFamily: T.fontMono, color: T.ink4 }}>
              {sessions.length} {sessions.length === 1 ? 'session' : 'sessions'}
            </span>
          }
        >
          {sessions.length === 0 ? (
            <div style={{ padding: '26px 16px', display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 6, textAlign: 'center' as const }}>
              <NavIcon name="device" size={20} color={T.ink4} />
              <div style={{ fontSize: 12.5, fontWeight: 600, color: T.ink }}>No active sessions</div>
              <div style={{ fontSize: 11, color: T.ink3, lineHeight: 1.5, maxWidth: 300 }}>
                Nothing is signed in to your account right now.
              </div>
            </div>
          ) : (
            <>
              {sessions.map((s, i) => (
                <SessionRow
                  key={s.id}
                  session={s}
                  isLast={i === sessions.length - 1}
                  pendingConfirm={confirmId === s.id}
                  busy={revokingId === s.id}
                  onAskRevoke={() => {
                    setActionError(null)
                    setActionNote(null)
                    setConfirmAllOthers(false)
                    setConfirmId(s.id)
                  }}
                  onCancelRevoke={() => setConfirmId(null)}
                  onConfirmRevoke={() => void doRevoke(s.id)}
                />
              ))}

              {/* Sign out all other sessions */}
              {otherCount > 0 && (
                <div style={{ padding: '13px 16px', borderTop: `1px solid ${T.line}`, background: T.paper2, display: 'flex', alignItems: 'center', gap: 12 }}>
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div style={{ fontSize: 12, fontWeight: 500, color: T.ink }}>Sign out everywhere else</div>
                    <div style={{ fontSize: 10.5, color: T.ink3, marginTop: 1, lineHeight: 1.4 }}>
                      Ends every session except this one. Those devices will need to sign in again.
                    </div>
                  </div>
                  {confirmAllOthers ? (
                    <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexShrink: 0 }}>
                      <span style={{ fontSize: 11, color: WARN, fontWeight: 500, whiteSpace: 'nowrap' as const }}>
                        Sign out {otherCount} other {otherCount === 1 ? 'session' : 'sessions'}?
                      </span>
                      <button
                        onClick={() => void doRevokeAllOthers()}
                        disabled={revokingAll}
                        style={{
                          padding: '7px 13px',
                          fontSize: 12,
                          fontFamily: T.fontSans,
                          fontWeight: 500,
                          borderRadius: 6,
                          border: `1px solid ${WARN}`,
                          background: WARN,
                          color: T.paper,
                          cursor: revokingAll ? 'not-allowed' : 'pointer',
                          opacity: revokingAll ? 0.6 : 1,
                          whiteSpace: 'nowrap' as const,
                        }}
                      >
                        {revokingAll ? 'Signing out…' : 'Confirm'}
                      </button>
                      <button
                        onClick={() => setConfirmAllOthers(false)}
                        disabled={revokingAll}
                        style={{
                          padding: '7px 13px',
                          fontSize: 12,
                          fontFamily: T.fontSans,
                          borderRadius: 6,
                          border: `1px solid ${T.line2}`,
                          background: T.paper,
                          color: T.ink2,
                          cursor: revokingAll ? 'not-allowed' : 'pointer',
                          whiteSpace: 'nowrap' as const,
                        }}
                      >
                        Cancel
                      </button>
                    </div>
                  ) : (
                    <PrimaryBtn
                      onClick={() => {
                        setActionError(null)
                        setActionNote(null)
                        setConfirmId(null)
                        setConfirmAllOthers(true)
                      }}
                    >
                      Sign out all other sessions
                    </PrimaryBtn>
                  )}
                </div>
              )}
            </>
          )}
        </SectionCard>

        {/* 3) Two-factor (honest link-out, never a fake toggle) */}
        <TwoFactorRow enabled={twoFactor} />
      </div>
    )
  }

  return (
    <div style={{ overflow: 'auto', padding: '28px 36px', flex: 1 }}>
      {header}
      {body}
    </div>
  )
}
