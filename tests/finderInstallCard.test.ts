/**
 * Task 1524 Issue 4 — red-first regression test for the "Install the Finder
 * location" card's decision logic.
 *
 * Symptom (Guus, 2026-09-28, real Mac): "Install Finder location" always
 * failed with "Timed out waiting for the Beebeeb File Provider domain to
 * become available" — even after the socket/sandbox-home fixes from earlier
 * in task 1524. Root cause: the domain already existed and was disabled by
 * the user in System Settings -> General -> Login Items & Extensions -> File
 * Providers. `addDomain` reports success either way, so the old code always
 * waited the full 10s for a stabilization callback that a disabled domain's
 * extension can never fire.
 *
 * `install_finder_location` (src-tauri/src/lib.rs) now returns `Ok` even in
 * this case, with `installed: false` and `reason_category: "user_disabled"`
 * — deliberately NOT an `Err`, so the frontend can distinguish it from a real
 * success. `classifyFinderInstallResult` (src/finderInstallCard.ts) is the
 * extracted decision point the fixed `Onboarding.tsx` now calls instead of
 * checking `result.ok` alone — checking `result.ok` alone is exactly the bug
 * these tests guard against (it was true in ALL three cases below the first
 * one, including the user_disabled case).
 */
import { describe, expect, test } from 'bun:test'
import {
  classifyFinderInstallResult,
  finderInstallStateAfterAttempt,
  finderInstallStateWhileAttempting,
  finderLocationButtonPlan,
  shouldRetryAfterUserEnabledPoll,
} from '../src/finderInstallCard'
import type { CommandResult, FinderInstallState } from '../src/desktopApi'

function installState(overrides: Partial<FinderInstallState> = {}): FinderInstallState {
  return {
    installed: false,
    path: null,
    status: 'missing',
    last_error: null,
    last_attempt_at: null,
    reason_category: null,
    ...overrides,
  }
}

describe('finderInstallCard.classifyFinderInstallResult', () => {
  test('a real install success is "installed" — the only case that should advance onboarding', () => {
    const result: CommandResult<FinderInstallState> = {
      ok: true,
      value: installState({ installed: true, path: '/Users/guus/Beebeeb', status: 'installed' }),
    }
    expect(classifyFinderInstallResult(result)).toEqual({ kind: 'installed', path: '/Users/guus/Beebeeb' })
  })

  test('a user-disabled domain is "user_disabled", NOT "installed" — this is the exact bug from Issue 4', () => {
    const result: CommandResult<FinderInstallState> = {
      ok: true,
      value: installState({
        installed: false,
        status: 'error',
        reason_category: 'user_disabled',
        last_error: 'Beebeeb is turned off in System Settings. Open Login Items & Extensions, turn on Beebeeb under File Providers, then try again.',
      }),
    }
    // The pre-fix bug: `result.ok` is `true` here too. Only `installed`/`reason_category`
    // distinguish this from a real success — asserting `kind !== 'installed'` is the
    // whole point of this test.
    const outcome = classifyFinderInstallResult(result)
    expect(outcome.kind).toBe('user_disabled')
    if (outcome.kind !== 'user_disabled') throw new Error('unreachable')
    expect(outcome.message).toContain('turned off in System Settings')
  })

  test('a genuine timeout (some other reason_category) is a plain error, not user_disabled', () => {
    const result: CommandResult<FinderInstallState> = {
      ok: true,
      value: installState({
        installed: false,
        status: 'error',
        reason_category: 'timeout',
        last_error: 'Timed out waiting for the Beebeeb File Provider domain to become available',
      }),
    }
    expect(classifyFinderInstallResult(result)).toEqual({
      kind: 'error',
      message: 'Timed out waiting for the Beebeeb File Provider domain to become available',
    })
  })

  test('a command-level failure (result.ok: false) surfaces the raw reason', () => {
    const result: CommandResult<FinderInstallState> = {
      ok: false,
      reason: 'Finder location must be absolute: relative/path',
      unsupported: false,
    }
    expect(classifyFinderInstallResult(result)).toEqual({
      kind: 'error',
      message: 'Finder location must be absolute: relative/path',
    })
  })
})

describe('finderInstallCard.shouldRetryAfterUserEnabledPoll', () => {
  test('userEnabled flips to true -> retry', () => {
    expect(shouldRetryAfterUserEnabledPoll({ ok: true, value: true })).toBe(true)
  })

  test('still disabled -> do not retry', () => {
    expect(shouldRetryAfterUserEnabledPoll({ ok: true, value: false })).toBe(false)
  })

  test('domain not registered (null) -> do not retry', () => {
    expect(shouldRetryAfterUserEnabledPoll({ ok: true, value: null })).toBe(false)
  })

  test('a failed poll must never be read as "the user fixed it"', () => {
    expect(shouldRetryAfterUserEnabledPoll({ ok: false, reason: 'lookup failed', unsupported: false })).toBe(false)
  })
})

describe('finderInstallCard.finderLocationButtonPlan (task 1670)', () => {
  test('installed -> only "Open in Finder", as the primary action; no "Install" button', () => {
    // The exact bug from Guus's screenshot: the pill said "Installed" but the
    // pane also showed the amber "Install in Finder" button. This is the one
    // assertion that guards against it ever coming back.
    expect(finderLocationButtonPlan(true)).toEqual({ showInstall: false, showOpen: true })
  })

  test('not installed -> only "Install in Finder"', () => {
    expect(finderLocationButtonPlan(false)).toEqual({ showInstall: true, showOpen: false })
  })
})

describe('finderInstallCard one-inline-error helpers (task 1683 slice 5, decision D1)', () => {
  const UNAVAILABLE = 'install_finder_location is not wired in this build yet.'
  const saved = installState({ status: 'error', last_error: 'Timed out', reason_category: 'timeout', path: 'Beebeeb in Finder' })

  test('a saved failure returned as a state is passed through untouched (it is already the one source)', () => {
    expect(finderInstallStateAfterAttempt({ ok: true, value: saved }, null, UNAVAILABLE)).toEqual(saved)
  })

  test('a rejected command (never saved) is folded into the same inline state, keeping the known path', () => {
    const next = finderInstallStateAfterAttempt({ ok: false, reason: 'Finder location must be absolute', unsupported: false }, installState({ path: 'Beebeeb in Finder' }), UNAVAILABLE)
    expect(next).toMatchObject({ installed: false, status: 'error', last_error: 'Finder location must be absolute', reason_category: null, path: 'Beebeeb in Finder' })
  })

  test('a rejection with no previous state still yields an inline error, not nothing', () => {
    const next = finderInstallStateAfterAttempt({ ok: false, reason: 'boom', unsupported: false }, null, UNAVAILABLE)
    expect(next.installed).toBe(false)
    expect(next.last_error).toBe('boom')
  })

  test('an unsupported command shows the honest label, not the raw transport error', () => {
    const next = finderInstallStateAfterAttempt({ ok: false, reason: 'unknown command install_finder_location', unsupported: true }, null, UNAVAILABLE)
    expect(next.last_error).toBe(UNAVAILABLE)
  })

  test('a failed attempt never reads as installed, even if the previous state was', () => {
    const next = finderInstallStateAfterAttempt({ ok: false, reason: 'boom', unsupported: false }, installState({ installed: true }), UNAVAILABLE)
    expect(next.installed).toBe(false)
  })

  test('starting a new attempt clears the stale failure and keeps everything else', () => {
    const next = finderInstallStateWhileAttempting(saved)
    expect(next).toMatchObject({ last_error: null, status: 'missing', reason_category: null, path: 'Beebeeb in Finder', installed: false })
  })

  test('starting a new attempt does not rewrite a non-error status, and tolerates no state yet', () => {
    expect(finderInstallStateWhileAttempting(installState({ status: 'installed', installed: true }))?.status).toBe('installed')
    expect(finderInstallStateWhileAttempting(null)).toBeNull()
  })
})
