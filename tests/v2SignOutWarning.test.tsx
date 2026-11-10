/**
 * Desktop v2's two sign-outs (R1f): the Settings confirm (`SignOutDialog`, DW-A57/A57a) and the no-plan screen
 * (`NoPlanWindow`, DW-0014). Both called the raw `clear_session` and dropped what it answered, so a sign-out that
 * happened but could not confirm one step (FB-24, `Ok({warning})`) said nothing about it. They now go through
 * desktopApi's `clearSession` and show the warning's sentence as a neutral status line before the window moves on
 * (tests/sessionActionOutcome.test.ts pins the source contract: the wrapper only, and every caller reads `.warning`).
 *
 * On Windows today `clear_session` never answers a warning (its only one, the Finder removal, is macOS-only), so the
 * warning path is the contract's, kept ready; the paths Windows takes are here too: a sign-out that did not happen says
 * why, and one that did lands on the v2 window's own sign-in. Run with the component harness against a scripted
 * `clear_session`.
 */
import { afterEach, describe, expect, test } from 'bun:test'
import { createElement } from 'react'
import { clearSession, command, signOutUnknownFiles } from '../src/desktopApi'
import { elementsOf, loadComponent, mount, textOf, type Mounted } from './fixtures/componentHarness'
import { rustStr } from './fixtures/rustConstants'

const SENTENCE = rustStr('lib.rs', 'FINDER_SIGN_OUT_UNCONFIRMED_WARNING')
const WARNED = { warning: { code: 'finder_removal_unconfirmed', sentence: SENTENCE } }
const CLEAN = { warning: null }
const REFUSED = 'Sign-out paused: the sync folder could not be read.'

const mounted: Mounted[] = []
afterEach(() => {
  while (mounted.length) mounted.pop()!.close()
})

const statusLines = (m: Mounted) => m.elements().filter((el) => el.props.role === 'status').map((el) => textOf(el.props.children))

describe('the no-plan screen\'s sign-out', () => {
  function open(answer: () => unknown) {
    const signedOut: Array<string | null> = []
    const m = mount('v2/auth/Gate.tsx', 'NoPlanWindow', {
      backend: { sign_out_unknown_files: () => ({ count: 0, folder: null }), clear_session: answer },
      // The view is a plain tag here: the test reads what NoPlanWindow hands it, not how it draws it.
      bindings: { clearSession, signOutUnknownFiles, command, PLANS_URL: 'https://beebeeb.io/pricing', NoPlanView: 'no-plan-view', SignOutUnknownFilesDialog: 'sign-out-unknown-dialog' },
      props: { onCheckAgain() {}, onSignedOut: (warning: string | null) => signedOut.push(warning) },
    })
    mounted.push(m)
    const view = () => m.elements().find((el) => el.type === 'no-plan-view')!
    const signOut = async () => { view().props.onSignOut(); await m.flush() }
    return { m, view, signOut, signedOut }
  }

  test('a warning goes up with the sign-out', async () => {
    const { m, signOut, signedOut } = open(() => WARNED)
    await signOut()
    expect(m.calls.map((c) => c.name)).toEqual(['sign_out_unknown_files', 'clear_session'])
    expect(signedOut).toEqual([SENTENCE])
  })

  test('a clean sign-out goes up with no warning', async () => {
    const { signOut, signedOut, view } = open(() => CLEAN)
    await signOut()
    expect(signedOut).toEqual([null])
    expect(view().props.error).toBeNull()
  })

  test('a sign-out that did not happen stays on the screen and says why', async () => {
    const { signOut, signedOut, view } = open(() => { throw new Error(REFUSED) })
    await signOut()
    expect(signedOut).toEqual([])
    expect(view().props.error).toBe(REFUSED)
  })
})

describe('the gate holds the warning until OK', () => {
  test('a later check that finds the session gone (a focus brings one) does not drop it', async () => {
    let loggedIn = true
    // hasNoPlan moved to its own module (#274 review: the flyout imported the gate's chrome for it).
    const bindings = { command, hasNoPlan: loadComponent('v2/auth/plan.ts', 'hasNoPlan', {}) }
    const readGate = loadComponent('v2/auth/Gate.tsx', 'readGate', bindings)
    const SignedOutWarning = loadComponent('v2/auth/Gate.tsx', 'SignedOutWarning', { GateWindow: 'gate-window', BBButton: 'bb-button', React: { createElement } })
    const m = mount('v2/auth/Gate.tsx', 'AuthGate', {
      backend: {
        sync_status: () => ({ logged_in: loggedIn, vault_unlocked: loggedIn }),
        account_subscription: () => ({ plan_state: 'none' }),
      },
      bindings: { readGate, SignedOutWarning, NoPlanWindow: 'no-plan-window', AccountEntry: 'account-entry', LockedWindow: 'locked-window' },
      props: { children: 'drive' },
      expand: true,
    })
    mounted.push(m)
    await m.flush()
    const noPlan = m.elements().find((el) => el.type === 'no-plan-window')!
    loggedIn = false
    noPlan.props.onSignedOut(SENTENCE)
    await m.flush()
    // The check after the sign-out (and any later one) reads `signed-out`; the warning still shows, over it.
    expect(statusLines(m)).toEqual([SENTENCE])
    expect(m.elements().some((el) => el.type === 'account-entry')).toBe(false)
    m.elements().find((el) => el.type === 'bb-button')!.props.onClick()
    await m.flush()
    expect(statusLines(m)).toEqual([])
    expect(m.elements().some((el) => el.type === 'account-entry')).toBe(true)
  })
})

describe('the Settings sign-out confirm', () => {
  const SIGN_IN = '?window=main-app&platform=windows'

  function open(answer: () => unknown) {
    const assigned: string[] = []
    const m = mount('v2/sections/settings/account/SecurityPane.tsx', 'SignOutDialog', {
      backend: { sign_out_unknown_files: () => ({ count: 0, folder: null }), clear_session: answer },
      bindings: { clearSession, signOutUnknownFiles, hasTauri: () => true, Dialog: 'settings-dialog', Btn: 'settings-btn', SignOutUnknownFilesDialog: 'sign-out-unknown-dialog' },
      props: { all: false, onCancel() {} },
    })
    mounted.push(m)
    ;(globalThis as any).window.location = { assign: (to: string) => assigned.push(to) }
    // The dialog's buttons are its `actions`, a prop of the dialog rather than a child.
    const dialog = () => m.elements().find((el) => el.type === 'settings-dialog')!
    const actions = () => elementsOf(dialog().props.actions)
    const confirm = async () => {
      const button = actions().find((el) => el.type === 'settings-btn' && el.props.variant === 'danger')!
      expect(textOf(button.props.children)).toBe('Sign out')
      button.props.onClick()
      await m.flush()
    }
    return { m, confirm, assigned, actions, dialog }
  }

  test('a clean sign-out reloads the v2 window, whose gate shows the sign-in (not the legacy window, not a frame)', async () => {
    const { confirm, assigned, m } = open(() => CLEAN)
    await confirm()
    expect(assigned).toEqual([SIGN_IN])
    expect(statusLines(m)).toEqual([])
  })

  test('a warning keeps the dialog, titled Signed out, with the sentence as a status line; OK moves on', async () => {
    const { m, confirm, assigned, actions, dialog } = open(() => WARNED)
    await confirm()
    expect(m.calls.map((c) => c.name)).toEqual(['sign_out_unknown_files', 'clear_session'])
    expect(dialog().props.title).toBe('Signed out')
    expect(statusLines(m)).toEqual([SENTENCE])
    expect(assigned).toEqual([])
    const ok = actions().filter((el) => el.type === 'settings-btn')
    expect(ok.map((el) => textOf(el.props.children))).toEqual(['OK'])
    ok[0].props.onClick()
    expect(assigned).toEqual([SIGN_IN])
  })

  test('a sign-out that did not happen keeps the confirm open and says why', async () => {
    const { m, confirm, assigned } = open(() => { throw new Error(REFUSED) })
    await confirm()
    expect(assigned).toEqual([])
    expect(m.elements().filter((el) => el.props.role === 'alert').map((el) => textOf(el.props.children))).toEqual([REFUSED])
  })
})
