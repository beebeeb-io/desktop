/**
 * R8 copy (spec 2026-10-06): everything the re-sign-in flow says that is new. The account-switch
 * warning is drawn first in design/hifi/macos-settings-dialogs.html §4 (Task 13, amended for the
 * onboarding-window Card variant); tests/reauthInPlace.test.tsx holds both to the same strings. Honest
 * about the loss: the changes that have not uploaded are removed.
 */
export const ACCOUNT_SWITCH_TITLE = 'Switch to a different account?'
export const ACCOUNT_SWITCH_CONFIRM = 'Sign out and switch'
export const ACCOUNT_SWITCH_CANCEL = 'Cancel'
export const ACCOUNT_SWITCH_FAILED = 'Couldn’t sign out'

/**
 * Shown on the sign-in form when a sign-in finished but its result could not be read (an unknown
 * shape from the backend; see `settledFrom` in onboardingSignIn.ts). Not drawn: one honest sentence
 * that claims nothing about what did or did not change.
 */
/**
 * The recovery step's opening line after a same-account re-sign-in whose kept vault key the server no
 * longer accepts (FB-I1, `LoginOutcome.key_replaced`): the key was removed, so the phrase is asked for,
 * and this says why. Wording for Guus on #114.
 */
export const KEY_REPLACED_RECOVERY_COPY = 'This account’s vault key was changed on another device. Enter your current recovery phrase to unlock.'

export const SIGN_IN_OUTCOME_UNREADABLE = 'Beebeeb couldn’t read the result of that sign-in. Try signing in again.'

export function accountSwitchBody(pendingChanges: number): string {
  if (pendingChanges <= 0) {
    return 'This Mac is signed in to another Beebeeb account. Switching signs that account out of this Mac.'
  }
  if (pendingChanges === 1) {
    return 'This Mac is signed in to another Beebeeb account, and 1 change on this Mac hasn’t uploaded yet. Switching signs that account out of this Mac and removes it.'
  }
  return `This Mac is signed in to another Beebeeb account, and ${pendingChanges} changes on this Mac haven’t uploaded yet. Switching signs that account out of this Mac and removes them.`
}
