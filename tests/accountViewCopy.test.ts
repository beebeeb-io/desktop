/**
 * The words of the signed-out Beebeeb window (task 1683 slice 2; spec `docs/specs/2026-10-24-macos-two-surface-app.md`
 * §7.4, §10.2 rows 36 and 44; spec C `docs/specs/2026-10-07-desktop-first-run-sign-in.md` §7.1, §7.2, §7.5 "Where").
 *
 * Every string is pinned, and the design artefact (`design/hifi/macos-two-surface.html`, section 2) is held to the same
 * strings, so a word changed in one place and not the other turns a test red (design first: the hi-fi and the rulings
 * win over the code).
 */
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import * as copy from '../src/accountViewCopy'
import * as browserCopy from '../src/browserLoginCopy'
import * as switchCopy from '../src/accountSwitchCopy'

const hifi = readFileSync(new URL('../design/hifi/macos-two-surface.html', import.meta.url), 'utf8')

describe('the sign-in screens, word for word (spec C §7.1, §7.2; spec §10.2 row 44)', () => {
  test('the sign-in card', () => {
    expect(copy.SIGN_IN_TITLE).toBe('Sign in to Beebeeb')
    expect(copy.SIGN_IN_WITH_BROWSER).toBe('Sign in with browser')
    expect(copy.USE_EMAIL_AND_PASSWORD).toBe('Use email and password')
    expect(copy.NEW_TO_BEEBEEB).toBe('New to Beebeeb?')
    expect(copy.CREATE_ACCOUNT).toBe('Create an account on beebeeb.io')
    expect(copy.QUIT_BEEBEEB).toBe('Quit Beebeeb')
  })

  test('the way back from the email form, which is new in the window (row 44)', () => {
    expect(copy.SIGN_IN_WITH_BROWSER_INSTEAD).toBe('Sign in with browser instead')
  })

  test('Sign in again: the title, the sentence, and the footer that names the account (R8, row 38, row 44)', () => {
    expect(copy.SIGN_IN_AGAIN_TITLE).toBe('Sign in again')
    expect(copy.SIGN_IN_AGAIN_COPY).toBe('Sync is paused until you sign in again. Your files stay in Finder.')
    expect(copy.signedInBeforeAs('sam@example.eu')).toBe('Signed in before as sam@example.eu')
  })

  test('offline: the line that replaces the buttons, and its one action (spec C §7.1)', () => {
    expect(copy.OFFLINE_LINE).toBe('Can’t reach Beebeeb. Check your connection.')
    expect(copy.TRY_AGAIN).toBe('Try again')
  })

  test('the browser code screen keeps task 1734’s words and adds only "Copy link"', () => {
    expect(copy.COPY_LINK).toBe('Copy link')
    expect(copy.COPIED_LINK).toBe('Copied')
    expect(copy.BROWSER_WAITING_BUTTON).toBe('Waiting for browser…')
    expect(copy.BROWSER_CONNECTING_BUTTON).toBe('Connecting…')
    expect(copy.BROWSER_FINISHING_BUTTON).toBe('Finishing…')
    // browserLoginCopy.ts is not edited: its three exports are the screen's instruction and hint.
    expect(browserCopy.browserWaitingInstruction('waiting')).toBe(
      'Type this code on the page we just opened in your browser. Only approve it if you started this sign-in yourself, just now:',
    )
    expect(browserCopy.BROWSER_DID_NOT_OPEN_HINT).toBe('in any signed-in browser, then type the code there.')
  })

  test('the email form and the second factor keep today’s words (spec §7.4 item 3: "today’s SignInStep, unchanged")', () => {
    expect(copy.PASSWORD_TITLE).toBe('Welcome back')
    expect(copy.PASSWORD_COPY).toBe('Sign in to unlock your encrypted vault.')
    expect(copy.TOTP_TITLE).toBe('Two-factor authentication')
    expect(copy.TOTP_COPY).toBe('Enter the 6-digit code from your authenticator app.')
    expect(copy.BACKUP_COPY).toBe('Enter one of the 8-digit backup codes you saved when enabling 2FA.')
  })

  test('set up this Mac and the Finder status screen (spec §7.4 item 6, §7.5)', () => {
    expect(copy.SET_UP_TITLE).toBe('Set up this Mac')
    expect(copy.SET_UP_COPY).toBe('This Mac does not have your encryption keys yet. Restore them to continue.')
    expect(copy.SET_UP_HINT).toBe(
      'Paste all 12 words into any box or type them one by one. Beebeeb stores the unlocked vault key in macOS Keychain for future unlocks.',
    )
    expect(copy.FINDER_CARD_TITLE).toBe('Beebeeb in Finder')
    expect(copy.FINDER_CARD_COPY).toBe('Beebeeb adds itself to Finder after sign-in.')
    // Spec §7.5 said no "Continue" on the Finder screen; the lead's ruling of 2026-10-25 (review P3-6) amends it: while Finder
    // is not Ready the card offers one secondary "Continue to Home", so a failing add does not hold the person on it.
    expect(copy.FINDER_CONTINUE_TO_HOME).toBe('Continue to Home')
    expect(Object.keys(copy).filter((key) => /continue/i.test(key)).sort()).toEqual(['FINDER_CONTINUE_TO_HOME', 'SET_UP_CONTINUE'])
  })

  test('the recovery-phrase offer on Home’s locked card, and the window that could not read the account (review P2-3, P3-4)', () => {
    expect(copy.USE_RECOVERY_PHRASE).toBe('Use your recovery phrase')
    expect(copy.LOAD_FAILED_TITLE).toBe('Couldn’t load your account')
    expect(copy.BACK_TO_HOME).toBe('Back to Home')
    expect(copy.SIGN_IN_AGAIN_REQUESTED_COPY).toBe('Beebeeb couldn’t send some changes with this sign-in. Sign in again to retry them.')
  })

  test('window titles follow the screen’s heading; signed in the window is "Beebeeb" (spec C C-W8, row 36)', () => {
    expect(copy.SIGNED_IN_WINDOW_TITLE).toBe('Beebeeb')
  })

  test('no developer word and no emoji in any string of the module (spec §10.1)', () => {
    const strings = Object.values(copy).filter((value): value is string => typeof value === 'string')
    expect(strings.length).toBeGreaterThan(20)
    for (const text of strings) {
      expect(text).not.toMatch(/daemon|hydrate|control center|namespace|telemetry|heartbeat|wired/i)
      expect(text).not.toMatch(/\p{Extended_Pictographic}/u)
    }
  })
})

describe('the design artefact draws the same strings (design first)', () => {
  const drawn = hifi.replace(/&rsquo;/g, '’')
  test('the sign-in card, the way back, and the footer', () => {
    for (const text of [copy.SIGN_IN_TITLE, copy.SIGN_IN_WITH_BROWSER, copy.USE_EMAIL_AND_PASSWORD, copy.SIGN_IN_WITH_BROWSER_INSTEAD, copy.NEW_TO_BEEBEEB, copy.CREATE_ACCOUNT, copy.QUIT_BEEBEEB]) {
      expect(drawn).toContain(text)
    }
  })
  test('Sign in again on a session that is fine, and its way back (lead copy ruling, 2026-10-25)', () => {
    expect(drawn).toContain(copy.SIGN_IN_AGAIN_REQUESTED_COPY)
    expect(drawn).toContain(copy.BACK_TO_HOME)
  })
  test('Sign in again, and the footer that names the account', () => {
    expect(drawn).toContain(copy.SIGN_IN_AGAIN_TITLE)
    expect(drawn).toContain(copy.SIGN_IN_AGAIN_COPY)
    expect(drawn).toContain('Signed in before as sam@example.eu')
  })
  test('the browser code screen, the email form, set up this Mac and Finder', () => {
    for (const text of [copy.COPY_LINK, copy.BROWSER_WAITING_BUTTON, copy.PASSWORD_TITLE, copy.PASSWORD_COPY, copy.SET_UP_TITLE, copy.SET_UP_COPY, copy.SET_UP_HINT, copy.FINDER_CARD_TITLE, copy.FINDER_CARD_COPY]) {
      expect(drawn).toContain(text)
    }
  })
  test('the offline line is in the figure’s caption', () => {
    expect(drawn).toContain(copy.OFFLINE_LINE)
  })
  test('the switch card keeps 1887’s title and buttons (copy unchanged)', () => {
    expect(drawn).toContain('Switch accounts with changes not sent?')
    expect(drawn).toContain(switchCopy.ACCOUNT_SWITCH_CANCEL)
    expect(drawn).toContain(switchCopy.ACCOUNT_SWITCH_CONFIRM)
  })
})
