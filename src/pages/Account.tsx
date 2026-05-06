/**
 * Account settings.
 *
 * Shows the current sign-in state plus three actions:
 *   1. "Open web app" — pops the Beebeeb web client in the system
 *      browser so the user can manage files / billing / sharing.
 *   2. "Sign out" — calls `clear_session` IPC, which also stops the
 *      engine runner. The web client is responsible for clearing its
 *      own localStorage afterwards (it listens for the
 *      `session-cleared` event).
 *
 * Signed-in email is read via `account_email` IPC when available; if
 * the command isn't registered yet (rust-engineer's pending work) the
 * page falls back to a generic "Signed in" / "Not signed in" label
 * derived from `sync_status.logged_in`.
 *
 * See docs/superpowers/plans/2026-05-07-desktop-sync-client.md (Task 9).
 */

import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'

const WEB_APP_URL = 'https://app.beebeeb.io'

export default function Account() {
  const [email, setEmail] = useState<string | null>(null)
  const [loggedIn, setLoggedIn] = useState<boolean | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    invoke<{ logged_in: boolean }>('sync_status')
      .then((s) => setLoggedIn(s.logged_in))
      .catch((e: unknown) => {
        console.warn('sync_status failed:', e)
        setLoggedIn(false)
      })

    // Email IPC is best-effort — rust-engineer hasn't shipped it yet.
    invoke<string | null>('account_email')
      .then(setEmail)
      .catch(() => {
        // Silently swallow — falling back to the generic label is fine.
      })
  }, [])

  const openWebApp = async () => {
    try {
      // Preferred path: tauri-plugin-opener handles the cross-platform
      // shell-open dance + respects the user's default browser.
      await invoke('plugin:opener|open_url', { url: WEB_APP_URL })
    } catch (e) {
      console.warn('plugin:opener|open_url failed, falling back:', e)
      // Fallback — works in dev (vite serves the page in a regular
      // WebView) but might be blocked in a hardened production bundle.
      window.open(WEB_APP_URL, '_blank', 'noopener,noreferrer')
    }
  }

  const signOut = async () => {
    setBusy(true)
    setError(null)
    try {
      await invoke('clear_session')
      setLoggedIn(false)
      setEmail(null)
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e)
      setError(`Sign-out failed: ${msg}`)
    } finally {
      setBusy(false)
    }
  }

  const stateLabel =
    loggedIn === null
      ? 'Loading…'
      : loggedIn
        ? (email ?? 'Signed in')
        : 'Not signed in'

  return (
    <div>
      <h2 style={{ fontSize: 18, fontWeight: 600, marginBottom: 16 }}>Account</h2>

      <div
        style={{
          background: '#f3f4f6',
          borderRadius: 8,
          padding: 12,
          fontSize: 13,
          marginBottom: 20,
        }}
      >
        <div style={{ color: '#6b7280', fontSize: 11, marginBottom: 4 }}>
          Signed in as
        </div>
        <div style={{ fontWeight: 500 }}>{stateLabel}</div>
      </div>

      {error && (
        <div
          style={{
            background: '#fee2e2',
            color: '#991b1b',
            border: '1px solid #fecaca',
            borderRadius: 6,
            padding: '8px 12px',
            fontSize: 12,
            marginBottom: 16,
          }}
        >
          {error}
        </div>
      )}

      <div style={{ display: 'flex', gap: 8 }}>
        <button
          onClick={openWebApp}
          style={{
            padding: '8px 16px',
            background: '#fbbf24',
            color: '#92400e',
            border: 'none',
            borderRadius: 6,
            cursor: 'pointer',
            fontWeight: 600,
          }}
        >
          Open web app
        </button>
        <button
          onClick={signOut}
          disabled={busy || loggedIn === false}
          style={{
            padding: '8px 16px',
            background: '#ffffff',
            color: '#374151',
            border: '1px solid #d1d5db',
            borderRadius: 6,
            cursor:
              busy || loggedIn === false ? 'not-allowed' : 'pointer',
            opacity: busy || loggedIn === false ? 0.6 : 1,
            fontWeight: 600,
          }}
        >
          {busy ? 'Signing out…' : 'Sign out'}
        </button>
      </div>

      <p style={{ marginTop: 16, fontSize: 12, color: '#9ca3af' }}>
        Signing out stops sync but keeps your local files. Re-sign-in from
        the web app to resume.
      </p>
    </div>
  )
}
