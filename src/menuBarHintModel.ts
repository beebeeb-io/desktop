/**
 * The one-time hint at the top of Home: "Beebeeb lives in the menu bar" (two-surface spec §7.6; hi-fi section 3, the
 * `.hint` card). It answers "too many windows": the b is home, the window is optional. Shown until "Got it", once per
 * Mac, stored in `desktop.toml` (`menu_bar_hint_seen`). The words are the spec's, byte for byte.
 */

export const MENU_BAR_HINT_TITLE = 'Beebeeb lives in the menu bar'
export const MENU_BAR_HINT_BODY = 'Click the b at the top of your screen to see what’s syncing. You can close this window; Beebeeb keeps running.'
export const MENU_BAR_HINT_DISMISS = 'Got it'

/** What the page knows about the hint: not read yet, to be shown, or never to be shown. */
export type MenuBarHintState = 'unknown' | 'show' | 'hide'

// Home is mounted again on every visit (Review, Trash, Settings and back), so "Got it" is also remembered here, for the life
// of the window, whatever the save did: a save that failed must not bring the hint back in the same run.
let dismissedThisRun = false
export const hintDismissedThisRun = (): boolean => dismissedThisRun
export function rememberHintDismissed(): void {
  dismissedThisRun = true
}
/** Tests only. */
export function forgetHintDismissedThisRun(): void {
  dismissedThisRun = false
}

/**
 * Shown only when the Mac says it has not been dismissed and this run has not dismissed it. Not read yet shows nothing (the
 * hint never flashes in and out), and so does a read that failed: a tip is not worth an error on Home, and the next launch
 * asks again.
 */
export function menuBarHintState(read: { ok: true; value: boolean } | { ok: false } | null, dismissedThisRun = false): MenuBarHintState {
  if (dismissedThisRun) return 'hide'
  if (read === null) return 'unknown'
  if (!read.ok) return 'hide'
  return read.value ? 'hide' : 'show'
}
