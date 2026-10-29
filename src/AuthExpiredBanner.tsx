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
 * The action button uses `forceReauth`, the EXACT SAME sign-in flow as
 * VersionCenter's "Sign in again" review action. On macOS (R8, spec
 * 2026-10-06) it opens sign-in in place and clears nothing: the same account
 * keeps Finder, keys, cache and pending edits, and another account gets the
 * switch warning. On Windows and Linux it clears the expired session first,
 * then opens onboarding — never onboarding on its own, which would
 * fast-forward an "unlocked, configured" user past the sign-in form.
 */

import { useEffect } from 'react'
import { forceReauth } from './desktopApi'
import { useToast } from './windows/ui'

const AUTH_EXPIRED_TOAST_ID = 'auth-expired-banner'
// Distinct id so the failure toast doesn't clobber the persistent banner.
const AUTH_EXPIRED_REAUTH_ERROR_TOAST_ID = 'auth-expired-reauth-error'

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
        onClick: () => {
          void forceReauth().then((result) => {
            if (result.ok) {
              // FB-24: the expired session was cleared, but a step could not be confirmed. Said
              // neutrally, never as "Couldn't …".
              if (result.value.warning) showToast({ variant: 'info', message: result.value.warning.sentence })
              return
            }
            // Surface the failure — the old handler ignored the result, so
            // a sign-out that did not happen (e.g. "Could not stop the sync
            // engine…") left the user stuck with no feedback. The
            // persistent banner itself stays up either way.
            showToast({
              id: AUTH_EXPIRED_REAUTH_ERROR_TOAST_ID,
              variant: 'error',
              title: "Couldn't start sign-in again",
              message: result.reason,
            })
          })
        },
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
