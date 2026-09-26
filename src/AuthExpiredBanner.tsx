/**
 * AuthExpiredBanner — the persistent "You're signed out on this device" call
 * to action (task 1546 Codex round 2, finding 5 / lead decision).
 *
 * The Rust runner tracks consecutive 401s from its heartbeat + sync-tick API
 * calls (`AuthHealth` in `src-tauri/src/runner.rs`); after 3 in a row it sets
 * `auth_expired: true` on the `sync_status` IPC response, independent of
 * `logged_in` (a stale token can still be installed — `logged_in: true` —
 * while every call it makes fails). Any subsequent success, or a fresh
 * sign-in, resets it.
 *
 * Mirrors `UpdateBanner`'s pattern: no bar of its own, just a persistent
 * (non-dismissible, `durationMs: null`) toast via the shared toast system —
 * driven by the parent's already-polled `sync_status`, not its own poll.
 * The action button uses `forceReauth`, the EXACT SAME forced sign-in flow
 * as VersionCenter's "Sign in again" review action: clear the expired
 * session first, then open onboarding — never open onboarding on its own,
 * which would fast-forward an "unlocked, configured" user past the sign-in
 * form.
 */

import { useEffect } from 'react'
import { forceReauth } from './desktopApi'
import { useToast } from './windows/ui'

const AUTH_EXPIRED_TOAST_ID = 'auth-expired-banner'

export default function AuthExpiredBanner({ authExpired }: { authExpired: boolean }) {
  const { showToast, dismissToast } = useToast()

  useEffect(() => {
    if (!authExpired) {
      dismissToast(AUTH_EXPIRED_TOAST_ID)
      return
    }
    showToast({
      id: AUTH_EXPIRED_TOAST_ID,
      variant: 'warning',
      title: "You're signed out on this device",
      message: 'Sync is paused until you sign in again.',
      action: {
        label: 'Sign in again',
        onClick: () => void forceReauth(),
      },
      durationMs: null,
      dismissible: false,
    })
    // Re-shown/kept alive whenever `authExpired` flips true; dismissed above
    // the instant it flips false (a successful call or a fresh sign-in).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [authExpired])

  return null
}
