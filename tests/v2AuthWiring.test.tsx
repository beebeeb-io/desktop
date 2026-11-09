/**
 * Desktop v2 M3b (B10, B11): wiring the map audit found missing or wrong, checked on the real component declarations.
 * - DW-A26: the sixth digit submits the code it completes. The view used to call `onSubmit()` with nothing, and the
 *   flow then sent the code from its last render (five digits), so typing a code always came back "isn't right".
 */
import { describe, expect, test } from 'bun:test'
import { createElement, Fragment } from 'react'
import { digitsOnly, totpEntry } from '../src/v2/auth/model'
import { elementsOf, expand, loadComponent } from './fixtures/componentHarness'

const pass = ({ children }: { children?: unknown }) => createElement(Fragment, null, children as never)
const AUTH_PARTS = {
  React: { createElement, Fragment },
  AuthWindow: pass, StepLockup: () => null, Heading: pass, Note: pass, TextLink: pass, PrimaryButton: pass,
  Field: (p: Record<string, unknown>) => createElement('input', p), SECRET_FIELD: {},
}

describe('DW-A26: the sixth digit submits the whole code', () => {
  // P1a (#271 review): the field reads its input through totpEntry (a pasted backup code goes to the backup field).
  const TwoFactorView = loadComponent('v2/auth/Steps.tsx', 'TwoFactorView', { ...AUTH_PARTS, useRef: () => ({ current: null }), digitsOnly, totpEntry })
  const totpInput = (props: Record<string, unknown>) =>
    elementsOf(expand(TwoFactorView(props))).find((e) => e.type === 'input' && e.props.id === 'totp')!

  test('typing the sixth digit hands the six digits to onSubmit', () => {
    const submitted: unknown[][] = []
    const typed: string[] = []
    // The flow's state still holds five digits when the sixth arrives: React has not re-rendered yet.
    const input = totpInput({ mode: 'totp', code: '48219', state: 'idle', onCode: (v: string) => typed.push(v), onSubmit: (...a: unknown[]) => submitted.push(a), onSwitch() {}, onBack() {} })
    input.props.onChange({ target: { value: '482193' } })
    expect(typed).toEqual(['482193'])
    expect(submitted).toEqual([['482193']])
  })

  test('five digits submit nothing; Enter on six submits them', () => {
    const submitted: unknown[][] = []
    const five = totpInput({ mode: 'totp', code: '4821', state: 'idle', onCode() {}, onSubmit: (...a: unknown[]) => submitted.push(a), onSwitch() {}, onBack() {} })
    five.props.onChange({ target: { value: '48219' } })
    expect(submitted).toEqual([])
    const six = totpInput({ mode: 'totp', code: '482193', state: 'idle', onCode() {}, onSubmit: (...a: unknown[]) => submitted.push(a), onSwitch() {}, onBack() {} })
    six.props.onKeyDown({ key: 'Enter' })
    expect(submitted).toEqual([['482193']])
  })
})

/**
 * - DW-A45: the recovery options are the board's three cards, each with its own button: "Recover with phrase" (the one
 *   amber action), "Sign in with another device", "Show my options".
 */
describe('DW-A45: the three option cards and their buttons', () => {
  const button = ({ children, onClick }: { children?: unknown; onClick?: () => void }) => createElement('button', { onClick }, children as never)
  const OptionsView = loadComponent('v2/auth/forgot/index.tsx', 'OptionsView', { ...AUTH_PARTS, Icon: () => null, PrimaryButton: button, SecondaryButton: button })

  test('each card has the board’s button, wired to its own path', () => {
    const calls: string[] = []
    const tree = expand(OptionsView({ onPhrase: () => calls.push('phrase'), onDevice: () => calls.push('device'), onNeither: () => calls.push('neither'), onBack: () => calls.push('back') }))
    const buttons = elementsOf(tree).filter((e) => e.type === 'button')
    expect(buttons.length).toBe(3)
    const find = (words: string) => buttons.find((b) => b.props.children === words)!
    for (const [words, call] of [['Recover with phrase', 'phrase'], ['Sign in with another device', 'device'], ['Show my options', 'neither']] as const) {
      find(words).props.onClick()
      expect(calls.at(-1)).toBe(call)
    }
    expect(calls).toEqual(['phrase', 'device', 'neither'])
  })
})

/** - DW-A13: one answer per request. A second Esc (or Esc and a click) while the deny is on its way sends nothing. */
describe('DW-A13: deny is sent once', () => {
  test('two denies in a row reach Rust once', async () => {
    const api = await import('../src/v2/auth/api')
    const model = await import('../src/v2/auth/model')
    const { mount } = await import('./fixtures/componentHarness')
    let dialog: Record<string, any> | null = null
    let release: (v: unknown) => void = () => {}
    const m = mount('v2/auth/Approve.tsx', 'ApproveSignIn', {
      backend: {
        auth_approve_lookup: () => ({ code: 'K7QM-4WTD', country: 'DE', ip: '203.0.113.42', requested_at: '', expires_at: '', seconds_left: 200, device_name: 'Anna’s iPhone', client: null, os: null }),
        auth_approve_request: () => new Promise((r) => { release = r }),
      },
      bindings: {
        approveLookup: api.approveLookup, approveRequest: api.approveRequest, approveErrorText: model.approveErrorText,
        ApproveCodePanel: (p: Record<string, any>) => createElement('button', { onClick: p.onContinue }, 'Continue'),
        ApproveDialog: (p: Record<string, any>) => { dialog = p; return null },
      },
      props: { onSeeSessions() {}, onDone() {} },
      expand: true,
    })
    try {
      await m.click('Continue')
      m.elements() // draws the tree, which hands the dialog its props
      expect(dialog!.stage).toBe('review')
      void dialog!.onDeny()
      void dialog!.onDeny()
      await m.flush()
      expect(m.calls.filter((c) => c.name === 'auth_approve_request').length).toBe(1)
      release({ status: 'denied' })
      await m.flush()
      m.elements()
      expect(dialog!.stage).toBe('denied')
    } finally {
      m.close()
    }
  })
})

/**
 * - DW-A36/A37 (and DW-A49): the strength meter shows only what core evaluated for the password now in the field. Found
 *   while proving DW-A36#1: typing a short password after a strong one kept "Strong." when the evaluation did not answer.
 */
describe('DW-A37: the meter follows the latest password, and a failed evaluation claims no strength', () => {
  const STRONG = { length: 25, min_length: 12, missing: 0, strength: 'strong', hint: 'none' } as const
  const SHORT = { length: 10, min_length: 12, missing: 2, strength: 'too_short', hint: 'more' } as const
  const flush = () => new Promise((r) => setTimeout(r, 0))

  test('a failed evaluation clears the meter', async () => {
    const { followMeter } = await import('../src/v2/auth/create/api')
    const seen: unknown[] = []
    followMeter('Winter-har', (m) => seen.push(m), async () => ({ ok: false, reason: 'no signup in progress' }))
    await flush()
    expect(seen).toEqual([null])
  })

  test('an answer for an older keystroke is dropped once a newer one asked', async () => {
    const { followMeter } = await import('../src/v2/auth/create/api')
    const seen: unknown[] = []
    let answerOld: (v: unknown) => void = () => {}
    const old = followMeter('Winter-harbour-lantern-42', (m) => seen.push(m), () => new Promise((r) => { answerOld = r as never }))
    old()
    followMeter('Winter-har', (m) => seen.push(m), async () => ({ ok: true, value: SHORT }))
    await flush()
    answerOld({ ok: true, value: STRONG })
    await flush()
    expect(seen).toEqual([SHORT])
  })
})

/**
 * - DW-A30: Unlock after a passkey says what went wrong. The screen took an `error` the flow never passed, so a wrong
 *   password (or a sign-in that did not finish) left the button doing nothing visible.
 */
describe('DW-A30: a failed Unlock after the passkey is said on the screen', () => {
  test('the password screen gets the board’s words for the failure', async () => {
    const api = await import('../src/v2/auth/api')
    const model = await import('../src/v2/auth/model')
    const { mount } = await import('./fixtures/componentHarness')
    let signInView: Record<string, any> | null = null
    let passkeyView: Record<string, any> | null = null
    const m = mount('v2/auth/index.tsx', 'SignInFlow', {
      backend: {
        last_signed_in_email: () => 'anna@northwind.studio',
        auth_passkey_sign_in: () => ({ requires_2fa: false, reauthenticated: false, vault_unlocked: false, key_replaced: false, account_mismatch: null }),
        desktop_login: () => { throw 'invalid_credentials' },
      },
      bindings: {
        ...api, ...model, CREATE_ACCOUNT_URL: '', FORGOT_PASSWORD_URL: '', openUrl() {}, EMPTY_WORDS: Array.from({ length: 12 }, () => ''),
        SignInView: (p: Record<string, any>) => { signInView = p; return null },
        PasskeyPasswordView: (p: Record<string, any>) => { passkeyView = p; return null },
        AnotherDeviceView: () => null, ContinueAsView: () => null, FirstRunView: () => null, LostPhraseView: () => null, PhraseView: () => null, TwoFactorView: () => null,
      },
      props: { onSignedIn() {}, startAt: 'sign-in' },
      expand: true,
    })
    try {
      await m.flush()
      m.elements()
      // A wrong password first, on the sign-in screen: its words belong to that try, not to the passkey's screen.
      signInView!.onPassword('a typo')
      await m.flush()
      m.elements()
      await signInView!.onSubmit()
      await m.flush()
      m.elements()
      expect(signInView!.problem).toEqual({ kind: 'invalid' })
      await signInView!.onPasskey()
      await m.flush()
      m.elements()
      expect(passkeyView).not.toBeNull()
      expect(passkeyView!.error).toBeUndefined()
      passkeyView!.onPassword('not my password')
      await m.flush()
      m.elements()
      await passkeyView!.onUnlock()
      await m.flush()
      m.elements()
      expect(m.calls.filter((c) => c.name === 'desktop_login').length).toBe(2)
      expect(passkeyView!.error).toBe('That email and password don’t match. Nothing was unlocked.')
    } finally {
      m.close()
    }
  })
})
