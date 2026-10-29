import { afterEach, describe, expect, test } from 'bun:test'
import { createElement, Fragment } from 'react'
import * as accountSession from '../src/accountSession'
import * as desktopApi from '../src/desktopApi'
import * as diagnosticsCopy from '../src/diagnosticsCopy'
import * as planPresentation from '../src/planPresentation'
import * as copy from '../src/signOutCopy'
import * as switchCopy from '../src/accountSwitchCopy'
import { settledFrom } from '../src/onboardingSignIn'
import { expand, mount, textOf, type Mounted } from './fixtures/componentHarness'

const React = { createElement, Fragment }
const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close() })

/** What the step shows: `tree()` is the unexpanded root, whose Card holds the title and first line as props. */
const shown = (m: Mounted) => textOf(expand(m.tree()))

const unsent = (over: Partial<copy.AskCounts> = {}): copy.AskCounts => ({ askId: 'switch-1', files: 1, copiedFiles: 1, others: 1, missingFiles: 0, earlierFolders: [], ...over })

function switchStep(props: { pendingChanges: number; unsent: copy.AskCounts | null; unsentError?: string | null }, backend: Record<string, (args: any) => unknown>) {
  const m = mount('Onboarding.tsx', 'AccountSwitchStep', {
    backend: { sign_out_result_ack: () => null, ...backend },
    expand: true,
    props: { unsentError: null, ...props, onSwitched: () => {}, onCancel: () => {} },
    bindings: {
      clearSession: desktopApi.clearSession, command: desktopApi.command, commandUnavailableLabel: desktopApi.commandUnavailableLabel,
      Card: ({ title, copy: body, children }: any) => createElement('section', null, createElement('h3', null, title), createElement('p', null, body), children),
      ...switchCopy, ...copy,
    },
  })
  mounted.push(m)
  return m
}

describe('entry points (spec 2026-10-10 §4)', () => {
  /** F8. */
  test('the switch step renders the snapshot, keeps with its askId, and asks again on a refusal', async () => {
    let refuse = true
    const m = switchStep({ pendingChanges: 2, unsent: unsent() }, {
      clear_session: () => { if (refuse) { refuse = false; throw new Error('unsent_changes: Some changes…') } return { warning: null } },
      sign_out_count: () => ({ ...unsent({ askId: 'switch-2', files: 2, copiedFiles: 2 }), canSend: false, notSending: null }),
    })
    await m.flush()
    const text = shown(m)
    for (const line of [copy.A0S_TITLE, copy.A1S_LINE, copy.askLines(unsent(), null, true).paragraphs[0], copy.askLines(unsent(), null, true).paragraphs[1]]) {
      expect(text).toContain(line)
    }
    expect(text).not.toContain('removes it')
    expect(text).not.toContain('removes them')
    await m.click(copy.ASK_SWITCH)
    expect(m.calls.find((c) => c.name === 'clear_session')?.args).toEqual({ forgetEmail: true, keepUnsent: { askId: 'switch-1' } })
    expect(m.toasts).toEqual([])
    expect(shown(m)).toContain('2 files changed in Finder while that account was signed in')
    await m.click(copy.ASK_SWITCH)
    expect(m.calls.filter((c) => c.name === 'clear_session').at(-1)?.args).toEqual({ forgetEmail: true, keepUnsent: { askId: 'switch-2' } })
  })

  test('the switch step with nothing waiting is spec A\'s, and with no snapshot (Linux) it keeps spec A\'s count', async () => {
    const empty = switchStep({ pendingChanges: 0, unsent: unsent({ files: 0, copiedFiles: 0, others: 0 }) }, {})
    await empty.flush()
    expect(shown(empty)).toContain(switchCopy.ACCOUNT_SWITCH_TITLE)
    expect(shown(empty)).toContain(switchCopy.accountSwitchBody(0))
    const linux = switchStep({ pendingChanges: 2, unsent: null }, { clear_session: () => ({ warning: null }) })
    await linux.flush()
    expect(shown(linux)).toContain(switchCopy.accountSwitchBody(2))
    await linux.click(switchCopy.ACCOUNT_SWITCH_CONFIRM)
    expect(linux.calls.find((c) => c.name === 'clear_session')?.args).toEqual({ forgetEmail: true })
  })

  test('another error keeps today\'s toast, then acknowledges it once (F12)', async () => {
    const m = switchStep({ pendingChanges: 1, unsent: unsent() }, { clear_session: () => { throw new Error('Sign-out paused: x') } })
    await m.flush()
    await m.click(copy.ASK_SWITCH)
    await m.flush()
    expect(m.toasts.map((t) => t.title)).toEqual([switchCopy.ACCOUNT_SWITCH_FAILED])
    expect(m.calls.filter((c) => c.name === 'sign_out_result_ack')).toHaveLength(1)
  })

  test('settledFrom carries the snapshot, and reads a missing one as none', () => {
    const base = { requires_2fa: false, reauthenticated: false, vault_unlocked: false, key_replaced: false }
    expect(settledFrom({ ...base, account_mismatch: { pending_changes: 1, unsent: unsent() } } as any)).toEqual({ kind: 'account_mismatch', pendingChanges: 1, unsent: unsent(), unsentError: null })
    expect(settledFrom({ ...base, account_mismatch: { pending_changes: 2 } } as any)).toEqual({ kind: 'account_mismatch', pendingChanges: 2, unsent: null, unsentError: null })
    const e3 = `unsent_count_failed: ${copy.E3_UNSENT_COUNT_FAILED}`
    expect(settledFrom({ ...base, account_mismatch: { pending_changes: 2, unsent_error: e3 } } as any)).toEqual({ kind: 'account_mismatch', pendingChanges: 2, unsent: null, unsentError: e3 })
  })

  /** §5.3 row 1 (plan review M8): a count that failed is E3, a line in the step, never a toast. "Sign out and switch"
   *  then counts again through the gate; a re-count that fails is the same line. */
  test('a count that failed shows E3 as a line in the step, and a fresh count replaces it', async () => {
    const e3 = `unsent_count_failed: ${copy.E3_UNSENT_COUNT_FAILED}`
    let counts = 0
    const m = switchStep({ pendingChanges: 2, unsent: null, unsentError: e3 }, {
      clear_session: () => { throw new Error('unsent_changes: Some changes…') },
      sign_out_count: () => {
        counts += 1
        if (counts === 1) throw new Error(e3)
        return { ...unsent({ askId: 'switch-3' }), canSend: false, notSending: null }
      },
    })
    await m.flush()
    expect(shown(m)).toContain(copy.E3_UNSENT_COUNT_FAILED)
    expect(shown(m)).not.toContain('unsent_count_failed')
    expect(shown(m)).toContain(switchCopy.ACCOUNT_SWITCH_TITLE)
    await m.click(copy.ASK_SWITCH)
    await m.flush()
    expect(m.toasts).toEqual([])
    expect(shown(m)).toContain(copy.E3_UNSENT_COUNT_FAILED) // the re-count failed too: still the line
    await m.click(copy.ASK_SWITCH)
    await m.flush()
    expect(m.toasts).toEqual([])
    expect(shown(m)).toContain(copy.A0S_TITLE)
    expect(shown(m)).not.toContain(copy.E3_UNSENT_COUNT_FAILED)
    expect(m.calls.filter((c) => c.name === 'clear_session').map((c) => c.args)).toEqual([{ forgetEmail: true }, { forgetEmail: true }])
  })

  /** F4. */
  test('the compact page opens the flow on a refusal, never a toast, and never keeps by itself', async () => {
    // The page's harness, as `tests/accountSessionWarnings.test.tsx` mounts it (`openAccountPage`).
    const m = mount('pages/Account.tsx', 'Account', {
      backend: {
        sync_status: () => ({ logged_in: true, vault_unlocked: true, engine: 'running', sync_root: null, syncing: 0, cloud_only: 0, conflicts: 0, engine_refusal: null }),
        autostart_enabled: () => false,
        account_email: () => 'sam@beebeeb.io',
        account_subscription: () => { throw new Error('not in this test') },
        clear_session: () => { throw new Error('unsent_changes: Some changes…') },
        request_sign_out_flow: () => null,
      },
      bindings: { ...accountSession, ...desktopApi, ...diagnosticsCopy, ...planPresentation, ...copy, WEB_APP_URL: 'https://app.beebeeb.io', useRegionLabel: () => 'Stored in the EU' },
    })
    mounted.push(m)
    for (let i = 0; i < 4; i++) await m.flush()
    await m.click('Sign out')
    expect(m.calls.map((c) => c.name)).toContain('request_sign_out_flow')
    expect(m.toasts).toEqual([])
    expect(m.calls.find((c) => c.name === 'clear_session')?.args?.keepUnsent).toBeUndefined()
  })

  /** F12, the compact page: a failure is shown, then acknowledged once. */
  test('the compact page acknowledges a failure it showed', async () => {
    const m = mount('pages/Account.tsx', 'Account', {
      backend: {
        sync_status: () => ({ logged_in: true, vault_unlocked: true, engine: 'running', sync_root: null, syncing: 0, cloud_only: 0, conflicts: 0, engine_refusal: null }),
        autostart_enabled: () => false,
        account_email: () => 'sam@beebeeb.io',
        account_subscription: () => { throw new Error('not in this test') },
        clear_session: () => { throw new Error('Sign-out paused: x') },
        sign_out_result_ack: () => null,
      },
      bindings: { ...accountSession, ...desktopApi, ...diagnosticsCopy, ...planPresentation, ...copy, WEB_APP_URL: 'https://app.beebeeb.io', useRegionLabel: () => 'Stored in the EU' },
    })
    mounted.push(m)
    for (let i = 0; i < 4; i++) await m.flush()
    await m.click('Sign out')
    for (let i = 0; i < 4; i++) await m.flush()
    expect(m.toasts.map((t) => t.title)).toEqual(['Couldn’t sign out'])
    const names = m.calls.map((c) => c.name)
    expect(names.filter((name) => name === 'sign_out_result_ack')).toHaveLength(1)
    expect(names.indexOf('sign_out_result_ack')).toBeGreaterThan(names.indexOf('clear_session'))
  })

  /** F4's last clause. `tests/forceReauth.test.ts` (`@184:tests/forceReauth.test.ts:125-160`) already pins the same
   *  for R8; this states it beside the other entry points. */
  test('on macOS "Sign in again" never clears the session', async () => {
    const calls: string[] = []
    const result = await desktopApi.forceReauth({
      platform: async () => ({ ok: true, value: 'macos' }),
      clearSession: async () => { calls.push('clear'); return { ok: true, value: { warning: null } } },
      openOnboardingWindow: async () => ({ ok: true, value: undefined }),
      openReauthWindow: async () => { calls.push('reauth'); return { ok: true, value: undefined } },
    })
    expect(result.ok).toBe(true)
    expect(calls).toEqual(['reauth'])
  })
})
