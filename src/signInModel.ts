/**
 * Which screen the signed-out Beebeeb window shows, and how a sign-in moves it on (task 1683 slice 2; spec
 * `docs/specs/2026-10-24-macos-two-surface-app.md` §7.4, §7.5, §8.2; spec C `…-first-run-sign-in.md` §7.1, §7.2).
 *
 * The signed-out window is one centred card at a time. Which card follows from the session (`popover_snapshot`'s
 * account: no session, an ended one, a live one) and from the FLOW: the steps a sign-in walks (the account switch,
 * set up this Mac, the Finder status) that the session alone cannot tell, because the session exists before they are
 * done. This file is Onboarding's `afterSignIn` and its siblings, moved into pure functions with no React and no Tauri.
 *
 * The flow is kept in a module-level store (`signInFlowStore.ts`), not in component state: `AccountSessionBoundary`
 * remounts the window on every session change, and a sign-in is one. Onboarding avoided that by being the one window
 * outside the boundary; the Beebeeb window is inside it because 1887's sign-out relies on the remount.
 */
import type { SignInSettled } from './onboardingSignIn'
import type { AskCounts } from './signOutCopy'

/**
 * The steps after (or beside) the sign-in screen. `null` is no step: the window follows the session alone.
 * `settling`: the sign-in is done and the next read of the session has not landed, so the old card is not drawn again.
 */
export type SignInStep = 'switch' | 'unlock' | 'finder' | 'settling'

/** What the account switch step shows (1887 W11): the count, the ask snapshot, or the coded E3 when the count failed. */
export interface SwitchInfo {
  pendingChanges: number
  unsent: AskCounts | null
  unsentError: string | null
}

export interface SignInFlow {
  step: SignInStep | null
  /** Set only on the switch step. */
  switch: SwitchInfo | null
  /** FB-I1: the kept vault key was replaced, so the recovery step says why it is asked. */
  keyReplaced: boolean
  /**
   * FT-I4: the address typed in the sign-in that turned out to be another account. After “Sign out and switch” the
   * next sign-in opens with it, so the person who confirmed account B never sees A's address.
   */
  email: string | undefined
}

export const IDLE_FLOW: SignInFlow = { step: null, switch: null, keyReplaced: false, email: undefined }

/** Which card the window draws. `home` is not a card: the window shows its sidebar and Home. */
export type WindowScreen = 'home' | 'signin' | 'reauth' | 'switch' | 'unlock' | 'finder' | 'settling'

/** The part of the account the routing reads (`popover_snapshot`'s `account`, i.e. today's `sync_status`). */
export interface RoutingAccount {
  logged_in: boolean
  auth_expired: boolean
}

export const settling = (): SignInFlow => ({ ...IDLE_FLOW, step: 'settling' })

/**
 * The screen for a state. An explicit step beats the session where the session is simply not caught up (a settling
 * sign-in) or cannot tell (the switch: another account is waiting while this Mac still holds the first). A step that
 * needs a session is dropped when the session is gone, which is Onboarding's “window shown again” recheck: a window
 * brought back after a sign-out starts at sign-in, not on a Finder screen of an account that left.
 */
export function windowScreen(account: RoutingAccount, flow: SignInFlow): WindowScreen {
  if (flow.step === 'settling') return 'settling'
  if (flow.step === 'switch') return 'switch'
  if (account.logged_in && (flow.step === 'unlock' || flow.step === 'finder')) return flow.step
  if (!account.logged_in) return 'signin'
  return account.auth_expired ? 'reauth' : 'home'
}

/** The flow with every step that needs a session removed when there is none. Same object when nothing changes. */
export function normalizeFlow(account: RoutingAccount, flow: SignInFlow): SignInFlow {
  if (!account.logged_in && (flow.step === 'unlock' || flow.step === 'finder')) return IDLE_FLOW
  return flow
}

/**
 * Where a completed sign-in leads (R8, spec C C-S1). `reauth` is “Sign in again”: the account is already on this
 * Mac, so a sign-in that delivered the keys is the same account (a different one is a mismatch), and there is
 * nothing to set up: Finder is kept. `email` is the address that was typed (the password path), '' for the browser.
 */
export function afterSignIn(settled: SignInSettled, email: string, reauth: boolean): SignInFlow {
  switch (settled.kind) {
    case 'account_mismatch':
      return {
        ...IDLE_FLOW,
        step: 'switch',
        switch: { pendingChanges: settled.pendingChanges, unsent: settled.unsent, unsentError: settled.unsentError },
        email,
      }
    case 'reauthenticated':
      // The same account signed in again in place; with its keys here, sync resumes and nothing else is set up.
      if (settled.vaultUnlocked) return settling()
      return { ...IDLE_FLOW, step: 'unlock', keyReplaced: settled.keyReplaced }
    case 'fresh':
      // C-S1: a fresh sign-in that delivered the keys (the browser path) never lands on the recovery phrase.
      if (!settled.vaultUnlocked) return { ...IDLE_FLOW, step: 'unlock' }
      return reauth ? settling() : { ...IDLE_FLOW, step: 'finder' }
  }
}

/** “Sign out and switch” signed the other account out: sign-in again, with the address that was typed. */
export function afterSwitched(flow: SignInFlow): SignInFlow {
  return { ...IDLE_FLOW, email: flow.email }
}

/** Cancel: the first account is untouched, and nothing of the second is kept. */
export function afterSwitchCancelled(): SignInFlow {
  return IDLE_FLOW
}

/** The recovery phrase restored the keys: Finder is next. */
export function afterUnlocked(): SignInFlow {
  return { ...IDLE_FLOW, step: 'finder' }
}

/** Finder is ready: the window moves on to Home. It never closes itself (D13). */
export function afterFinderReady(): SignInFlow {
  return IDLE_FLOW
}
