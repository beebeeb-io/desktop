/**
 * Desktop v2's two sign-outs (R1f): the Settings confirm (`SignOutDialog`, DW-A57/A57a) and the no-plan screen
 * (`NoPlanWindow`, DW-0014). Both called the raw `clear_session` and dropped what it answered, so a sign-out that
 * happened but could not confirm one step (FB-24, `Ok({warning})`) said nothing about it. They now go through
 * desktopApi's `clearSession` and show the warning's sentence as a neutral status line before the window moves on
 * (tests/sessionActionOutcome.test.ts pins the source contract: the wrapper only, and every caller reads `.warning`).
 * Run with the component harness against a scripted `clear_session`.
 */
import { afterEach, describe, expect, test } from 'bun:test'
import { clearSession, command } from '../src/desktopApi'
import { elementsOf, mount, textOf, type Mounted } from './fixtures/componentHarness'
import { rustStr } from './fixtures/rustConstants'

const SENTENCE = rustStr('lib.rs', 'FINDER_SIGN_OUT_UNCONFIRMED_WARNING')
const WARNED = { warning: { code: 'finder_removal_unconfirmed', sentence: SENTENCE } }
const CLEAN = { warning: null }

const mounted: Mounted[] = []
afterEach(() => {
  while (mounted.length) mounted.pop()!.close()
})

const statusLines = (m: Mounted) => m.elements().filter((el) => el.props.role === 'status').map((el) => textOf(el.props.children))

describe('the no-plan screen\'s sign-out', () => {
  function open(answer: unknown) {
    let signedOut = 0
    const m = mount('v2/auth/Gate.tsx', 'NoPlanWindow', {
      backend: { clear_session: () => answer },
      // The views are plain tags here: the test reads what NoPlanWindow hands them, not how they draw it.
      bindings: { clearSession, command, PLANS_URL: 'https://beebeeb.io/pricing', GateWindow: 'gate-window', NoPlanView: 'no-plan-view', BBButton: 'bb-button' },
      props: { onCheckAgain() {}, onSignedOut: () => { signedOut += 1 } },
    })
    mounted.push(m)
    const signOut = async () => { m.elements().find((el) => el.type === 'no-plan-view')!.props.onSignOut(); await m.flush() }
    return { m, signOut, signedOut: () => signedOut }
  }

  test('a warning is said as a status line, and the sign-in shows after OK', async () => {
    const { m, signOut, signedOut } = open(WARNED)
    await signOut()
    expect(m.calls.map((c) => c.name)).toEqual(['clear_session'])
    expect(statusLines(m)).toEqual([SENTENCE])
    expect(signedOut()).toBe(0)
    const ok = m.elements().find((el) => el.type === 'bb-button')!
    expect(textOf(ok.props.children)).toBe('OK')
    ok.props.onClick()
    expect(signedOut()).toBe(1)
  })

  test('without a warning the sign-in shows at once, with no extra screen', async () => {
    const { m, signOut, signedOut } = open(CLEAN)
    await signOut()
    expect(signedOut()).toBe(1)
    expect(statusLines(m)).toEqual([])
    expect(m.elements().some((el) => el.type === 'no-plan-view')).toBe(true)
  })
})

describe('the Settings sign-out confirm', () => {
  function open(answer: unknown) {
    const assigned: string[] = []
    const m = mount('v2/sections/settings/account/SecurityPane.tsx', 'SignOutDialog', {
      backend: { clear_session: () => answer },
      bindings: { clearSession, hasTauri: () => true, Dialog: 'settings-dialog', Btn: 'settings-btn' },
      props: { all: false, onCancel() {} },
    })
    mounted.push(m)
    ;(globalThis as any).window.location = { assign: (to: string) => assigned.push(to) }
    // The dialog's buttons are its `actions`, a prop of the dialog rather than a child.
    const actions = () => elementsOf(m.elements().find((el) => el.type === 'settings-dialog')!.props.actions)
    const confirm = async () => {
      const button = actions().find((el) => el.type === 'settings-btn' && el.props.variant === 'danger')!
      expect(textOf(button.props.children)).toBe('Sign out')
      button.props.onClick()
      await m.flush()
    }
    return { m, confirm, assigned, actions }
  }

  test('a warning keeps the dialog, titled Signed out, with the sentence as a status line; OK moves on', async () => {
    const { m, confirm, assigned, actions } = open(WARNED)
    await confirm()
    expect(m.calls.map((c) => c.name)).toEqual(['clear_session'])
    const dialog = elementsOf(m.tree()).find((el) => el.type === 'settings-dialog')!
    expect(dialog.props.title).toBe('Signed out')
    expect(statusLines(m)).toEqual([SENTENCE])
    expect(assigned).toEqual([])
    const ok = actions().filter((el) => el.type === 'settings-btn')
    expect(ok.map((el) => textOf(el.props.children))).toEqual(['OK'])
    ok[0].props.onClick()
    expect(assigned).toEqual(['?window=main-app&frame=DW-A02'])
  })

  test('without a warning the window moves on at once', async () => {
    const { m, confirm, assigned } = open(CLEAN)
    await confirm()
    expect(assigned).toEqual(['?window=main-app&frame=DW-A02'])
    expect(statusLines(m)).toEqual([])
  })
})
