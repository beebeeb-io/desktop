/**
 * AccountView — the native Account page for the Windows app (PKG-DATA bind).
 *
 * Replaces the scaffolded `account` placeholder in WindowsApp.tsx. Two cards:
 *   1. Profile  — email + verified/unverified state, member-since, role, and a
 *                 calm warning row when the account is frozen / pending deletion.
 *   2. Plan     — plan + billing cycle + status, region (city only, e.g. Falkenstein),
 *                 quota usage, renewal date, and seats when a multi-seat plan.
 * plus an honest footer: password & billing are managed on the web, opened via
 * the real `openUrl` IPC — no faked in-app mutation surfaces.
 *
 * Self-fetches on mount via accountProfile() + accountSubscription(). Handles all
 * four states (loading skeletons in the real shape, unsupported, error+retry,
 * loaded). Grounded on design/hifi/hifi-settings.jsx (Profile section) adapted to
 * the WindowsApp.tsx card idiom. Machine values (email, ids, dates, byte sizes,
 * percentages) render in JetBrains Mono; human copy in Inter.
 */

import { useEffect, useState } from 'react'
import {
  accountProfile,
  accountSubscription,
  clearSession,
  formatBytes,
  openUrl,
  regionCityFromCode,
  type AccountProfile,
  type Subscription,
} from '../../desktopApi'
import { T, Card, PageHeader, Chip, Skeleton, NavIcon, PrimaryBtn } from '../ui'

const ACCOUNT_URL = 'https://app.beebeeb.io/settings/account'
const BILLING_URL = 'https://app.beebeeb.io/billing'
const DANGER = 'oklch(0.5 0.18 25)'

// ── Date helpers ────────────────────────────────────────────────────────────
// Timestamps arrive as RFC3339 strings. Parse defensively — a malformed or null
// value must never throw; it degrades to a dash.

function parseDate(value: string | null | undefined): Date | null {
  if (!value) return null
  const d = new Date(value)
  return Number.isNaN(d.getTime()) ? null : d
}

/** Short absolute date, e.g. "14 Jun 2026". */
function shortDate(value: string | null | undefined): string {
  const d = parseDate(value)
  if (!d) return '—'
  return d.toLocaleDateString('en-GB', { day: 'numeric', month: 'short', year: 'numeric' })
}

/** Relative phrasing for a future renewal, falling back to the absolute date. */
function relativeFuture(value: string | null | undefined): string | null {
  const d = parseDate(value)
  if (!d) return null
  const ms = d.getTime() - Date.now()
  const days = Math.round(ms / 86_400_000)
  if (days < 0) return null
  if (days === 0) return 'today'
  if (days === 1) return 'tomorrow'
  if (days < 45) return `in ${days} days`
  const months = Math.round(days / 30)
  if (months < 12) return `in ${months} month${months === 1 ? '' : 's'}`
  return null
}

// ── Presentational helpers ──────────────────────────────────────────────────

function CardHeader({ icon, title }: { icon: string; title: string }) {
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '14px 18px', borderBottom: `1px solid ${T.line}` }}>
      <div style={{ width: 28, height: 28, borderRadius: 7, background: T.paper2, border: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
        <NavIcon name={icon} size={14} color={T.ink2} />
      </div>
      <span style={{ fontSize: 13, fontWeight: 600, color: T.ink, letterSpacing: '-0.01em' }}>{title}</span>
    </div>
  )
}

// A label/value row. `mono` puts the value in JetBrains Mono (the rule for any
// machine value: emails, dates, byte sizes, ids).
function Row({ label, children, mono = false, last = false }: { label: string; children: React.ReactNode; mono?: boolean; last?: boolean }) {
  return (
    <div style={{
      display: 'grid',
      gridTemplateColumns: '140px 1fr',
      gap: 16,
      alignItems: 'center',
      padding: '13px 18px',
      borderBottom: last ? 'none' : `1px solid ${T.line}`,
    }}>
      <span style={{ fontSize: 11.5, color: T.ink3, fontFamily: T.fontSans }}>{label}</span>
      <div style={{
        fontSize: mono ? 12 : 12.5,
        fontFamily: mono ? T.fontMono : T.fontSans,
        color: T.ink,
        display: 'flex',
        alignItems: 'center',
        gap: 8,
        flexWrap: 'wrap' as const,
        minWidth: 0,
      }}>
        {children}
      </div>
    </div>
  )
}

function Dash() {
  return <span style={{ fontFamily: T.fontMono, color: T.ink4 }}>—</span>
}

// Maps a free-form subscription status to a chip tone. "active" reads as the
// good state (amber — it's the live, paid state); anything else stays neutral so
// amber never becomes decorative.
function statusTone(status: string): 'green' | 'neutral' {
  const s = status.toLowerCase()
  if (s === 'active' || s === 'trialing') return 'green'
  return 'neutral'
}

function titleCase(value: string): string {
  if (!value) return value
  return value.charAt(0).toUpperCase() + value.slice(1)
}

// ── States ──────────────────────────────────────────────────────────────────

function LoadingState() {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
      {[0, 1].map((card) => (
        <Card key={card}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '14px 18px', borderBottom: `1px solid ${T.line}` }}>
            <Skeleton width={28} height={28} radius={7} />
            <Skeleton width={card === 0 ? 90 : 110} height={13} />
          </div>
          <div style={{ display: 'flex', flexDirection: 'column' }}>
            {Array.from({ length: card === 0 ? 3 : 4 }).map((_, i) => (
              <div key={i} style={{ display: 'grid', gridTemplateColumns: '140px 1fr', gap: 16, alignItems: 'center', padding: '13px 18px', borderBottom: i < (card === 0 ? 2 : 3) ? `1px solid ${T.line}` : 'none' }}>
                <Skeleton width={`${70 - i * 6}%`} height={11} />
                <Skeleton width={`${55 - i * 7}%`} height={12} />
              </div>
            ))}
          </div>
        </Card>
      ))}
    </div>
  )
}

function UnsupportedState() {
  return (
    <Card style={{ padding: 28 }}>
      <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', textAlign: 'center' as const, gap: 8 }}>
        <NavIcon name="user" size={22} color={T.ink4} />
        <div style={{ fontSize: 13.5, fontWeight: 600, color: T.ink, marginTop: 4 }}>Not available in this build</div>
        <div style={{ fontSize: 12, color: T.ink3, lineHeight: 1.6, maxWidth: 340 }}>
          Account details aren't wired into this desktop build yet. They'll appear here once the build includes them.
        </div>
      </div>
    </Card>
  )
}

function ErrorState({ reason, onRetry }: { reason: string; onRetry: () => void }) {
  return (
    <Card style={{ padding: 28 }}>
      <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', textAlign: 'center' as const, gap: 10 }}>
        <NavIcon name="shield" size={22} color={DANGER} />
        <div style={{ fontSize: 13.5, fontWeight: 600, color: T.ink, marginTop: 2 }}>Couldn't load your account</div>
        <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.6, maxWidth: 380, wordBreak: 'break-word' as const }}>
          {reason}
        </div>
        <div style={{ marginTop: 6 }}>
          <PrimaryBtn onClick={onRetry}>Retry</PrimaryBtn>
        </div>
      </div>
    </Card>
  )
}

// ── Loaded content ──────────────────────────────────────────────────────────

function FrozenRow({ frozenAt }: { frozenAt: string }) {
  return (
    <div style={{
      display: 'flex',
      alignItems: 'flex-start',
      gap: 10,
      padding: '12px 18px',
      borderTop: `1px solid ${T.line}`,
      background: 'oklch(0.97 0.04 30)',
    }}>
      <div style={{ marginTop: 1, flexShrink: 0 }}>
        <NavIcon name="lock" size={14} color={DANGER} />
      </div>
      <div style={{ minWidth: 0 }}>
        <div style={{ fontSize: 12, fontWeight: 600, color: T.ink }}>This account is frozen</div>
        <div style={{ fontSize: 11.5, color: T.ink2, lineHeight: 1.6, marginTop: 2 }}>
          Frozen since <span style={{ fontFamily: T.fontMono }}>{shortDate(frozenAt)}</span>. Sync is paused. Reach support at beebeeb.io to restore access.
        </div>
      </div>
    </div>
  )
}

function ProfileCard({ profile }: { profile: AccountProfile }) {
  const role = profile.role?.toLowerCase()
  const isPrivileged = role === 'admin' || role === 'superadmin'

  return (
    <Card>
      <CardHeader icon="user" title="Profile" />

      <Row label="Email">
        <span style={{ fontFamily: T.fontMono, fontSize: 12, color: T.ink, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' as const, maxWidth: '100%' }}>
          {profile.email}
        </span>
        {profile.email_verified
          ? <Chip tone="amber">Verified</Chip>
          : <Chip tone="neutral">Unverified</Chip>}
        {isPrivileged && <Chip tone="neutral">{titleCase(profile.role ?? '')}</Chip>}
      </Row>

      <Row label="Member since" mono last={!profile.frozen_at}>
        {shortDate(profile.created_at)}
      </Row>

      {profile.frozen_at && <FrozenRow frozenAt={profile.frozen_at} />}
    </Card>
  )
}

function QuotaBar({ used, quota }: { used: number; quota: number }) {
  const pct = quota > 0 ? Math.min(100, Math.max(0, (used / quota) * 100)) : 0
  return (
    <div style={{ width: '100%' }}>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline', marginBottom: 6 }}>
        <span style={{ fontFamily: T.fontMono, fontSize: 12, color: T.ink }}>
          {formatBytes(used)} <span style={{ color: T.ink4 }}>/</span> {formatBytes(quota)}
        </span>
        <span style={{ fontFamily: T.fontMono, fontSize: 11, color: T.ink3 }}>{pct.toFixed(pct >= 10 ? 0 : 1)}%</span>
      </div>
      <div className="progress-track">
        <div className="progress-fill" style={{ width: `${pct}%` }} />
      </div>
    </div>
  )
}

function PlanCard({ sub }: { sub: Subscription }) {
  const renewsRel = relativeFuture(sub.current_period_end)
  const renewsAbs = sub.current_period_end ? shortDate(sub.current_period_end) : null
  const hasSeats = sub.seats > 1

  return (
    <Card>
      <CardHeader icon="cloud" title="Plan & subscription" />

      <Row label="Plan">
        <span style={{ fontWeight: 600, fontSize: 13, color: T.ink }}>{titleCase(sub.plan)}</span>
        {sub.billing_cycle && (
          <span style={{ fontFamily: T.fontMono, fontSize: 11, color: T.ink3 }}>{sub.billing_cycle}</span>
        )}
        {sub.status && <Chip tone={statusTone(sub.status)}>{titleCase(sub.status)}</Chip>}
      </Row>

      <Row label="Storage">
        {sub.quota_bytes > 0
          ? <QuotaBar used={sub.used_bytes} quota={sub.quota_bytes} />
          : <Dash />}
      </Row>

      <Row label="Region">
        <NavIcon name="shield" size={12} color={T.ink3} />
        <span style={{ fontFamily: T.fontMono, fontSize: 11.5, color: T.ink2, letterSpacing: '0.02em' }}>{regionCityFromCode(sub.region)}</span>
      </Row>

      <Row label="Renews" mono last={!hasSeats}>
        {renewsAbs
          ? <span>{renewsAbs}{renewsRel ? <span style={{ color: T.ink3 }}> · {renewsRel}</span> : null}</span>
          : <span style={{ color: T.ink3, fontFamily: T.fontSans, fontSize: 12 }}>No renewal — this plan doesn't expire</span>}
      </Row>

      {hasSeats && (
        <Row label="Seats" mono last>
          {sub.seats}
        </Row>
      )}
    </Card>
  )
}

// ── Disconnect section ──────────────────────────────────────────────────────
//
// Inline confirm step — no window.confirm(). Three states:
//   idle       → shows the "Disconnect" button
//   confirming → shows the copy + Confirm / Cancel pair
//   busy       → shows the spinner copy while the IPC call is in flight
//   error      → shows the error reason, lets the user retry or cancel

type DisconnectPhase = 'idle' | 'confirming' | 'busy' | 'error'

function DisconnectSection() {
  const [phase, setPhase] = useState<DisconnectPhase>('idle')
  // UNTRIAGED (task 1319 notes). Rendered inside a confirmation flow, so it may well be form-blocking,
  // but that was not verified in this task.
  // eslint-disable-next-line beebeeb/no-ad-hoc-error-surface
  const [errorMsg, setErrorMsg] = useState<string>('')

  const handleConfirm = async () => {
    setPhase('busy')
    const result = await clearSession()
    if (!result.ok) {
      setErrorMsg(result.reason)
      setPhase('error')
    }
    // On success: the root sync_status poll will detect logged_in: false and
    // unmount this whole view — no explicit navigation needed here.
  }

  const reset = () => {
    setErrorMsg('')
    setPhase('idle')
  }

  return (
    <div style={{
      borderRadius: 10,
      border: `1px solid ${T.line}`,
      background: T.paper,
      overflow: 'hidden',
    }}>
      {/* Header row */}
      <div style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'space-between',
        padding: '14px 18px',
        borderBottom: phase !== 'idle' ? `1px solid ${T.line}` : 'none',
      }}>
        <div>
          <div style={{ fontSize: 13, fontWeight: 600, color: T.ink, marginBottom: 2 }}>Disconnect this device</div>
          <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.6 }}>
            Signs out this PC. Your files stay safe in your vault.
          </div>
        </div>
        {phase === 'idle' && (
          <button
            onClick={() => setPhase('confirming')}
            style={{
              display: 'inline-flex',
              alignItems: 'center',
              padding: '7px 14px',
              fontSize: 12,
              fontFamily: T.fontSans,
              fontWeight: 500,
              borderRadius: 6,
              border: `1px solid ${T.line2}`,
              background: T.paper2,
              color: T.ink,
              cursor: 'pointer',
              whiteSpace: 'nowrap' as const,
              flexShrink: 0,
            }}
          >
            Disconnect
          </button>
        )}
      </div>

      {/* Confirm step */}
      {phase === 'confirming' && (
        <div style={{ padding: '14px 18px', display: 'flex', flexDirection: 'column', gap: 12 }}>
          <div style={{ fontSize: 12, color: T.ink2, lineHeight: 1.6 }}>
            <strong style={{ color: T.ink }}>Disconnect this account from this PC</strong> — your files stay safe in your vault. Sync will stop and you'll need to sign in again to resume.
          </div>
          <div style={{ display: 'flex', gap: 8 }}>
            <button
              onClick={() => void handleConfirm()}
              style={{
                display: 'inline-flex',
                alignItems: 'center',
                padding: '7px 14px',
                fontSize: 12,
                fontFamily: T.fontSans,
                fontWeight: 500,
                borderRadius: 6,
                border: `1px solid ${DANGER}`,
                background: 'oklch(0.97 0.03 25)',
                color: DANGER,
                cursor: 'pointer',
              }}
            >
              Disconnect
            </button>
            <button
              onClick={reset}
              style={{
                display: 'inline-flex',
                alignItems: 'center',
                padding: '7px 14px',
                fontSize: 12,
                fontFamily: T.fontSans,
                fontWeight: 500,
                borderRadius: 6,
                border: `1px solid ${T.line2}`,
                background: T.paper,
                color: T.ink2,
                cursor: 'pointer',
              }}
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      {/* Busy step */}
      {phase === 'busy' && (
        <div style={{ padding: '14px 18px', fontSize: 12, color: T.ink3 }}>
          Disconnecting…
        </div>
      )}

      {/* Error step */}
      {phase === 'error' && (
        <div style={{ padding: '14px 18px', display: 'flex', flexDirection: 'column', gap: 10 }}>
          <div style={{ fontSize: 11.5, fontFamily: T.fontMono, color: DANGER, lineHeight: 1.6, wordBreak: 'break-word' as const }}>
            {errorMsg}
          </div>
          <div style={{ display: 'flex', gap: 8 }}>
            <button
              onClick={() => void handleConfirm()}
              style={{
                display: 'inline-flex',
                alignItems: 'center',
                padding: '7px 14px',
                fontSize: 12,
                fontFamily: T.fontSans,
                fontWeight: 500,
                borderRadius: 6,
                border: `1px solid ${DANGER}`,
                background: 'oklch(0.97 0.03 25)',
                color: DANGER,
                cursor: 'pointer',
              }}
            >
              Retry
            </button>
            <button
              onClick={reset}
              style={{
                display: 'inline-flex',
                alignItems: 'center',
                padding: '7px 14px',
                fontSize: 12,
                fontFamily: T.fontSans,
                fontWeight: 500,
                borderRadius: 6,
                border: `1px solid ${T.line2}`,
                background: T.paper,
                color: T.ink2,
                cursor: 'pointer',
              }}
            >
              Cancel
            </button>
          </div>
        </div>
      )}
    </div>
  )
}

function Footer() {
  return (
    <div style={{
      display: 'flex',
      alignItems: 'center',
      gap: 12,
      padding: '14px 18px',
      borderRadius: 10,
      border: `1px solid ${T.line}`,
      background: T.paper2,
    }}>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ fontSize: 12, fontWeight: 600, color: T.ink, marginBottom: 2 }}>Password & billing live on the web</div>
        <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.6 }}>
          Changing your password, email, or plan happens in your browser at beebeeb.io — the desktop app never holds those controls.
        </div>
      </div>
      <button
        onClick={() => void openUrl(ACCOUNT_URL)}
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
          color: T.ink,
          cursor: 'pointer',
          whiteSpace: 'nowrap' as const,
          flexShrink: 0,
        }}
      >
        <NavIcon name="external" size={12} color={T.ink2} /> Manage account
      </button>
      <button
        onClick={() => void openUrl(BILLING_URL)}
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
          color: T.ink,
          cursor: 'pointer',
          whiteSpace: 'nowrap' as const,
          flexShrink: 0,
        }}
      >
        <NavIcon name="external" size={12} color={T.ink2} /> Billing
      </button>
    </div>
  )
}

// ── Root ────────────────────────────────────────────────────────────────────

type LoadState =
  | { phase: 'loading' }
  | { phase: 'unsupported' }
  | { phase: 'error'; reason: string }
  | { phase: 'loaded'; profile: AccountProfile; subscription: Subscription | null; subError: string | null }

export default function AccountView() {
  const [state, setState] = useState<LoadState>({ phase: 'loading' })

  const load = () => {
    setState({ phase: 'loading' })
    let cancelled = false
    void (async () => {
      const [p, s] = await Promise.all([accountProfile(), accountSubscription()])
      if (cancelled) return

      // The profile is the page's spine — if it's gone, the page can't render.
      if (!p.ok) {
        setState(p.unsupported ? { phase: 'unsupported' } : { phase: 'error', reason: p.reason })
        return
      }

      // The subscription is secondary: a free-plan account or an unavailable
      // billing endpoint shouldn't blank the profile. Degrade the plan card
      // instead of failing the whole page.
      setState({
        phase: 'loaded',
        profile: p.value,
        subscription: s.ok ? s.value : null,
        subError: s.ok ? null : (s.unsupported ? null : s.reason),
      })
    })()
    return () => { cancelled = true }
  }

  useEffect(load, [])

  let body: React.ReactNode
  if (state.phase === 'loading') {
    body = <LoadingState />
  } else if (state.phase === 'unsupported') {
    body = <UnsupportedState />
  } else if (state.phase === 'error') {
    body = <ErrorState reason={state.reason} onRetry={load} />
  } else {
    body = (
      <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
        <ProfileCard profile={state.profile} />
        {state.subscription
          ? <PlanCard sub={state.subscription} />
          : state.subError
            ? <PlanCardError reason={state.subError} />
            : <PlanUnavailable />}
        <DisconnectSection />
        <Footer />
      </div>
    )
  }

  return (
    <div style={{ overflow: 'auto', padding: '28px 36px', flex: 1 }}>
      <PageHeader
        title="Account"
        subtitle="Your profile and subscription. Files stay encrypted on this PC — your account record holds none of your keys."
        aside={
          <Chip tone="green">
            <span style={{ width: 6, height: 6, borderRadius: '50%', background: T.green, display: 'inline-block' }} /> Zero-knowledge
          </Chip>
        }
      />
      {body}
    </div>
  )
}

// Plan card couldn't load but the profile did — keep the page intact, name the
// gap honestly inside the plan slot rather than failing the whole view.
function PlanCardError({ reason }: { reason: string }) {
  return (
    <Card>
      <CardHeader icon="cloud" title="Plan & subscription" />
      <div style={{ padding: '16px 18px', display: 'flex', flexDirection: 'column', gap: 4 }}>
        <div style={{ fontSize: 12, color: T.ink2 }}>Couldn't load your subscription</div>
        <div style={{ fontSize: 11, fontFamily: T.fontMono, color: T.ink3, lineHeight: 1.6, wordBreak: 'break-word' as const }}>{reason}</div>
      </div>
    </Card>
  )
}

function PlanUnavailable() {
  return (
    <Card>
      <CardHeader icon="cloud" title="Plan & subscription" />
      <div style={{ padding: '16px 18px', fontSize: 12, color: T.ink3, lineHeight: 1.6 }}>
        Subscription details aren't available in this build.
      </div>
    </Card>
  )
}
