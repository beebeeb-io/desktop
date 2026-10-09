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
 *
 * R8 (spec 2026-10-06): on macOS "Sign in again" no longer clears anything. It opens sign-in in
 * place, so a same-account sign-in keeps Finder, keys, cache and pending edits. The clear-first
 * order above is now the Windows/Linux flow, pinned unchanged below.
 */
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { forceReauth, type ForceReauthApi } from '../src/desktopApi'

function fakeApi(overrides: Partial<ForceReauthApi> = {}): ForceReauthApi {
  return {
    platform: async () => ({ ok: true, value: 'windows' }),
    clearSession: async () => ({ ok: true, value: { warning: null } }),
    openOnboardingWindow: async () => ({ ok: true, value: undefined }),
    openReauthWindow: async () => ({ ok: true, value: undefined }),
    ...overrides,
  }
}

// FB-24: the clear can be Ok with a warning (it happened, but a step was not confirmed). forceReauth
// carries it back to its two callers, which show it neutrally; it is no reason to stop.
describe('forceReauth and a sign-out warning (FB-24)', () => {
  test('a clear that happened with a warning still opens onboarding, and the warning comes back', async () => {
    const warning = { code: 'finder_removal_unconfirmed' as const, sentence: 'You are signed out, but…' }
    const calls: string[] = []
    const result = await forceReauth(fakeApi({
      clearSession: async () => { calls.push('clear_session'); return { ok: true, value: { warning } } },
      openOnboardingWindow: async () => { calls.push('open_onboarding_window'); return { ok: true, value: undefined } },
    }))
    expect(calls).toEqual(['clear_session', 'open_onboarding_window'])
    expect(result).toEqual({ ok: true, value: { warning } })
  })

  test('on macOS nothing is cleared, so there is no warning', async () => {
    const result = await forceReauth(fakeApi({ platform: async () => ({ ok: true, value: 'macos' }) }))
    expect(result).toEqual({ ok: true, value: { warning: null } })
  })
})

describe('forceReauth', () => {
  test('clears the session BEFORE opening onboarding, in that order', async () => {
    const calls: string[] = []
    const api = fakeApi({
      clearSession: async () => {
        calls.push('clear_session')
        return { ok: true, value: { warning: null } }
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

// M7: a command this build does not have is named correctly. "Sign in again" can stop at the platform
// read, the reauth window (macOS), the clear or the onboarding window (Windows/Linux); the callers show
// the reason as is, so forceReauth names the command that is missing.
describe('forceReauth names the command a build is missing (M7)', () => {
  const missing = { ok: false as const, reason: 'Command not found', unsupported: true }
  test('desktop_platform', async () => {
    const result = await forceReauth(fakeApi({ platform: async () => missing }))
    expect(result).toEqual({ ok: false, reason: 'desktop_platform is not wired in this build yet.', unsupported: true })
  })
  test('open_reauth_window on macOS', async () => {
    const result = await forceReauth(fakeApi({ platform: async () => ({ ok: true, value: 'macos' }), openReauthWindow: async () => missing }))
    expect(result).toEqual({ ok: false, reason: 'open_reauth_window is not wired in this build yet.', unsupported: true })
  })
  test('clear_session and open_onboarding_window elsewhere', async () => {
    expect(await forceReauth(fakeApi({ clearSession: async () => missing }))).toEqual({ ok: false, reason: 'clear_session is not wired in this build yet.', unsupported: true })
    expect(await forceReauth(fakeApi({ openOnboardingWindow: async () => missing }))).toEqual({ ok: false, reason: 'open_onboarding_window is not wired in this build yet.', unsupported: true })
  })
  test('a failure that is not a missing command keeps its own reason', async () => {
    expect(await forceReauth(fakeApi({ openOnboardingWindow: async () => ({ ok: false, reason: 'window failed', unsupported: false }) }))).toEqual({ ok: false, reason: 'window failed', unsupported: false })
  })
  test('the review action shows the reason it gets, and names no command of its own', () => {
    const text = readFileSync(new URL('../src/pages/VersionCenter.tsx', import.meta.url), 'utf8')
    const body = text.slice(text.indexOf('const signInAgain = async () => {'), text.indexOf('const restoreVersion'))
    expect(body).not.toContain('open_onboarding_window')
    expect(body).toContain('message: result.reason')
  })
})

describe('forceReauth on macOS (R8: re-sign-in in place)', () => {
  test('opens sign-in in place and never clears the session', async () => {
    const calls: string[] = []
    const api = fakeApi({
      platform: async () => ({ ok: true, value: 'macos' }),
      clearSession: async () => { calls.push('clear_session'); return { ok: true, value: { warning: null } } },
      openOnboardingWindow: async () => { calls.push('open_onboarding_window'); return { ok: true, value: undefined } },
      openReauthWindow: async () => { calls.push('open_reauth_window'); return { ok: true, value: undefined } },
    })
    expect((await forceReauth(api)).ok).toBe(true)
    expect(calls).toEqual(['open_reauth_window'])
  })

  test('if the platform cannot be read, nothing is cleared', async () => {
    const calls: string[] = []
    const api = fakeApi({
      platform: async () => ({ ok: false, reason: 'ipc down', unsupported: false }),
      clearSession: async () => { calls.push('clear_session'); return { ok: true, value: { warning: null } } },
    })
    expect((await forceReauth(api)).ok).toBe(false)
    expect(calls).toEqual([])
  })

  test('a failed open_reauth_window is reported, and still nothing is cleared', async () => {
    const calls: string[] = []
    const api = fakeApi({
      platform: async () => ({ ok: true, value: 'macos' }),
      clearSession: async () => { calls.push('clear_session'); return { ok: true, value: { warning: null } } },
      openOnboardingWindow: async () => { calls.push('open_onboarding_window'); return { ok: true, value: undefined } },
      openReauthWindow: async () => ({ ok: false, reason: 'window failed', unsupported: false }),
    })
    expect(await forceReauth(api)).toEqual({ ok: false, reason: 'window failed', unsupported: false })
    expect(calls).toEqual([])
  })

  for (const platform of ['windows', 'linux'] as const) {
    test(`${platform} is unchanged: clear first, then onboarding`, async () => {
      const calls: string[] = []
      const api = fakeApi({
        platform: async () => ({ ok: true, value: platform }),
        clearSession: async () => { calls.push('clear_session'); return { ok: true, value: { warning: null } } },
        openOnboardingWindow: async () => { calls.push('open_onboarding_window'); return { ok: true, value: undefined } },
        openReauthWindow: async () => { calls.push('open_reauth_window'); return { ok: true, value: undefined } },
      })
      await forceReauth(api)
      expect(calls).toEqual(['clear_session', 'open_onboarding_window'])
    })
  }
})
