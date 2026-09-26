/**
 * Task 1546 Codex round 2, finding 2: "Sign in again" must actually force
 * the sign-in flow. Both onboarding implementations decide "already signed
 * in" from `sync_status`'s `logged_in` field — if the expired token is still
 * installed when onboarding opens, `logged_in` still reads `true` and
 * onboarding fast-forwards an "unlocked, configured" user straight past the
 * sign-in form, never actually replacing the invalid token.
 *
 * `forceReauth` (src/desktopApi.ts) is the fix: clear the session FIRST,
 * THEN open onboarding. It's the shared decision both VersionCenter's "Sign
 * in again" review action and the persistent auth-expired banner
 * (AuthExpiredBanner.tsx) route through — these tests drive it directly
 * against an injected fake API, no Tauri runtime needed (mirrors
 * onboardingSignIn.test.ts's pattern for the same DI shape).
 */
import { describe, expect, test } from 'bun:test'
import { forceReauth, type ForceReauthApi } from '../src/desktopApi'

function fakeApi(overrides: Partial<ForceReauthApi> = {}): ForceReauthApi {
  return {
    clearSession: async () => ({ ok: true, value: undefined }),
    openOnboardingWindow: async () => ({ ok: true, value: undefined }),
    ...overrides,
  }
}

describe('forceReauth', () => {
  test('clears the session BEFORE opening onboarding, in that order', async () => {
    const calls: string[] = []
    const api = fakeApi({
      clearSession: async () => {
        calls.push('clear_session')
        return { ok: true, value: undefined }
      },
      openOnboardingWindow: async () => {
        calls.push('open_onboarding_window')
        return { ok: true, value: undefined }
      },
    })

    const result = await forceReauth(api)

    expect(result.ok).toBe(true)
    // The exact bug this fixes: the old code called `open_onboarding_window`
    // alone, with the stale session still installed. This asserts the real
    // fix's ordering, not just that both calls eventually happen.
    expect(calls).toEqual(['clear_session', 'open_onboarding_window'])
  })

  test('never opens onboarding when clearing the session fails', async () => {
    const calls: string[] = []
    const api = fakeApi({
      clearSession: async () => {
        calls.push('clear_session')
        return { ok: false, reason: 'session mutex poisoned', unsupported: false }
      },
      openOnboardingWindow: async () => {
        calls.push('open_onboarding_window')
        return { ok: true, value: undefined }
      },
    })

    const result = await forceReauth(api)

    expect(result).toEqual({ ok: false, reason: 'session mutex poisoned', unsupported: false })
    expect(calls).toEqual(['clear_session'])
  })
})
