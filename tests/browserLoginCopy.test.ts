import { describe, expect, test } from 'bun:test'

import {
  BROWSER_DID_NOT_OPEN_HINT,
  BROWSER_SIGN_IN_INTRO,
  browserWaitingInstruction,
} from '../src/browserLoginCopy'

// Task 1734 (security review 2026-10-04, finding 9): the approval page no
// longer shows a code to "match" and its link carries no code. The words around
// the handoff must tell people to TYPE the code, and to approve only a sign-in
// they started - never to compare or "confirm" a code somebody else put there.

describe('browser sign-in copy', () => {
  test('the waiting instruction tells the person to type the code and to approve only their own sign-in', () => {
    const text = browserWaitingInstruction('waiting')
    expect(text).toContain('Type this code')
    expect(text).toContain('started this sign-in yourself')
  })

  test('nothing tells the person to compare or confirm a code the page showed them', () => {
    for (const text of [BROWSER_SIGN_IN_INTRO, browserWaitingInstruction('waiting'), BROWSER_DID_NOT_OPEN_HINT]) {
      expect(text.toLowerCase()).not.toContain('matches')
      expect(text.toLowerCase()).not.toContain('confirm the code')
      expect(text.toLowerCase()).not.toContain('check that the code')
    }
  })

  test('the intro describes what really happens: type the code, confirm with the password', () => {
    expect(BROWSER_SIGN_IN_INTRO).toContain('type the code')
    expect(BROWSER_SIGN_IN_INTRO).toContain('password')
    // There is no refresh-token grant (see browser_login.rs): do not promise one.
    expect(BROWSER_SIGN_IN_INTRO.toLowerCase()).not.toContain('refresh')
  })

  test('the hint for a browser that did not open says to type the code there', () => {
    expect(BROWSER_DID_NOT_OPEN_HINT).toContain('type the code')
  })

  test('after approval the screen only reports progress', () => {
    expect(browserWaitingInstruction('authorized')).toContain('Decrypting')
  })
})
