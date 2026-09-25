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
import { submitPassword, submitTotpCode, type SignInApi } from '../src/onboardingSignIn'

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
    expect(result).toEqual({ ok: true, requiresTotp: false })
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
    expect(passwordResult).toEqual({ ok: true, requiresTotp: true })
    // Session must NOT be considered installed yet — the caller's UI is
    // expected to show a code prompt now, not proceed to the vault step.

    const totpResult = await submitTotpCode('123456', api)
    expect(totpResult).toEqual({ ok: true })

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
    expect(result).toEqual({ ok: true })
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
