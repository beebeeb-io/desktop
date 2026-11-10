// What a changed password reports (task 2439 item 2, DW-A67). The server has already committed
// when `account_change_password` answers, so a failure to store the new sign-in or to restart the
// engine is said under "Password changed", never as a failed change. Pure, so the test needs no DOM.

/** The DTO `account_change_password` answers once the server has committed. No key, token or code. */
export interface PasswordChangeResult {
  status: 'changed'
  /** False when the new token could not be stored: the old one is dead, so this PC signs in again. */
  signedIn: boolean
  /** Whether the sync engine came back with the new token. */
  restart: 'ok' | 'failed'
}

export interface PasswordChangeWords {
  /** Always "Password changed": the server committed before this answer. */
  headline: string
  /** The line under it, or null when nothing followed. */
  detail: string | null
}

export const PASSWORD_CHANGED = 'Password changed.'
export const RESTART_FAILED = 'Sync will start again when you unlock.'
export const SIGN_IN_AGAIN = 'Sign in again on this PC.'

export function passwordChangeWords(value: PasswordChangeResult): PasswordChangeWords {
  // A dead sign-in is the bigger fact: the engine cannot run without it, so say that line only.
  if (!value.signedIn) return { headline: PASSWORD_CHANGED, detail: SIGN_IN_AGAIN }
  if (value.restart === 'failed') return { headline: PASSWORD_CHANGED, detail: RESTART_FAILED }
  return { headline: PASSWORD_CHANGED, detail: null }
}
