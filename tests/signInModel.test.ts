/**
 * Which screen the signed-out Beebeeb window shows, and how a sign-in moves it on (task 1683 slice 2; spec
 * `docs/specs/2026-10-24-macos-two-surface-app.md` §7.4, §7.5, §8.2; spec C `…-first-run-sign-in.md` §7.1, §7.2).
 *
 * The model is pure: `windowScreen` is the "screen routing per state" the slice names, and the `after*` functions are
 * Onboarding's `afterSignIn` and its siblings, which moved into the window. The flow lives in a module-level store
 * (`signInFlowStore.ts`) because `AccountSessionBoundary` remounts the window on every session change; the store is
 * tested at the end of this file.
 */
import { afterEach, describe, expect, test } from 'bun:test'
import {
  IDLE_FLOW,
  afterFinderReady,
  afterSignIn,
  afterSwitchCancelled,
  afterSwitched,
  afterUnlocked,
  normalizeFlow,
  settling,
  windowScreen,
  type SignInFlow,
} from '../src/signInModel'
import { getSignInFlow, resetSignInFlow, setSignInFlow, subscribeSignInFlow } from '../src/signInFlowStore'
import type { AskCounts } from '../src/signOutCopy'

const out = { logged_in: false, auth_expired: false }
const ended = { logged_in: true, auth_expired: true }
const live = { logged_in: true, auth_expired: false }
const ask: AskCounts = { askId: 'a1', files: 2, copiedFiles: 2, others: 1, missingFiles: 0, earlierFolders: [] }
const step = (s: SignInFlow['step'], over: Partial<SignInFlow> = {}): SignInFlow => ({ ...IDLE_FLOW, step: s, ...over })

describe('windowScreen: the screen per session state', () => {
  test('no session is the sign-in screen', () => {
    expect(windowScreen(out, IDLE_FLOW)).toBe('signin')
  })

  test('a signed-in account whose session ended is "Sign in again" (R8, C-R2): same screen, other words', () => {
    expect(windowScreen(ended, IDLE_FLOW)).toBe('reauth')
  })

  test('a live session with no flow is Home', () => {
    expect(windowScreen(live, IDLE_FLOW)).toBe('home')
  })

  test('the account switch is a screen whatever the session says: another account is waiting on this Mac (1887 W11)', () => {
    for (const account of [out, ended, live]) expect(windowScreen(account, step('switch'))).toBe('switch')
  })

  test('set up this Mac and the Finder screen belong to a live session, and take over from Home', () => {
    expect(windowScreen(live, step('unlock'))).toBe('unlock')
    expect(windowScreen(live, step('finder'))).toBe('finder')
  })

  test('…and also while the snapshot still says the session ended (a same-account sign-in has not been read yet)', () => {
    expect(windowScreen(ended, step('unlock'))).toBe('unlock')
    expect(windowScreen(ended, step('finder'))).toBe('finder')
  })

  test('a step that needs a session is dropped when the session is gone (Onboarding’s “window shown again” recheck)', () => {
    expect(windowScreen(out, step('unlock'))).toBe('signin')
    expect(windowScreen(out, step('finder'))).toBe('signin')
  })

  test('settling is its own screen: the sign-in finished and the next read has not landed, so the old card is not drawn again', () => {
    for (const account of [out, ended, live]) expect(windowScreen(account, settling())).toBe('settling')
  })
})

describe('afterSignIn: Onboarding’s afterSignIn, in the window (R8, C-S1)', () => {
  test('another account is the switch step, with the count, the ask snapshot, the coded E3 and the typed address', () => {
    expect(afterSignIn({ kind: 'account_mismatch', pendingChanges: 3, unsent: ask, unsentError: null }, 'b@example.eu', false)).toEqual({
      step: 'switch',
      switch: { pendingChanges: 3, unsent: ask, unsentError: null },
      keyReplaced: false,
      email: 'b@example.eu',
    })
    const e3 = 'unsent_count_failed: the queue could not be read'
    expect(afterSignIn({ kind: 'account_mismatch', pendingChanges: 0, unsent: null, unsentError: e3 }, 'b@example.eu', true).switch).toEqual({
      pendingChanges: 0,
      unsent: null,
      unsentError: e3,
    })
  })

  test('the same account with its keys here is done: nothing to set up, the window waits for the next read and shows Home', () => {
    expect(afterSignIn({ kind: 'reauthenticated', vaultUnlocked: true, keyReplaced: false }, 'a@example.eu', true)).toEqual(settling())
  })

  test('the same account whose kept key was replaced goes to set up this Mac, and says why (FB-I1)', () => {
    expect(afterSignIn({ kind: 'reauthenticated', vaultUnlocked: false, keyReplaced: true }, 'a@example.eu', true)).toEqual(
      step('unlock', { keyReplaced: true }),
    )
    expect(afterSignIn({ kind: 'reauthenticated', vaultUnlocked: false, keyReplaced: false }, 'a@example.eu', true).keyReplaced).toBe(false)
  })

  test('a fresh sign-in with no key on this Mac asks for the recovery phrase', () => {
    expect(afterSignIn({ kind: 'fresh', vaultUnlocked: false }, 'a@example.eu', false)).toEqual(step('unlock'))
  })

  test('C-S1: a fresh sign-in that delivered the keys (the browser path) never lands on the recovery phrase: Finder', () => {
    expect(afterSignIn({ kind: 'fresh', vaultUnlocked: true }, 'a@example.eu', false)).toEqual(step('finder'))
  })

  test('the browser path in "Sign in again" is the same account (a different one is a mismatch), so it is done, not a first-run Finder screen', () => {
    expect(afterSignIn({ kind: 'fresh', vaultUnlocked: true }, 'a@example.eu', true)).toEqual(settling())
  })
})

describe('the other steps', () => {
  test('"Sign out and switch" returns to sign-in with the address that was typed (FT-I4), not the old account’s', () => {
    const flow = step('switch', { email: 'b@example.eu', switch: { pendingChanges: 1, unsent: null, unsentError: null } })
    expect(afterSwitched(flow)).toEqual({ ...IDLE_FLOW, email: 'b@example.eu' })
  })

  test('Cancel returns to sign-in and keeps nothing of the other account: no prefill, no count', () => {
    expect(afterSwitchCancelled()).toEqual(IDLE_FLOW)
  })

  test('the recovery phrase leads to the Finder screen', () => {
    expect(afterUnlocked()).toEqual(step('finder'))
  })

  test('Finder ready ends the flow: Home (the window never closes itself, D13)', () => {
    expect(afterFinderReady()).toEqual(IDLE_FLOW)
  })

  test('normalizeFlow drops a step that needs a session when there is none, and keeps the rest', () => {
    expect(normalizeFlow(out, step('unlock'))).toEqual(IDLE_FLOW)
    expect(normalizeFlow(out, step('finder'))).toEqual(IDLE_FLOW)
    const sw = step('switch', { email: 'b@example.eu' })
    expect(normalizeFlow(out, sw)).toBe(sw)
    expect(normalizeFlow(live, step('unlock'))).toEqual(step('unlock'))
    expect(normalizeFlow(out, IDLE_FLOW)).toBe(IDLE_FLOW)
  })
})

describe('the flow store: it outlives AccountSessionBoundary’s remount', () => {
  afterEach(() => resetSignInFlow())

  test('a flow set while one tree is mounted is read by the next tree', () => {
    expect(getSignInFlow()).toBe(IDLE_FLOW)
    setSignInFlow(step('unlock', { keyReplaced: true }))
    // A remount is a new subscriber reading the same module state.
    expect(getSignInFlow()).toEqual(step('unlock', { keyReplaced: true }))
  })

  test('subscribers hear every change once, and a stopped subscriber hears none', () => {
    let heard = 0
    const stop = subscribeSignInFlow(() => { heard += 1 })
    setSignInFlow(step('finder'))
    setSignInFlow(IDLE_FLOW)
    expect(heard).toBe(2)
    stop()
    setSignInFlow(step('unlock'))
    expect(heard).toBe(2)
  })

  test('setting the flow it already holds tells nobody (useSyncExternalStore would loop on a new object each time)', () => {
    const flow = step('finder')
    setSignInFlow(flow)
    let heard = 0
    subscribeSignInFlow(() => { heard += 1 })
    setSignInFlow(flow)
    expect(heard).toBe(0)
  })

  test('the snapshot is one stable object between changes', () => {
    setSignInFlow(step('unlock'))
    expect(getSignInFlow()).toBe(getSignInFlow())
  })
})
