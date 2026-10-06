/**
 * R8 (spec 2026-10-06): the one warning an account switch shows. Drawn first in
 * design/hifi/macos-settings-dialogs.html (Task 13); tests/reauthInPlace.test.tsx holds both to the
 * same strings. Honest about the loss: the changes that have not uploaded are removed.
 */
export const ACCOUNT_SWITCH_TITLE = 'Switch to a different account?'
export const ACCOUNT_SWITCH_CONFIRM = 'Sign out and switch'
export const ACCOUNT_SWITCH_CANCEL = 'Cancel'
export const ACCOUNT_SWITCH_FAILED = 'Couldn’t sign out'

export function accountSwitchBody(pendingChanges: number): string {
  if (pendingChanges <= 0) {
    return 'This Mac is signed in to another Beebeeb account. Switching signs that account out of this Mac.'
  }
  if (pendingChanges === 1) {
    return 'This Mac is signed in to another Beebeeb account, and 1 change on this Mac hasn’t uploaded yet. Switching signs that account out of this Mac and removes it.'
  }
  return `This Mac is signed in to another Beebeeb account, and ${pendingChanges} changes on this Mac haven’t uploaded yet. Switching signs that account out of this Mac and removes them.`
}
