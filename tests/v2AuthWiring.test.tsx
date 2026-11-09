/**
 * Desktop v2 M3b (B10, B11): wiring the map audit found missing or wrong, checked on the real component declarations.
 * - DW-A26: the sixth digit submits the code it completes. The view used to call `onSubmit()` with nothing, and the
 *   flow then sent the code from its last render (five digits), so typing a code always came back "isn't right".
 */
import { describe, expect, test } from 'bun:test'
import { createElement, Fragment } from 'react'
import { digitsOnly } from '../src/v2/auth/model'
import { elementsOf, expand, loadComponent } from './fixtures/componentHarness'

const pass = ({ children }: { children?: unknown }) => createElement(Fragment, null, children as never)
const AUTH_PARTS = {
  React: { createElement, Fragment },
  AuthWindow: pass, StepLockup: () => null, Heading: pass, Note: pass, TextLink: pass, PrimaryButton: pass,
  Field: (p: Record<string, unknown>) => createElement('input', p), SECRET_FIELD: {},
}

describe('DW-A26: the sixth digit submits the whole code', () => {
  const TwoFactorView = loadComponent('v2/auth/Steps.tsx', 'TwoFactorView', { ...AUTH_PARTS, useRef: () => ({ current: null }), digitsOnly })
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
