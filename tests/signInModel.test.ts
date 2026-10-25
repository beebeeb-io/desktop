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
  canLeaveSignInAgain,
  flowForNav,
  flowForPendingSetup,
  isNoVaultKey,
  setupCheckWanted,
  afterUnlocked,
  normalizeFlow,
  settling,
  windowScreen,
  type SignInFlow,
} from '../src/signInModel'
import { getSignInFlow, resetSignInFlow, setSignInFlow, subscribeSignInFlow } from '../src/signInFlowStore'
import type { AskCounts } from '../src/signOutCopy'
import * as copy from '../src/accountViewCopy'
import { rustStr } from './fixtures/rustConstants'

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

  test('set up this Mac and the Finder screen take over from Home', () => {
    expect(windowScreen(live, step('unlock'))).toBe('unlock')
    expect(windowScreen(live, step('finder'))).toBe('finder')
  })

  // The bug the real-browser run found: a sign-in lands while the window still holds the snapshot it read BEFORE it
  // (up to five seconds old). The step the sign-in set must not be read against that snapshot, or the card the person
  // just signed in on flashes back, and a rule that dropped the step on it threw the next screen away.
  test('a step is the screen whatever the snapshot says: the snapshot is stale for a moment after a sign-in', () => {
    for (const account of [out, ended, live]) {
      expect(windowScreen(account, step('unlock'))).toBe('unlock')
      expect(windowScreen(account, step('finder'))).toBe('finder')
    }
  })

  test('settling is its own screen: the sign-in finished and the next read has not landed, so the old card is not drawn again', () => {
    for (const account of [out, ended, live]) expect(windowScreen(account, settling())).toBe('settling')
  })
})

/**
 * Task 1747 Part 2 (plan D4): a signed-in, locked Mac with no vault key anywhere opens on Set up this Mac BY STATE, not only
 * after a click. The route is what the Keychain says (`vault_unlock_route`); the model only reads it.
 */
describe('windowScreen: the Keychain route of a locked Mac (1747 Part 2)', () => {
  const locked = { logged_in: true, auth_expired: false, vault_unlocked: false }
  const unlocked = { logged_in: true, auth_expired: false, vault_unlocked: true }

  // Item 7, red first: today this is `home`, a Home whose locked card calls `unlock_vault`, which has no key to read.
  test('locked, no key anywhere (route recovery_phrase), idle flow: Set up this Mac, by state (the interrupted first run)', () => {
    expect(windowScreen(locked, IDLE_FLOW, 'recovery_phrase')).toBe('unlock')
  })

  // Item 8
  test('locked with the key in the Keychain (route keychain): Home, whose locked card unlocks in place', () => {
    expect(windowScreen(locked, IDLE_FLOW, 'keychain')).toBe('home')
  })

  // Item 9: no flash of Home while the question is out.
  test('locked and the route not known yet: settling, never Home', () => {
    expect(windowScreen(locked, IDLE_FLOW, 'unknown')).toBe('settling')
  })

  // Item 10
  test('a flow step still wins over the route (the switch, Finder, Set up this Mac, settling, Sign in again)', () => {
    for (const route of ['keychain', 'recovery_phrase', 'unknown'] as const) {
      for (const stepName of ['switch', 'finder', 'unlock', 'settling', 'reauth'] as const) {
        expect({ route, step: stepName, screen: windowScreen(locked, step(stepName), route) }).toEqual({ route, step: stepName, screen: stepName })
      }
    }
  })

  test('the route is about a locked, signed-in Mac with a good session and nothing else', () => {
    for (const route of ['keychain', 'recovery_phrase', 'unknown'] as const) {
      expect(windowScreen(unlocked, IDLE_FLOW, route)).toBe('home')
      expect(windowScreen(out, IDLE_FLOW, route)).toBe('signin')
      expect(windowScreen({ ...locked, auth_expired: true }, IDLE_FLOW, route)).toBe('reauth')
    }
  })

  test('an account read without a vault state (the routing tests above) is Home, whatever the route', () => {
    expect(windowScreen(live, IDLE_FLOW, 'recovery_phrase')).toBe('home')
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

  // Onboarding’s “window shown again after a sign-out” recheck, once, where it cannot be wrong: against the first
  // snapshot a window reads when it mounts. A sign-out moves the session revision, which remounts the window
  // (AccountSessionBoundary), so the window that shows a flow of an account that left is always a fresh one.
  test('normalizeFlow drops a step that needs a session when there is none, and keeps the rest', () => {
    expect(normalizeFlow(out, step('unlock'))).toEqual(IDLE_FLOW)
    expect(normalizeFlow(out, step('finder'))).toEqual(IDLE_FLOW)
    // “Sign in again” asked for by an opener (task 1683 fix round, P3-1) is a screen of an account on this Mac.
    expect(normalizeFlow(out, step('reauth'))).toEqual(IDLE_FLOW)
    expect(normalizeFlow(live, step('unlock'))).toEqual(step('unlock'))
    expect(normalizeFlow(live, step('reauth'))).toEqual(step('reauth'))
    expect(normalizeFlow(out, IDLE_FLOW)).toBe(IDLE_FLOW)
  })

  // Review P2-2: the switch card is a screen of the account that is still on this Mac while another waits. When that
  // account signed out another way (the app menu's Sign Out, the popover) the card is stale: its question has no
  // subject. It goes, and the address FT-I4 carries to the next sign-in stays (the same flow “Sign out and switch” ends
  // in), so the remount that a sign-out causes does not drop it either.
  test('normalizeFlow drops a stale switch card when the account left, keeping the typed address (P2-2)', () => {
    const sw = step('switch', { email: 'b@example.eu', switch: { pendingChanges: 3, unsent: ask, unsentError: null } })
    expect(normalizeFlow(out, sw)).toEqual({ ...IDLE_FLOW, email: 'b@example.eu' })
    expect(normalizeFlow(out, sw)).toEqual(afterSwitched(sw))
    // The account is still here: the question stands, and nothing is replaced.
    expect(normalizeFlow(live, sw)).toBe(sw)
    expect(normalizeFlow(ended, sw)).toBe(sw)
  })
})

// Review P3-1: `open_reauth_window` on a Mac asked for “Sign in again”, but the window only drew it when the session was
// already marked ended; with the session still good the click ended on Home and nothing happened. The request is a flow
// step now, so the card is shown whatever the session says (Linux's `mode=reauth`).
describe('flowForNav: a “Sign in again” request is the card, whatever the session says (P3-1)', () => {
  const idle = { signedIn: true, sheetOpen: false }

  test('a signed-in window with no step in progress shows the card', () => {
    expect(flowForNav({ to: 'reauth' }, idle, IDLE_FLOW)).toEqual({ flow: step('reauth'), held: false })
    expect(windowScreen(live, flowForNav({ to: 'reauth' }, idle, IDLE_FLOW).flow)).toBe('reauth')
  })

  test('no account on this Mac: the sign-in card is already what shows, and nothing is set or held', () => {
    expect(flowForNav({ to: 'reauth' }, { signedIn: false, sheetOpen: false }, IDLE_FLOW)).toEqual({ flow: IDLE_FLOW, held: false })
    expect(flowForNav({ to: 'reauth' }, { signedIn: false, sheetOpen: true }, IDLE_FLOW)).toEqual({ flow: IDLE_FLOW, held: false })
  })

  test('a step in progress is never replaced: the switch question, the phrase, Finder and settling stand', () => {
    for (const s of ['switch', 'unlock', 'finder', 'settling'] as const) {
      const flow = step(s)
      expect(flowForNav({ to: 'reauth' }, idle, flow)).toEqual({ flow, held: false })
    }
  })

  // Delta review P3-2: under a sheet nothing navigates (§8.2), and the card is a navigation. The request waits.
  test('under a sheet the request changes nothing and is held for when the sheet closes', () => {
    expect(flowForNav({ to: 'reauth' }, { signedIn: true, sheetOpen: true }, IDLE_FLOW)).toEqual({ flow: IDLE_FLOW, held: true })
  })

  test('every other request leaves the flow alone and holds nothing', () => {
    for (const nav of [{ to: 'home' }, { to: 'back' }, { to: 'settings', section: null }] as const) {
      expect(flowForNav(nav, idle, IDLE_FLOW)).toEqual({ flow: IDLE_FLOW, held: false })
      expect(flowForNav(nav, { signedIn: true, sheetOpen: true }, IDLE_FLOW)).toEqual({ flow: IDLE_FLOW, held: false })
    }
  })

  test('the sign-in it leads to ends the step like any “Sign in again”', () => {
    expect(afterSignIn({ kind: 'reauthenticated', vaultUnlocked: true, keyReplaced: false }, '', true)).toEqual(settling())
  })
})

// Delta review P3-2: the forced card has no Cancel. When the session is actually fine there is nothing to sign in again to,
// so the card offers a quiet way back; when the session really ended, it is the one screen that can help, and it does not.
describe('canLeaveSignInAgain (P3-2)', () => {
  test('only the requested card of a session that is fine can be left', () => {
    expect(canLeaveSignInAgain({ logged_in: true, auth_expired: false }, step('reauth'))).toBe(true)
    expect(canLeaveSignInAgain({ logged_in: true, auth_expired: true }, step('reauth'))).toBe(false)
    expect(canLeaveSignInAgain({ logged_in: true, auth_expired: true }, IDLE_FLOW)).toBe(false)
    expect(canLeaveSignInAgain({ logged_in: false, auth_expired: false }, step('reauth'))).toBe(false)
    for (const s of ['switch', 'unlock', 'finder', 'settling'] as const) expect(canLeaveSignInAgain(live, step(s))).toBe(false)
  })
})

// Review P2-3: ⌘W on “Set up this Mac” destroys the window and its flow store with it; the session stays, the key was
// never delivered, and the reopened window showed Home, whose locked card only called `unlock_vault` (nothing to read).
// Rust keeps the pending step for the life of the process (`pending_setup`); the window asks once when it opens locked.
describe('the pending set-up step: a window reopened while it was open (P2-3)', () => {
  const lockedIn = { logged_in: true, auth_expired: false, vault_unlocked: false }

  test('it is asked for exactly when an account is signed in, its session is good, the vault is locked, and no step is in progress', () => {
    expect(setupCheckWanted(lockedIn, IDLE_FLOW)).toBe(true)
    expect(setupCheckWanted({ ...lockedIn, vault_unlocked: true }, IDLE_FLOW)).toBe(false)
    expect(setupCheckWanted({ ...lockedIn, logged_in: false }, IDLE_FLOW)).toBe(false)
    expect(setupCheckWanted({ ...lockedIn, auth_expired: true }, IDLE_FLOW)).toBe(false)
    for (const s of ['switch', 'unlock', 'finder', 'settling', 'reauth'] as const) expect(setupCheckWanted(lockedIn, step(s))).toBe(false)
  })

  test('a pending step resumes at Set up this Mac, saying why when the kept key was replaced; none resumes nothing', () => {
    expect(flowForPendingSetup({ key_replaced: false })).toEqual(step('unlock'))
    expect(flowForPendingSetup({ key_replaced: true })).toEqual(step('unlock', { keyReplaced: true }))
    expect(flowForPendingSetup(null)).toBeNull()
  })

  test('an answer that is not the shape Rust sends resumes nothing: a wrong guess would trap the person on a card', () => {
    for (const raw of [undefined, 'yes', 1, [], {}, { key_replaced: 'yes' }]) expect(flowForPendingSetup(raw as never)).toBeNull()
  })
})

// The sentence `unlock_vault` answers on a Mac that holds a token and no vault key. It is the approved Set up this Mac
// copy, so every surface that shows an unlock failure says something true; the window recognises it to offer the phrase.
describe('isNoVaultKey (P2-3 b)', () => {
  // The two halves of one contract, pinned equal: what Rust's `unlock_vault` says, and what the window recognises.
  test('Rust’s sentence is the approved Set up this Mac copy, word for word', () => {
    expect(rustStr('lib.rs', 'NO_VAULT_KEY_UNLOCK_ERROR')).toBe(copy.SET_UP_COPY)
    expect(isNoVaultKey(rustStr('lib.rs', 'NO_VAULT_KEY_UNLOCK_ERROR'))).toBe(true)
  })

  test('only the Set up this Mac sentence is the no-key answer', () => {
    expect(isNoVaultKey('This Mac does not have your encryption keys yet. Restore them to continue.')).toBe(true)
    for (const reason of ['', 'Sign in before unlocking the vault.', 'unlock vault key from Keychain: secret not found', 'Account changed. Please try again.']) {
      expect(isNoVaultKey(reason)).toBe(false)
    }
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
