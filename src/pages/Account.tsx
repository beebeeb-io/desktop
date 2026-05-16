import { useEffect, useState } from 'react'
import {
  command,
  commandUnavailableLabel,
  loadSyncStatus,
  openUrl,
  type SyncStatus,
} from '../desktopApi'

const WEB_APP_URL = 'https://app.beebeeb.io'

export default function Account() {
  const [email, setEmail] = useState<string | null>(null)
  const [status, setStatus] = useState<SyncStatus | null>(null)
  const [autostart, setAutostart] = useState<boolean | null>(null)
  const [busy, setBusy] = useState<string | null>(null)
  const [notice, setNotice] = useState<string | null>(null)

  useEffect(() => {
    void loadSyncStatus().then(setStatus)
    command<string | null>('account_email').then((result) => {
      if (result.ok) setEmail(result.value)
    })
    command<boolean>('autostart_enabled').then((result) => {
      if (result.ok) setAutostart(result.value)
    })
  }, [])

  const runAction = async (name: string, args?: Record<string, unknown>) => {
    setBusy(name)
    setNotice(null)
    const result = await command<void>(name, args)
    setBusy(null)
    if (!result.ok) {
      setNotice(result.unsupported ? commandUnavailableLabel(name) : result.reason)
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
    setNotice(null)
    const result = await command<void>('clear_session')
    setBusy(null)
    if (!result.ok) {
      setNotice(result.unsupported ? commandUnavailableLabel('clear_session') : result.reason)
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
    else setNotice(result.unsupported ? commandUnavailableLabel('toggle_autostart') : result.reason)
  }

  const diagnostics = async () => {
    if (await runAction('export_diagnostics')) {
      setNotice('Diagnostics export started.')
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

      {notice && <div className="notice" style={{ marginBottom: 14 }}>{notice}</div>}

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
            <button className="button primary" disabled={busy === 'unlock_vault'} onClick={() => void unlock()}>
              {busy === 'unlock_vault' ? 'Unlocking…' : 'Unlock'}
            </button>
            <button className="button" disabled={busy === 'lock_vault'} onClick={() => void lock()}>
              {busy === 'lock_vault' ? 'Locking…' : 'Lock'}
            </button>
          </div>
        </div>
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
