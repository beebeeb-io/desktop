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
import { SIGN_IN_OUTCOME_UNREADABLE } from '../src/accountSwitchCopy'
import { settledFrom, submitPassword, submitTotpCode, type SignInApi } from '../src/onboardingSignIn'

function fakeApi(overrides: Partial<SignInApi> = {}): SignInApi {
  return {
    desktopLogin: async () => ({ ok: true, value: { requires_2fa: false } }),
    desktopLogin2fa: async () => ({ ok: true, value: undefined }),
    ...overrides,
  }
}

describe('onboardingSignIn.submitPassword', () => {
  test('a 2FA account reports requiresTotp — sign-in must NOT be treated as complete', async () => {
    const api = fakeApi({
      desktopLogin: async () => ({ ok: true, value: { requires_2fa: true } }),
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
      desktopLogin: async () => ({ ok: true, value: { requires_2fa: false } }),
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
        return { ok: true, value: { requires_2fa: true } }
      },
      desktopLogin2fa: async (code) => {
        calls.push(`desktop_login_2fa(${code})`)
        return { ok: true, value: undefined }
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
        return { ok: true, value: undefined }
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
  test('maps the three outcomes', () => {
    expect(settledFrom({ requires_2fa: false })).toEqual({ kind: 'fresh' })
    expect(settledFrom(null)).toEqual({ kind: 'fresh' })
    expect(settledFrom({ requires_2fa: false, reauthenticated: true, vault_unlocked: true })).toEqual({ kind: 'reauthenticated', vaultUnlocked: true })
    expect(settledFrom({ requires_2fa: false, reauthenticated: true, vault_unlocked: false })).toEqual({ kind: 'reauthenticated', vaultUnlocked: false })
    expect(settledFrom({ requires_2fa: false, account_mismatch: { pending_changes: 3 } })).toEqual({ kind: 'account_mismatch', pendingChanges: 3 })
  })

  // The Rust side serializes every field every time (LoginOutcome, Task 11): `false`, `false`, `null`.
  test('reads the JSON the Rust LoginOutcome actually sends', () => {
    expect(settledFrom(JSON.parse('{"requires_2fa":false,"reauthenticated":false,"vault_unlocked":false,"account_mismatch":null}'))).toEqual({ kind: 'fresh' })
    expect(settledFrom(JSON.parse('{"requires_2fa":false,"reauthenticated":true,"vault_unlocked":true,"account_mismatch":null}'))).toEqual({ kind: 'reauthenticated', vaultUnlocked: true })
    expect(settledFrom(JSON.parse('{"requires_2fa":false,"reauthenticated":false,"vault_unlocked":false,"account_mismatch":{"pending_changes":3}}'))).toEqual({ kind: 'account_mismatch', pendingChanges: 3 })
    expect(settledFrom(JSON.parse('{"requires_2fa":false,"reauthenticated":false,"vault_unlocked":false,"account_mismatch":{"pending_changes":0}}'))).toEqual({ kind: 'account_mismatch', pendingChanges: 0 })
  })

  test('the same account is never assumed: without `vault_unlocked` the keys are not claimed to be here', () => {
    expect(settledFrom({ requires_2fa: false, reauthenticated: true })).toEqual({ kind: 'reauthenticated', vaultUnlocked: false })
  })

  // Fail closed (lead, Task 18): a result that cannot be classified is never the same account, and
  // never an account switch with an invented count. It is `unreadable`, and the sign-in shows an error.
  test('a shape it cannot classify is `unreadable`, never the same account', () => {
    const base = { requires_2fa: false }
    const unreadable = [
      { ...base, account_mismatch: {} },
      { ...base, account_mismatch: { pending_changes: '3' } },
      { ...base, account_mismatch: { pending_changes: -1 } },
      { ...base, account_mismatch: { pending_changes: 1.5 } },
      { ...base, account_mismatch: { pending_changes: null } },
      { ...base, account_mismatch: true },
      { ...base, account_mismatch: false },
      { ...base, reauthenticated: true, vault_unlocked: true, account_mismatch: { pending_changes: 2 } },
      { ...base, reauthenticated: 'yes' },
      { ...base, reauthenticated: 1 },
      { ...base, reauthenticated: true, vault_unlocked: 'yes' },
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
  const mismatch = { requires_2fa: false, reauthenticated: false, vault_unlocked: false, account_mismatch: { pending_changes: 3 } }
  const reauthenticated = { requires_2fa: false, reauthenticated: true, vault_unlocked: true, account_mismatch: null }

  test('the password step reports a same-account sign-in and an account mismatch', async () => {
    const same = await submitPassword('user@beebeeb.io', 'pw', fakeApi({ desktopLogin: async () => ({ ok: true, value: reauthenticated }) }))
    expect(same).toEqual({ ok: true, requiresTotp: false, settled: { kind: 'reauthenticated', vaultUnlocked: true } })
    const other = await submitPassword('user@beebeeb.io', 'pw', fakeApi({ desktopLogin: async () => ({ ok: true, value: mismatch }) }))
    expect(other).toEqual({ ok: true, requiresTotp: false, settled: { kind: 'account_mismatch', pendingChanges: 3 } })
  })

  test('the 2FA step reports them too (that is where a 2FA account learns it)', async () => {
    const same = await submitTotpCode('123456', fakeApi({ desktopLogin2fa: async () => ({ ok: true, value: reauthenticated }) }))
    expect(same).toEqual({ ok: true, settled: { kind: 'reauthenticated', vaultUnlocked: true } })
    const other = await submitTotpCode('123456', fakeApi({ desktopLogin2fa: async () => ({ ok: true, value: mismatch }) }))
    expect(other).toEqual({ ok: true, settled: { kind: 'account_mismatch', pendingChanges: 3 } })
  })

  test('an unreadable result is an error in both steps, not a completed sign-in', async () => {
    const odd = { requires_2fa: false, account_mismatch: { pending_changes: 'three' } }
    const password = await submitPassword('user@beebeeb.io', 'pw', fakeApi({ desktopLogin: async () => ({ ok: true, value: odd as never }) }))
    expect(password).toEqual({ ok: false, message: SIGN_IN_OUTCOME_UNREADABLE })
    const totp = await submitTotpCode('123456', fakeApi({ desktopLogin2fa: async () => ({ ok: true, value: odd as never }) }))
    expect(totp).toEqual({ ok: false, message: SIGN_IN_OUTCOME_UNREADABLE })
  })

  // Task 1521's bug class: a missing `requires_2fa` used to read as "no second factor needed".
  test('a password result without a boolean `requires_2fa` is unreadable, not "no 2FA needed"', async () => {
    for (const value of [{}, { requires_2fa: 'false' }, { requires_2fa: 0 }, null]) {
      const result = await submitPassword('user@beebeeb.io', 'pw', fakeApi({ desktopLogin: async () => ({ ok: true, value: value as never }) }))
      expect({ value, result }).toEqual({ value, result: { ok: false, message: SIGN_IN_OUTCOME_UNREADABLE } })
    }
  })
})
