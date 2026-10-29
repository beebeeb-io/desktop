/**
 * Words the first-run sign-in screen uses around the browser handoff (task 1734).
 *
 * The approval page no longer shows a code to "match" and the link it opens
 * carries no code: the person TYPES the code this app shows. A link someone
 * else sends has nothing in it to approve - that is the whole point - so these
 * words must tell people to type the code, and to approve only a sign-in they
 * started themselves.
 */

export const BROWSER_SIGN_IN_INTRO =
  'Sign in through your browser. Beebeeb opens a Beebeeb tab, you type the code shown below into it ' +
  'and confirm with your password, and your session and key are handed back encrypted - ' +
  'end-to-end, never touching this app in the clear.'

/** What to do once the browser has opened, above the code. */
export function browserWaitingInstruction(phase: 'waiting' | 'authorized'): string {
  return phase === 'authorized'
    ? 'Decrypting your credentials and starting sync…'
    : 'Type this code on the page we just opened in your browser. ' +
        'Only approve it if you started this sign-in yourself, just now:'
}

/** Shown when the browser did not open on its own. */
export const BROWSER_DID_NOT_OPEN_HINT = 'in any signed-in browser, then type the code there.'
