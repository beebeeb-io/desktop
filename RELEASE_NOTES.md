# Beebeeb Desktop 0.8.10 — your desktop survives a dead session

A password change on any device revokes every session, and the Windows app used
to trap you behind one: every page 401'd, the only sign-out control disappeared
with the failing profile load, the menu path failed while blaming the sync
engine, and every sign-in route demanded a sign-out that could never succeed.
This release turns a revoked session into a two-click recovery.

### What's New

- **[Windows] Dead sessions are detected at startup (PR #108):** the app
  validates its stored session with one bounded `/auth/me` check when it boots.
  On a definitive HTTP 401 it clears the stored credentials and lands on the
  sign-in form with your email prefilled. Network errors, timeouts, and server
  errors fail open — an offline laptop is never logged out.
- **[Windows] Sign-out is reachable again:** the Account page shows the
  disconnect control even when the profile load fails — the control now exists
  exactly when you need it.
- **[Windows] Sign-out tells the truth:** the engine-abort result is a typed
  status, so errors name the real failing stage (Cloud Files revocation vs the
  sync engine task) instead of a generic "could not stop the sync engine". A
  failed abort re-arms the stop gate, so a retry can no longer skip the
  confirmation and purge while the engine may still be running.
- **[Windows] Signing out while signed out is a no-op:** the menu says "You are
  not signed in on this device." instead of running a teardown that could fail
  on stale state.
- **[Windows] The "Sign in again" banner stops swallowing errors:** if
  re-authentication fails, the banner says why.

### Windows and Linux

Windows carries the fixes above. Linux assets are rebuilt from the same source
with no behavior changes; macOS is unchanged in this release.
