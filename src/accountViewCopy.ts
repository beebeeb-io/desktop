/**
 * The words of the signed-out Beebeeb window on a Mac (task 1683 slice 2; spec `docs/specs/2026-10-24-macos-two-surface-app.md`
 * §7.4, §7.5, §10.2 rows 36 and 44; spec C `docs/specs/2026-10-07-desktop-first-run-sign-in.md` §7.1 and §7.2).
 *
 * Spec C names this module for ALL of its screens (“All strings live in `src/accountViewCopy.ts`”). Slice 2 renders
 * today's `sync_status`, so only the strings of the screens it draws are here: sign-in, sign-in again, the email form,
 * the second factor, set up this Mac, and the Finder status. Task 1747 (the Rust account view) adds the update-required,
 * choose-a-plan and checking screens beside them.
 *
 * Words that already live elsewhere stay there and are imported, not copied: the browser handoff's instruction in
 * `browserLoginCopy.ts` (task 1734), the account switch in `accountSwitchCopy.ts` and `signOutCopy.ts` (1887), the Finder
 * sentences in `finderSetupCopy.ts` (spec A). `tests/accountViewCopy.test.ts` pins every string and holds the hi-fi
 * (`design/hifi/macos-two-surface.html`, section 2) to the same words. Apostrophes are the typographic ’.
 */

// ── Sign in (spec C C-R1) ──────────────────────────────────────────────────

export const SIGN_IN_TITLE = 'Sign in to Beebeeb'
export const SIGN_IN_WITH_BROWSER = 'Sign in with browser'
export const USE_EMAIL_AND_PASSWORD = 'Use email and password'
export const NEW_TO_BEEBEEB = 'New to Beebeeb?'
export const CREATE_ACCOUNT = 'Create an account on beebeeb.io'
export const QUIT_BEEBEEB = 'Quit Beebeeb'

/** The link back from the email form to the browser (new in the window, spec §10.2 row 44). */
export const SIGN_IN_WITH_BROWSER_INSTEAD = 'Sign in with browser instead'

// ── Sign in again (spec C C-R2, R8; spec §10.2 rows 38 and 44) ─────────────

export const SIGN_IN_AGAIN_TITLE = 'Sign in again'
export const SIGN_IN_AGAIN_COPY = 'Sync is paused until you sign in again. Your files stay in Finder.'
export const signedInBeforeAs = (email: string): string => `Signed in before as ${email}`

// ── Offline (spec C C-R7): the buttons are replaced by this line and one action ──

export const OFFLINE_LINE = 'Can’t reach Beebeeb. Check your connection.'
export const TRY_AGAIN = 'Try again'

// ── The browser code screen (task 1734; the instruction itself is `browserLoginCopy.ts`) ──

export const COPY_LINK = 'Copy link'
export const COPIED_LINK = 'Copied'
export const BROWSER_CONNECTING_BUTTON = 'Connecting…'
export const BROWSER_WAITING_BUTTON = 'Waiting for browser…'
export const BROWSER_FINISHING_BUTTON = 'Finishing…'
/** Above the address of the page the browser did not open on its own. */
export const BROWSER_DID_NOT_OPEN = 'Browser didn’t open? Visit'
/** The accessible name of the device code. */
export const SIGN_IN_CODE_LABEL = 'Sign-in code'

// ── The email form and the second factor: today's SignInStep, unchanged (spec §7.4 item 3) ──

export const PASSWORD_TITLE = 'Welcome back'
export const PASSWORD_COPY = 'Sign in to unlock your encrypted vault.'
export const EMAIL_LABEL = 'Email'
export const EMAIL_PLACEHOLDER = 'you@example.com'
export const PASSWORD_LABEL = 'Password'
export const PASSWORD_PLACEHOLDER = 'Your password'
export const SIGN_IN_BUTTON = 'Sign in'
export const SIGNING_IN_BUTTON = 'Signing in…'

export const TOTP_TITLE = 'Two-factor authentication'
export const TOTP_COPY = 'Enter the 6-digit code from your authenticator app.'
export const BACKUP_COPY = 'Enter one of the 8-digit backup codes you saved when enabling 2FA.'
export const TOTP_LABEL = 'Authentication code'
export const BACKUP_LABEL = 'Backup code'
export const VERIFY_BUTTON = 'Verify'
export const VERIFYING_BUTTON = 'Verifying…'
export const USE_BACKUP_CODE = 'Use backup code'
export const USE_AUTHENTICATOR_CODE = 'Use authenticator code instead'
export const BACK_BUTTON = '← Back'

// ── Set up this Mac: the recovery phrase (spec §7.4 item 6) ──────────────────

export const SET_UP_TITLE = 'Set up this Mac'
export const SET_UP_COPY = 'This Mac does not have your encryption keys yet. Restore them to continue.'
export const SET_UP_HINT =
  'Paste all 12 words into any box or type them one by one. Beebeeb stores the unlocked vault key in macOS Keychain for future unlocks.'
export const RECOVERY_PHRASE_LABEL = 'Recovery phrase'
export const UNLOCK_VAULT_BUTTON = 'Unlock vault'
export const UNLOCKING_BUTTON = 'Unlocking…'
/** Only beside a build that has no unlock command; a working unlock is never skipped. */
export const SET_UP_CONTINUE = 'Continue to Finder setup'
/**
 * Home's locked card, after “Unlock vault” found no key on this Mac (review P2-3 b): the sentence is `SET_UP_COPY`, and
 * this opens the same card as the sign-in does. Lead wording, 2026-10-25.
 */
export const USE_RECOVERY_PHRASE = 'Use your recovery phrase'

// ── The Finder status screen (spec §7.5; the sentences are spec A's, `finderSetupCopy.ts`) ──

export const FINDER_CARD_TITLE = 'Beebeeb in Finder'
/** The sentence the onboarding rail's macOS row used to say (`FINDER_RAIL_DETAIL`); the rail is gone from a Mac, the words stay. */
export const FINDER_CARD_COPY = 'Beebeeb adds itself to Finder after sign-in.'
/**
 * Review P3-6 (lead ruling, 2026-10-25): while Finder is not Ready the card offers a way on, so a Finder add that fails
 * (or an extension that is off) does not hold the person on a status screen. Home shows the Finder row's state.
 */
export const FINDER_CONTINUE_TO_HOME = 'Continue to Home'

// ── The window could not read the account (the settling screen, and the window before its first read) ──

export const LOAD_FAILED_TITLE = 'Couldn’t load your account'

// ── The window's title (spec C C-W8, spec §10.2 row 36) ──────────────────────

/** Signed out the title is the heading of the card shown; signed in it is the app's name. */
export const SIGNED_IN_WINDOW_TITLE = 'Beebeeb'
