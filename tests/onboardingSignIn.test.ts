/**
 * Task 1521 — red-first regression test for the macOS onboarding 2FA bug.
 *
 * Symptom (Guus, 2026-09-25, real Mac, TOTP-enabled prod account): sign-in
 * never asked for a 2FA code, then the recovery-phrase step failed with
 * "Sign in before unlocking the vault." Root cause: `Onboarding.tsx`'s
 * `SignInStep` called `desktop_login` and advanced on `result.ok` alone —
 * true even when the server reports `requires_2fa: true`, because the
 * password step only proves the password, not the second factor. No session
 * token was ever persisted, so the later vault-unlock step found none.
 *
 * `submitPassword` (src/onboardingSignIn.ts) is the extracted decision point
 * the fixed component now calls. These tests drive it directly against a
 * mocked `desktopLogin`/`desktopLogin2fa` pair — no Tauri runtime needed.
 */
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { SIGN_IN_OUTCOME_UNREADABLE } from '../src/accountSwitchCopy'
import { settledFrom, submitPassword, submitTotpCode, type SignInApi } from '../src/onboardingSignIn'

/** What Rust's `LoginOutcome` always sends: all five fields (lib.rs `login_outcome_json_is_the_frontends_contract`). */
const outcome = (over: Record<string, unknown> = {}) => ({ requires_2fa: false, reauthenticated: false, vault_unlocked: false, key_replaced: false, account_mismatch: null, ...over }) as never

function fakeApi(overrides: Partial<SignInApi> = {}): SignInApi {
  return {
    desktopLogin: async () => ({ ok: true, value: outcome() }),
    desktopLogin2fa: async () => ({ ok: true, value: outcome() }),
    ...overrides,
  }
}

describe('onboardingSignIn.submitPassword', () => {
  test('a 2FA account reports requiresTotp — sign-in must NOT be treated as complete', async () => {
    const api = fakeApi({
      desktopLogin: async () => ({ ok: true, value: outcome({ requires_2fa: true }) }),
    })
    const result = await submitPassword('user@beebeeb.io', 'correct horse', api)
    expect(result.ok).toBe(true)
    if (!result.ok) throw new Error('unreachable')
    // This is the exact assertion the old code got wrong: `ok: true` alone
    // used to be read as "done". It is NOT done until requiresTotp is false.
    expect(result.requiresTotp).toBe(true)
  })

  test('an account without 2FA reports requiresTotp: false — sign-in completes immediately', async () => {
    const api = fakeApi({
      desktopLogin: async () => ({ ok: true, value: outcome() }),
    })
    const result = await submitPassword('user@beebeeb.io', 'correct horse', api)
    expect(result).toEqual({ ok: true, requiresTotp: false, settled: { kind: 'fresh' } })
  })

  test('a wrong password surfaces the server message and never reaches requiresTotp', async () => {
    const api = fakeApi({
      desktopLogin: async () => ({ ok: false, reason: 'Invalid email or password', unsupported: false }),
    })
    const result = await submitPassword('user@beebeeb.io', 'wrong', api)
    expect(result).toEqual({ ok: false, message: 'Invalid email or password' })
  })
})

describe('onboardingSignIn.submitTotpCode', () => {
  test('the full 2FA sequence: password → requiresTotp → code prompt → desktop_login_2fa → signed in', async () => {
    const calls: string[] = []
    const api: SignInApi = {
      desktopLogin: async (email, password) => {
        calls.push(`desktop_login(${email}, ${password})`)
        return { ok: true, value: outcome({ requires_2fa: true }) }
      },
      desktopLogin2fa: async (code) => {
        calls.push(`desktop_login_2fa(${code})`)
        return { ok: true, value: outcome() }
      },
    }

    const passwordResult = await submitPassword('user@beebeeb.io', 'correct horse', api)
    expect(passwordResult).toEqual({ ok: true, requiresTotp: true, settled: { kind: 'fresh' } })
    // Session must NOT be considered installed yet — the caller's UI is
    // expected to show a code prompt now, not proceed to the vault step.

    const totpResult = await submitTotpCode('123456', api)
    expect(totpResult).toEqual({ ok: true, settled: { kind: 'fresh' } })

    // Both legs of the handoff actually ran, in order — proves the code
    // prompt's `desktop_login_2fa` call really happens before sign-in is
    // considered complete.
    expect(calls).toEqual(['desktop_login(user@beebeeb.io, correct horse)', 'desktop_login_2fa(123456)'])
  })

  test('an 8-digit backup code is passed through unchanged (server tries TOTP then backup codes)', async () => {
    let received: string | null = null
    const api = fakeApi({
      desktopLogin2fa: async (code) => {
        received = code
        return { ok: true, value: outcome() }
      },
    })
    const result = await submitTotpCode('12345678', api)
    expect(result).toEqual({ ok: true, settled: { kind: 'fresh' } })
    expect(received).toBe('12345678')
  })

  test('a wrong code is retryable: surfaces the message, does not throw', async () => {
    const api = fakeApi({
      desktopLogin2fa: async () => ({ ok: false, reason: 'Invalid authentication code', unsupported: false }),
    })
    const result = await submitTotpCode('000000', api)
    expect(result).toEqual({ ok: false, message: 'Invalid authentication code' })
  })

  test('an unsupported build (older Tauri binary missing the command) surfaces a clear label', async () => {
    const api = fakeApi({
      desktopLogin2fa: async () => ({ ok: false, reason: 'not found', unsupported: true }),
    })
    const result = await submitTotpCode('123456', api)
    expect(result.ok).toBe(false)
    if (result.ok) throw new Error('unreachable')
    expect(result.message).toContain('desktop_login_2fa')
  })
})

describe('settledFrom (R8)', () => {
  test('maps the outcomes', () => {
    expect(settledFrom(outcome())).toEqual({ kind: 'fresh' })
    expect(settledFrom(outcome({ reauthenticated: true, vault_unlocked: true }))).toEqual({ kind: 'reauthenticated', vaultUnlocked: true, keyReplaced: false })
    expect(settledFrom(outcome({ reauthenticated: true, vault_unlocked: false }))).toEqual({ kind: 'reauthenticated', vaultUnlocked: false, keyReplaced: false })
    expect(settledFrom(outcome({ account_mismatch: { pending_changes: 3 } }))).toEqual({ kind: 'account_mismatch', pendingChanges: 3 })
  })

  // FB-I1: a same-account re-sign-in whose kept vault key the server no longer accepts. The key was
  // removed, so the recovery phrase follows, and that step says why.
  test('key_replaced: the same account, no keys here, and the reason', () => {
    expect(settledFrom(outcome({ reauthenticated: true, vault_unlocked: false, key_replaced: true }))).toEqual({ kind: 'reauthenticated', vaultUnlocked: false, keyReplaced: true })
  })

  test('key_replaced is required and boolean: without it, or with anything else, the result is unreadable', () => {
    const { key_replaced: _omitted, ...withoutIt } = outcome() as Record<string, unknown>
    for (const value of [withoutIt, outcome({ key_replaced: null }), outcome({ key_replaced: 'true' }), outcome({ key_replaced: 1 })]) {
      expect({ value, settled: settledFrom(value as never) }).toEqual({ value, settled: { kind: 'unreadable' } })
    }
  })

  test('a replaced key with anything that contradicts it is unreadable', () => {
    for (const value of [
      outcome({ key_replaced: true }),
      outcome({ key_replaced: true, reauthenticated: true, vault_unlocked: true }),
      outcome({ key_replaced: true, account_mismatch: { pending_changes: 1 } }),
    ]) {
      expect({ value, settled: settledFrom(value) }).toEqual({ value, settled: { kind: 'unreadable' } })
    }
  })

  test('reads every JSON shape the Rust LoginOutcome contract pins (lib.rs), and none is unreadable', () => {
    const lib = readFileSync(new URL('../src-tauri/src/lib.rs', import.meta.url), 'utf8')
    const start = lib.indexOf('fn login_outcome_json_is_the_frontends_contract()')
    const body = lib.slice(start, lib.indexOf('\n    }\n', start))
    const shapes = [...body.matchAll(/r#"(\{.*?\})"#/g)].map((m) => JSON.parse(m[1]))
    expect(shapes.length).toBeGreaterThanOrEqual(7)
    const settled = shapes.map((shape) => settledFrom(shape))
    expect(settled.filter((s) => s.kind === 'unreadable')).toEqual([])
    expect(settled).toContainEqual({ kind: 'reauthenticated', vaultUnlocked: false, keyReplaced: true })
    expect(settled).toContainEqual({ kind: 'reauthenticated', vaultUnlocked: true, keyReplaced: false })
    expect(settled).toContainEqual({ kind: 'account_mismatch', pendingChanges: 3 })
  })

  // M9: Lane R always sends all five fields (lib.rs pins it), so a bundle cannot skew against it any
  // more. A missing field, or no result at all, is a contract break: unreadable, never a plain sign-in.
  test('every R8 field is required: a missing one, or no result at all, is unreadable (M9)', () => {
    for (const field of ['requires_2fa', 'reauthenticated', 'vault_unlocked', 'key_replaced', 'account_mismatch']) {
      const { [field]: _omitted, ...withoutIt } = outcome() as Record<string, unknown>
      expect({ field, settled: settledFrom(withoutIt as never) }).toEqual({ field, settled: { kind: 'unreadable' } })
    }
    expect(settledFrom(null)).toEqual({ kind: 'unreadable' })
    expect(settledFrom(undefined)).toEqual({ kind: 'unreadable' })
    for (const field of ['reauthenticated', 'vault_unlocked']) {
      expect(settledFrom(outcome({ [field]: null }))).toEqual({ kind: 'unreadable' })
    }
    expect(settledFrom(outcome({ requires_2fa: 'false' }))).toEqual({ kind: 'unreadable' })
  })

  // Fail closed (lead, Task 18): a result that cannot be classified is never the same account, and
  // never an account switch with an invented count. It is `unreadable`, and the sign-in shows an error.
  test('a shape it cannot classify is `unreadable`, never the same account', () => {
    const unreadable = [
      outcome({ account_mismatch: {} }),
      outcome({ account_mismatch: { pending_changes: '3' } }),
      outcome({ account_mismatch: { pending_changes: -1 } }),
      outcome({ account_mismatch: { pending_changes: 1.5 } }),
      outcome({ account_mismatch: { pending_changes: null } }),
      outcome({ account_mismatch: true }),
      outcome({ account_mismatch: false }),
      outcome({ reauthenticated: true, vault_unlocked: true, account_mismatch: { pending_changes: 2 } }),
      outcome({ reauthenticated: 'yes' }),
      outcome({ reauthenticated: 1 }),
      outcome({ reauthenticated: true, vault_unlocked: 'yes' }),
      'signed in',
      42,
      [],
    ]
    for (const value of unreadable) {
      expect({ value, settled: settledFrom(value as never) }).toEqual({ value, settled: { kind: 'unreadable' } })
    }
  })
})

describe('onboardingSignIn carries what the sign-in became (R8)', () => {
  const mismatch = outcome({ account_mismatch: { pending_changes: 3 } })
  const reauthenticated = outcome({ reauthenticated: true, vault_unlocked: true })

  test('the password step reports a same-account sign-in and an account mismatch', async () => {
    const same = await submitPassword('user@beebeeb.io', 'pw', fakeApi({ desktopLogin: async () => ({ ok: true, value: reauthenticated }) }))
    expect(same).toEqual({ ok: true, requiresTotp: false, settled: { kind: 'reauthenticated', vaultUnlocked: true, keyReplaced: false } })
    const other = await submitPassword('user@beebeeb.io', 'pw', fakeApi({ desktopLogin: async () => ({ ok: true, value: mismatch }) }))
    expect(other).toEqual({ ok: true, requiresTotp: false, settled: { kind: 'account_mismatch', pendingChanges: 3 } })
  })

  test('the 2FA step reports them too (that is where a 2FA account learns it)', async () => {
    const same = await submitTotpCode('123456', fakeApi({ desktopLogin2fa: async () => ({ ok: true, value: reauthenticated }) }))
    expect(same).toEqual({ ok: true, settled: { kind: 'reauthenticated', vaultUnlocked: true, keyReplaced: false } })
    const other = await submitTotpCode('123456', fakeApi({ desktopLogin2fa: async () => ({ ok: true, value: mismatch }) }))
    expect(other).toEqual({ ok: true, settled: { kind: 'account_mismatch', pendingChanges: 3 } })
  })

  test('an unreadable result is an error in both steps, not a completed sign-in', async () => {
    const odd = outcome({ account_mismatch: { pending_changes: 'three' } })
    const password = await submitPassword('user@beebeeb.io', 'pw', fakeApi({ desktopLogin: async () => ({ ok: true, value: odd as never }) }))
    expect(password).toEqual({ ok: false, message: SIGN_IN_OUTCOME_UNREADABLE })
    const totp = await submitTotpCode('123456', fakeApi({ desktopLogin2fa: async () => ({ ok: true, value: odd as never }) }))
    expect(totp).toEqual({ ok: false, message: SIGN_IN_OUTCOME_UNREADABLE })
  })

  // FB-I1: the sign-in can stop because the old key could not be removed; Rust's sentence is the
  // form's error, verbatim, like any other sign-in error.
  test('SIGN_IN_KEY_NOT_REMOVED is the form\'s error, verbatim, in both steps', async () => {
    const sentence = 'Beebeeb couldn’t remove this account’s old vault key from this computer, so nothing changed. Try again.'
    const password = await submitPassword('user@beebeeb.io', 'pw', fakeApi({ desktopLogin: async () => ({ ok: false, reason: sentence, unsupported: false }) }))
    expect(password).toEqual({ ok: false, message: sentence })
    const totp = await submitTotpCode('123456', fakeApi({ desktopLogin2fa: async () => ({ ok: false, reason: sentence, unsupported: false }) }))
    expect(totp).toEqual({ ok: false, message: sentence })
  })

  // Task 1521's bug class: a missing `requires_2fa` used to read as "no second factor needed".
  test('a password result without a boolean `requires_2fa` is unreadable, not "no 2FA needed"', async () => {
    for (const value of [{}, { requires_2fa: 'false' }, { requires_2fa: 0 }, null]) {
      const result = await submitPassword('user@beebeeb.io', 'pw', fakeApi({ desktopLogin: async () => ({ ok: true, value: value as never }) }))
      expect({ value, result }).toEqual({ value, result: { ok: false, message: SIGN_IN_OUTCOME_UNREADABLE } })
    }
  })
})
