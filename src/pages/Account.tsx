import { useEffect, useState } from 'react'
import { useToast } from '../windows/ui'
import {
  accountSubscription,
  BILLING_URL,
  command,
  commandUnavailableLabel,
  formatBytes,
  loadSyncStatus,
  openUrl,
  regionCityFromCode,
  type Subscription,
  type SyncStatus,
} from '../desktopApi'
import { planRenewalCopy, planStatusTone, quotaPercent, titleCasePlan } from '../planPresentation'

const WEB_APP_URL = 'https://app.beebeeb.io'

// Task 1546 finding 1: the macOS Account page had NO plan/trial/quota UI
// anywhere — accountSubscription() was already wired but called only from
// the Windows views. This mirrors AccountView.tsx's PlanCard content in the
// macOS page's own panel/row CSS idiom (not its inline-style Windows one).
type PlanState =
  | { phase: 'loading' }
  | { phase: 'unsupported' }
  | { phase: 'error'; reason: string }
  | { phase: 'loaded'; sub: Subscription }

export default function Account() {
  const { showToast } = useToast()
  const [email, setEmail] = useState<string | null>(null)
  const [status, setStatus] = useState<SyncStatus | null>(null)
  const [autostart, setAutostart] = useState<boolean | null>(null)
  const [busy, setBusy] = useState<string | null>(null)
  const [plan, setPlan] = useState<PlanState>({ phase: 'loading' })

  // Poll sync_status so the component reflects auto-unlock state that
  // occurs on startup — the vault may unlock a few seconds after mount.
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
    command<boolean>('autostart_enabled').then((result) => {
      if (result.ok) setAutostart(result.value)
    })
  }, [])

  // Re-fetch the account email whenever we become logged-in/unlocked so the
  // field populates correctly after an auto-unlock at startup (the IPC
  // account_email returns null until auth_email is populated by the Rust side).
  const loggedInKey = status?.logged_in ? (status.vault_unlocked ? 'unlocked' : 'locked') : 'out'
  useEffect(() => {
    if (!status?.logged_in) return
    command<string | null>('account_email').then((result) => {
      if (result.ok && result.value) setEmail(result.value)
    })
  // eslint-disable-next-line react-hooks/exhaustive-deps -- intentionally keyed on the derived loggedInKey, not the full status object; re-fetch should fire only when the auth-state category (out/locked/unlocked) changes, not on every status poll tick
  }, [loggedInKey])

  // Plan & subscription — same loggedInKey-keyed re-fetch pattern as the
  // account_email effect above, so it re-runs once per sign-in, not once per
  // 5s status poll tick.
  useEffect(() => {
    if (!status?.logged_in) {
      setPlan({ phase: 'loading' })
      return
    }
    let cancelled = false
    setPlan({ phase: 'loading' })
    accountSubscription().then((result) => {
      if (cancelled) return
      if (result.ok) setPlan({ phase: 'loaded', sub: result.value })
      else setPlan(result.unsupported ? { phase: 'unsupported' } : { phase: 'error', reason: result.reason })
    })
    return () => {
      cancelled = true
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps -- intentionally keyed on the derived loggedInKey, matching the account_email effect above
  }, [loggedInKey])

  const runAction = async (name: string, args?: Record<string, unknown>) => {
    setBusy(name)
    const result = await command<void>(name, args)
    setBusy(null)
    if (!result.ok) {
      showToast({
        variant: 'error',
        title: 'That didn’t work',
        message: result.unsupported ? commandUnavailableLabel(name) : result.reason,
      })
      return false
    }
    return true
  }

  const lock = async () => {
    if (await runAction('lock_vault')) {
      const next = await loadSyncStatus()
      setStatus(next)
    }
  }

  const unlock = async () => {
    if (await runAction('unlock_vault')) {
      const next = await loadSyncStatus()
      setStatus(next)
    }
  }

  const signOut = async () => {
    setBusy('clear_session')
    const result = await command<void>('clear_session')
    setBusy(null)
    if (!result.ok) {
      showToast({
        variant: 'error',
        title: 'Couldn’t sign out',
        message: result.unsupported ? commandUnavailableLabel('clear_session') : result.reason,
      })
      return
    }
    setEmail(null)
    setStatus({ logged_in: false, engine: 'stopped', sync_root: null, syncing: 0, cloud_only: 0, conflicts: 0 })
  }

  const toggleStartAtLogin = async () => {
    setBusy('toggle_autostart')
    const result = await command<boolean>('toggle_autostart')
    setBusy(null)
    if (result.ok) setAutostart(result.value)
    else showToast({
      variant: 'error',
      title: 'Couldn’t change launch setting',
      message: result.unsupported ? commandUnavailableLabel('toggle_autostart') : result.reason,
    })
  }

  const diagnostics = async () => {
    if (await runAction('export_diagnostics')) {
      showToast({
        variant: 'success',
        title: 'Diagnostics export started',
        message: 'The bundle is being written to your sync folder.',
      })
    }
  }

  const loggedIn = status?.logged_in ?? false
  const unlocked = loggedIn && (status?.vault_unlocked ?? status?.engine === 'running')

  return (
    <section className="page">
      <div className="page-header">
        <div>
          <h1 className="page-title">Account & security</h1>
          <p className="page-copy">
            Session token, unlock state, diagnostics, sign-out, and launch behavior. Locking clears
            file-content access while leaving the Finder location visible.
          </p>
        </div>
        <span className="status-pill">
          <span className={`dot ${unlocked ? 'ok' : loggedIn ? 'warn' : ''}`} />
          {unlocked ? 'Unlocked' : loggedIn ? 'Locked or paused' : 'Signed out'}
        </span>
      </div>


      <div className="grid two">
        <div className="panel">
          <h2 className="section-title">Identity</h2>
          <div className="row">
            <div>
              <div className="row-title">Signed in as</div>
              <div className="row-detail">{email ?? (loggedIn ? 'Signed in' : 'Not signed in')}</div>
            </div>
            <button className="button" onClick={() => void openUrl(WEB_APP_URL)}>
              Open web app
            </button>
          </div>
          <div className="row">
            <div>
              <div className="row-title">Session token</div>
              <div className="row-detail">{loggedIn ? 'Stored by native session layer' : 'Missing'}</div>
            </div>
          </div>
        </div>

        <div className="panel">
          <h2 className="section-title">Vault lock</h2>
          <div className="row">
            <div>
              <div className="row-title">Master key lifetime</div>
              <div className="row-detail">
                {unlocked ? 'Unwrapped in memory' : 'Unavailable to file operations'}
              </div>
            </div>
          </div>
          <div className="button-row">
            <button className="button amber" disabled={busy === 'unlock_vault'} onClick={() => void unlock()}>
              {busy === 'unlock_vault' ? 'Unlocking…' : 'Unlock'}
            </button>
            <button className="button" disabled={busy === 'lock_vault'} onClick={() => void lock()}>
              {busy === 'lock_vault' ? 'Locking…' : 'Lock'}
            </button>
          </div>
        </div>
      </div>

      <div className="panel" style={{ marginTop: 14 }}>
        <h2 className="section-title">Plan & subscription</h2>
        {!loggedIn ? (
          <div className="empty-state">Sign in to see your plan.</div>
        ) : plan.phase === 'loading' ? (
          <div className="empty-state">Loading plan…</div>
        ) : plan.phase === 'unsupported' ? (
          <div className="notice">Plan details aren’t available in this build.</div>
        ) : plan.phase === 'error' ? (
          <div className="notice">{plan.reason}</div>
        ) : (
          <>
            <div className="row">
              <div>
                <div className="row-title">
                  {titleCasePlan(plan.sub.plan)}
                  {plan.sub.billing_cycle ? ` · ${plan.sub.billing_cycle}` : ''}
                </div>
                <div className="row-detail">
                  <span className="status-pill" style={{ marginRight: 8 }}>
                    <span className={`dot ${planStatusTone(plan.sub.status) === 'green' ? 'ok' : ''}`} />
                    {titleCasePlan(plan.sub.status)}
                  </span>
                  {regionCityFromCode(plan.sub.region)}
                </div>
              </div>
              <button className="button amber" onClick={() => void openUrl(BILLING_URL)}>
                {plan.sub.plan.toLowerCase() === 'free' ? 'Upgrade' : 'Manage plan'}
              </button>
            </div>
            {plan.sub.quota_bytes > 0 && (
              <div className="row">
                <div style={{ width: '100%' }}>
                  <div className="row-title">
                    {formatBytes(plan.sub.used_bytes)} of {formatBytes(plan.sub.quota_bytes)}
                  </div>
                  <div
                    className="progress-track"
                    style={{ marginTop: 8 }}
                    aria-label={`Storage ${Math.round(quotaPercent(plan.sub.used_bytes, plan.sub.quota_bytes))}% full`}
                  >
                    <div
                      className="progress-fill"
                      style={{ width: `${quotaPercent(plan.sub.used_bytes, plan.sub.quota_bytes)}%` }}
                    />
                  </div>
                </div>
              </div>
            )}
            <div className="row">
              <div>
                <div className="row-title">Renews</div>
                <div className="row-detail">{planRenewalCopy(plan.sub.current_period_end)}</div>
              </div>
            </div>
          </>
        )}
      </div>

      <div className="grid two" style={{ marginTop: 14 }}>
        <div className="panel">
          <h2 className="section-title">Diagnostics</h2>
          <div className="row">
            <div>
              <div className="row-title">Support bundle</div>
              <div className="row-detail">Export logs and daemon state without secrets or plaintext names.</div>
            </div>
            <button className="button" disabled={busy === 'export_diagnostics'} onClick={() => void diagnostics()}>
              Export
            </button>
          </div>
          <div className="row">
            <div>
              <div className="row-title">Start at login</div>
              <div className="row-detail">
                {autostart === null ? 'Unknown' : autostart ? 'Enabled' : 'Disabled'}
              </div>
            </div>
            <button
              className="button"
              disabled={busy === 'toggle_autostart'}
              onClick={() => void toggleStartAtLogin()}
            >
              Toggle
            </button>
          </div>
        </div>

        <div className="panel">
          <h2 className="section-title">Sign out</h2>
          <p className="page-copy" style={{ margin: '0 0 14px' }}>
            Signing out stops sync and removes the session. Future native wiring should also clear
            cached plaintext and wrapped-key material.
          </p>
          <button
            className="button danger"
            disabled={!loggedIn || busy === 'clear_session'}
            onClick={() => void signOut()}
          >
            {busy === 'clear_session' ? 'Signing out…' : 'Sign out'}
          </button>
        </div>
      </div>
    </section>
  )
}
