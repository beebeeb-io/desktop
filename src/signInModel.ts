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
import { SET_UP_COPY } from './accountViewCopy'
import type { MainNav } from './mainWindowModel'
import type { SignInSettled } from './onboardingSignIn'
import type { AskCounts } from './signOutCopy'

/**
 * The steps after (or beside) the sign-in screen. `null` is no step: the window follows the session alone.
 * `settling`: the sign-in is done and the next read of the session has not landed, so the old card is not drawn again.
 * `reauth`: “Sign in again” was asked for by an opener (`open_reauth_window`), whatever the session says: the person
 * clicked it, so the window shows it, as Linux's onboarding window does (`mode=reauth`).
 */
export type SignInStep = 'switch' | 'unlock' | 'finder' | 'settling' | 'reauth'

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
 * The screen for a state. A step is the screen, whatever the session says: the window's snapshot is read every five
 * seconds, and a sign-in lands in a window that still holds the one it read BEFORE the sign-in, so “signed out” on that
 * snapshot is not evidence against the step the sign-in has just set (it was: a rule that judged the step against it
 * threw the next screen away, and showed the card the person had just signed in on again). The switch is a screen of an
 * account that is still on this Mac while another waits; the others follow a session that is there.
 */
export function windowScreen(account: RoutingAccount, flow: SignInFlow): WindowScreen {
  if (flow.step !== null) return flow.step
  if (!account.logged_in) return 'signin'
  return account.auth_expired ? 'reauth' : 'home'
}

/**
 * The flow with every step that needs a session removed when there is none (Onboarding's “window shown again after a
 * sign-out” recheck). Same object when nothing changes. It is applied ONCE, to the first snapshot a window reads when it
 * mounts, and never to a later one: a sign-out moves the session revision, which remounts the window
 * (`AccountSessionBoundary`), so the window that shows a flow of an account that left is always a fresh one; a snapshot
 * read by a window that has been up for a while may predate the step it would judge.
 */
export function normalizeFlow(account: RoutingAccount, flow: SignInFlow): SignInFlow {
  if (account.logged_in) return flow
  if (flow.step === 'unlock' || flow.step === 'finder' || flow.step === 'reauth') return IDLE_FLOW
  // The switch card asks about the account that is still on this Mac. When that account signed out another way (the app
  // menu, the popover) the card is stale; it goes, and the address typed for the next sign-in stays, as when “Sign out
  // and switch” ends the same way (review P2-2).
  if (flow.step === 'switch') return afterSwitched(flow)
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

/**
 * A request to the window that is a step of this flow: “Sign in again” from `open_reauth_window` (review P3-1). It shows the
 * card whatever the session says, but only for a signed-in window with no step in progress: the switch question, the
 * recovery phrase, Finder and settling are not replaced, and with no account on this Mac the sign-in card shows already.
 * Every other request is the window's navigation (`mainWindowModel.navigate`), not a step.
 */
export function flowForNav(nav: MainNav, signedIn: boolean, flow: SignInFlow): SignInFlow {
  if (nav.to !== 'reauth' || !signedIn || flow.step !== null) return flow
  return { ...IDLE_FLOW, step: 'reauth' }
}

/**
 * Whether a window that has just read its first account state should ask Rust for a pending set-up step (review P2-3):
 * the account is signed in with a good session, the vault is locked, and no step is already in progress. Anything else
 * has its own screen, or nothing to resume.
 */
export function setupCheckWanted(account: RoutingAccount & { vault_unlocked: boolean }, flow: SignInFlow): boolean {
  return account.logged_in && !account.auth_expired && !account.vault_unlocked && flow.step === null
}

/**
 * The flow a pending set-up step resumes at: Set up this Mac, with the reason line when the kept key was replaced.
 * `raw` is what `pending_setup` answered; anything but `{ key_replaced: boolean }` resumes nothing (null), because a
 * guess would put the person on a card with no way out.
 */
export function flowForPendingSetup(raw: unknown): SignInFlow | null {
  if (typeof raw !== 'object' || raw === null) return null
  const keyReplaced = (raw as { key_replaced?: unknown }).key_replaced
  if (typeof keyReplaced !== 'boolean') return null
  return { ...IDLE_FLOW, step: 'unlock', keyReplaced }
}

/**
 * `unlock_vault`'s answer on a Mac that holds a session token and no vault key: Rust says the approved Set up this Mac
 * sentence (`NO_VAULT_KEY_UNLOCK_ERROR`, pinned equal in `lib.rs`'s tests), instead of the Keychain's own words. The window
 * recognises it to offer the recovery phrase.
 */
export function isNoVaultKey(reason: string): boolean {
  return reason === SET_UP_COPY
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
