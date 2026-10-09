/**
 * R8 (spec 2026-10-06): "Sign in again as the same account and only the session token is replaced.
 * … Signing in as a different account is an account switch: full sign-out, with a warning if edits
 * haven't uploaded yet." The onboarding window in reauth mode, the routing of the three outcomes,
 * and the switch warning. The copy must match the design artefact (Task 13).
 *
 * The one rule these tests exist to hold: on macOS the ONLY path from this flow to a purge is the
 * switch warning's "Sign out and switch", after an explicit click. Same account: `clear_session` is
 * never called. Another account and Cancel: never called. Another account and confirm: called
 * exactly once, and only after the click.
 *
 * What this proves: the components' real handlers and render output for scripted backend answers.
 * What it does NOT prove: layout, focus, or a real two-account sign-in on a Mac (Task 21).
 */
import { afterEach, describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { createElement, Fragment } from 'react'
import * as desktopApi from '../src/desktopApi'
import * as switchCopy from '../src/accountSwitchCopy'
import * as signIn from '../src/onboardingSignIn'
import { FINDER_RAIL_DETAIL, FINDER_RAIL_TITLE } from '../src/finderSetupCopy'
import { loadComponent, mount, textOf, type Mounted } from './fixtures/componentHarness'

const React = { createElement, Fragment }
const Card = loadComponent('Onboarding.tsx', 'Card', { React })
const Field = loadComponent('Onboarding.tsx', 'Field', { React })
const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close() })

/** Stand-ins for the steps OnboardingView routes between. The harness does not expand components,
 *  so a rendered step is an element whose `type` is one of these functions. */
function stepStubs() {
  const byName: Record<string, (props: any) => null> = {}
  for (const name of ['SignInStep', 'UnlockStep', 'MacFinderStep', 'FinderInstallStep', 'PinningStep', 'ReadyStep', 'AccountSwitchStep']) {
    byName[name] = () => null
  }
  return {
    byName,
    bindings: { ...byName, STEPS: [], Wordmark: () => null, useRegionLabel: () => 'Stored in the EU' },
  }
}

function openView(mode: 'setup' | 'reauth', opts: { signedIn?: boolean; closeRefused?: boolean } = {}) {
  const signedIn = opts.signedIn ?? true
  const closed: string[] = []
  // `closeRefused`: the Tauri ACL rejects the close (no `core:window:allow-close` grant). The view
  // must then fall back to the DOM close, never leave the rejection unhandled.
  const close = opts.closeRefused
    ? async () => { throw new Error('window.close not allowed') }
    : async () => { closed.push('close') }
  const stubs = stepStubs()
  const m = mount('Onboarding.tsx', 'OnboardingView', {
    backend: {
      sync_status: () => ({ logged_in: signedIn, vault_unlocked: signedIn, engine: 'running', sync_root: '/x', syncing: 0, cloud_only: 0, conflicts: 0 }),
      desktop_platform: () => 'macos',
      finder_setup_state: () => ({ setup: 'ready', reason: null, launch_location: 'applications', attempt: 1, max_attempts: 1, last_failure: null }),
    },
    props: { mode },
    bindings: {
      ...desktopApi,
      loadFinderSetup: () => desktopApi.command('finder_setup_state'),
      getCurrentWindow: () => ({ close }),
      // The capability snapshot (main.tsx's CapabilityProvider) and the macOS rail copy, which the
      // view reads on every render even though the rail itself is empty here.
      useCapabilities: () => ({ host_os: 'macos' }),
      FINDER_RAIL_TITLE,
      FINDER_RAIL_DETAIL,
      ...stubs.bindings,
    },
  })
  mounted.push(m)
  ;(globalThis as any).window.close = () => { closed.push('dom-close') }
  const find = (name: string) => m.elements().find((el) => el.type === stubs.byName[name])
  const shown = () => Object.keys(stubs.byName).filter((name) => find(name) !== undefined)
  return { m, find, shown, has: (name: string) => find(name) !== undefined, closed }
}

const clearCalls = (m: Mounted) => m.calls.filter((c) => c.name === 'clear_session')

describe('Onboarding passes its mode to the view', () => {
  const Boundary = () => null
  const View = () => null
  const Root = loadComponent('Onboarding.tsx', 'Onboarding', { React, OnboardingErrorBoundary: Boundary, OnboardingView: View })

  test('reauth is reauth; anything else, including no prop, is setup', () => {
    for (const [props, mode] of [[{ mode: 'reauth' }, 'reauth'], [{ mode: 'setup' }, 'setup'], [{}, 'setup']] as const) {
      const tree = Root(props)
      expect(tree.type).toBe(Boundary)
      expect(tree.props.children.type).toBe(View)
      expect(tree.props.children.props.mode).toBe(mode)
    }
  })
})

describe('reauth mode', () => {
  test('starts at sign-in even when sync_status says signed in and unlocked (setup mode still fast-forwards)', async () => {
    const reauth = openView('reauth')
    await reauth.m.flush(); await reauth.m.flush()
    expect(reauth.has('SignInStep')).toBe(true)
    const setup = openView('setup')
    await setup.m.flush(); await setup.m.flush()
    expect(setup.has('ReadyStep')).toBe(true)
  })

  test('the same account with its keys here closes the window and never signs out', async () => {
    const v = openView('reauth')
    await v.m.flush(); await v.m.flush()
    v.find('SignInStep')!.props.onDone({ kind: 'reauthenticated', vaultUnlocked: true })
    await v.m.flush()
    expect(v.closed).toEqual(['close'])
    expect(v.m.calls.map((c) => c.name)).not.toContain('clear_session')
  })

  test('the same account: a close the window ACL refuses falls back to the DOM close (awaited, never unhandled)', async () => {
    const v = openView('reauth', { closeRefused: true })
    await v.m.flush(); await v.m.flush()
    v.find('SignInStep')!.props.onDone({ kind: 'reauthenticated', vaultUnlocked: true })
    await v.m.flush()
    expect(v.closed).toEqual(['dom-close'])
  })

  test('the same account without keys on this Mac goes on to the recovery phrase', async () => {
    const v = openView('reauth')
    await v.m.flush(); await v.m.flush()
    v.find('SignInStep')!.props.onDone({ kind: 'reauthenticated', vaultUnlocked: false })
    await v.m.flush()
    expect(v.has('UnlockStep')).toBe(true)
    expect(v.closed).toEqual([])
    expect(clearCalls(v.m)).toHaveLength(0)
  })

  test('another account shows the switch warning with the pending count, and nothing is cleared yet', async () => {
    const v = openView('reauth')
    await v.m.flush(); await v.m.flush()
    v.find('SignInStep')!.props.onDone({ kind: 'account_mismatch', pendingChanges: 3 })
    await v.m.flush()
    expect(v.find('AccountSwitchStep')!.props.pendingChanges).toBe(3)
    expect(v.m.calls.map((c) => c.name)).not.toContain('clear_session')
    expect(v.closed).toEqual([])
  })

  test('a sign-in that is neither (nothing local to keep) continues the normal set-up', async () => {
    const v = openView('reauth')
    await v.m.flush(); await v.m.flush()
    v.find('SignInStep')!.props.onDone({ kind: 'fresh' })
    await v.m.flush()
    expect(v.shown()).toEqual(['UnlockStep'])
    expect(clearCalls(v.m)).toHaveLength(0)
  })

  test('Cancel on the switch warning closes the reauth window and changes nothing', async () => {
    const v = openView('reauth')
    await v.m.flush(); await v.m.flush()
    v.find('SignInStep')!.props.onDone({ kind: 'account_mismatch', pendingChanges: 2 })
    await v.m.flush()
    v.find('AccountSwitchStep')!.props.onCancel()
    await v.m.flush()
    expect(v.closed).toEqual(['close'])
    expect(clearCalls(v.m)).toHaveLength(0)
  })

  test('Cancel in reauth mode: a close the window ACL refuses falls back to the DOM close', async () => {
    const v = openView('reauth', { closeRefused: true })
    await v.m.flush(); await v.m.flush()
    v.find('SignInStep')!.props.onDone({ kind: 'account_mismatch', pendingChanges: 2 })
    await v.m.flush()
    v.find('AccountSwitchStep')!.props.onCancel()
    await v.m.flush()
    expect(v.closed).toEqual(['dom-close'])
    expect(clearCalls(v.m)).toHaveLength(0)
  })

  test('after the switch, sign-in starts over (and the view itself never signs anything out)', async () => {
    const v = openView('reauth')
    await v.m.flush(); await v.m.flush()
    v.find('SignInStep')!.props.onDone({ kind: 'account_mismatch', pendingChanges: 2 })
    await v.m.flush()
    v.find('AccountSwitchStep')!.props.onSwitched()
    await v.m.flush()
    expect(v.shown()).toEqual(['SignInStep'])
    expect(v.closed).toEqual([])
    expect(clearCalls(v.m)).toHaveLength(0)
  })

  test('setup mode: Cancel goes back to sign-in rather than closing the window', async () => {
    const v = openView('setup', { signedIn: false })
    await v.m.flush(); await v.m.flush()
    expect(v.shown()).toEqual(['SignInStep'])
    v.find('SignInStep')!.props.onDone({ kind: 'account_mismatch', pendingChanges: 1 })
    await v.m.flush()
    expect(v.shown()).toEqual(['AccountSwitchStep'])
    v.find('AccountSwitchStep')!.props.onCancel()
    await v.m.flush()
    expect(v.shown()).toEqual(['SignInStep'])
    expect(v.closed).toEqual([])
    expect(clearCalls(v.m)).toHaveLength(0)
  })
})

describe('SignInStep reports what the sign-in became', () => {
  function openSignIn(backend: Record<string, (args: any) => unknown>) {
    const done: unknown[] = []
    const m = mount('Onboarding.tsx', 'SignInStep', {
      backend: { last_signed_in_email: () => null, ...backend },
      props: { onDone: (settled: unknown) => done.push(settled) },
      bindings: {
        ...signIn,
        lastSignedInEmail: desktopApi.lastSignedInEmail,
        Card,
        Field,
        requestAnimationFrame: () => 0,
      },
    })
    mounted.push(m)
    return { m, done }
  }

  const field = (m: Mounted, label: string) => m.elements().find((el) => el.props.label === label)!
  async function submitPassword(m: Mounted) {
    field(m, 'Email').props.onChange('user@beebeeb.io')
    field(m, 'Password').props.onChange('correct horse')
    m.render()
    await m.elements().find((el) => el.type === 'form')!.props.onSubmit({ preventDefault() {} })
    await m.flush()
  }
  async function submitCode(m: Mounted, code: string) {
    m.elements().find((el) => el.type === 'input' && el.props.inputMode === 'numeric')!.props.onChange({ currentTarget: { value: code } })
    m.render()
    await m.elements().find((el) => el.type === 'form')!.props.onSubmit({ preventDefault() {} })
    await m.flush()
  }

  test('a same-account password sign-in hands its outcome to onDone', async () => {
    const { m, done } = openSignIn({
      desktop_login: () => ({ requires_2fa: false, reauthenticated: true, vault_unlocked: true, account_mismatch: null }),
    })
    await m.flush()
    await submitPassword(m)
    expect(done).toEqual([{ kind: 'reauthenticated', vaultUnlocked: true }])
    expect(clearCalls(m)).toHaveLength(0)
  })

  test('another account hands over the mismatch and its count; nothing is cleared', async () => {
    const { m, done } = openSignIn({
      desktop_login: () => ({ requires_2fa: false, reauthenticated: false, vault_unlocked: false, account_mismatch: { pending_changes: 4 } }),
    })
    await m.flush()
    await submitPassword(m)
    expect(done).toEqual([{ kind: 'account_mismatch', pendingChanges: 4 }])
    expect(clearCalls(m)).toHaveLength(0)
  })

  test('a 2FA account learns its outcome from the code step, not the password step', async () => {
    const { m, done } = openSignIn({
      desktop_login: () => ({ requires_2fa: true, reauthenticated: false, vault_unlocked: false, account_mismatch: null }),
      desktop_login_2fa: () => ({ requires_2fa: false, reauthenticated: false, vault_unlocked: false, account_mismatch: { pending_changes: 2 } }),
    })
    await m.flush()
    await submitPassword(m)
    expect(done).toEqual([])
    await submitCode(m, '123456')
    expect(done).toEqual([{ kind: 'account_mismatch', pendingChanges: 2 }])
    expect(clearCalls(m)).toHaveLength(0)
  })

  test('a result it cannot read is an error on the form: onDone is never called', async () => {
    const { m, done } = openSignIn({
      desktop_login: () => ({ requires_2fa: false, reauthenticated: true, vault_unlocked: true, account_mismatch: { pending_changes: 2 } }),
    })
    await m.flush()
    await submitPassword(m)
    expect(done).toEqual([])
    expect(textOf(m.tree())).toContain(switchCopy.SIGN_IN_OUTCOME_UNREADABLE)
    expect(clearCalls(m)).toHaveLength(0)
  })
})

describe('AccountSwitchStep', () => {
  /**
   * A stand-in for the browser's focus. React attaches refs at commit, before effects run, so each
   * button that carries a ref gets a fake element that records `focus()` BEFORE the first flush.
   * `pressEnter` is what Enter does in a browser: it activates the focused button.
   */
  function withFocusModel(m: Mounted) {
    const focused: string[] = []
    for (const el of m.elements().filter((e) => e.type === 'button')) {
      const label = textOf(el.props.children).trim()
      if (el.props.ref && typeof el.props.ref === 'object') el.props.ref.current = { focus: () => focused.push(label) }
    }
    const pressEnter = async () => {
      const label = focused.at(-1)
      if (label === undefined) return
      await m.elements().find((e) => e.type === 'button' && textOf(e.props.children).trim() === label)!.props.onClick()
      await m.flush()
    }
    return { focused, pressEnter }
  }

  function openSwitch(pendingChanges: number, clear: () => unknown = () => undefined) {
    const events: string[] = []
    const m = mount('Onboarding.tsx', 'AccountSwitchStep', {
      backend: { clear_session: clear },
      props: { pendingChanges, onSwitched: () => events.push('switched'), onCancel: () => events.push('cancelled') },
      bindings: { ...desktopApi, ...switchCopy, Card },
    })
    mounted.push(m)
    return { m, events }
  }

  test('says how many changes would be removed, in the drawn copy', async () => {
    const { m } = openSwitch(3)
    await m.flush()
    // The root is the (unexpanded) Card element: its title and copy are props.
    expect(m.tree().props.title).toBe(switchCopy.ACCOUNT_SWITCH_TITLE)
    expect(m.tree().props.copy).toBe(switchCopy.accountSwitchBody(3))
  })

  test('nothing waiting names no loss; one change is singular', async () => {
    const none = openSwitch(0)
    await none.m.flush()
    expect(none.m.tree().props.copy).toBe(switchCopy.accountSwitchBody(0))
    const one = openSwitch(1)
    await one.m.flush()
    expect(one.m.tree().props.copy).toBe(switchCopy.accountSwitchBody(1))
  })

  test('mounting it signs nothing out: the warning comes first', async () => {
    const { m, events } = openSwitch(3)
    await m.flush(); await m.flush()
    expect(m.calls).toEqual([])
    expect(events).toEqual([])
  })

  // Design §4 (lead ruling 2026-10-06): on open, focus is on Cancel, so Enter cancels and can never confirm.
  test('opens with focus on Cancel, and on nothing else', async () => {
    const { m } = openSwitch(3)
    const { focused } = withFocusModel(m)
    await m.flush()
    expect(focused).toEqual(['Cancel'])
  })

  test('Enter right after the step opens cancels; it never signs out', async () => {
    const { m, events } = openSwitch(3)
    const { pressEnter } = withFocusModel(m)
    await m.flush()
    await pressEnter()
    expect(events).toEqual(['cancelled'])
    expect(clearCalls(m)).toHaveLength(0)
  })

  test('Cancel changes nothing', async () => {
    const { m, events } = openSwitch(3)
    await m.flush()
    await m.click('Cancel')
    expect(events).toEqual(['cancelled'])
    expect(m.calls.map((c) => c.name)).not.toContain('clear_session')
  })

  test('Sign out and switch signs out once, then hands back to sign-in', async () => {
    const { m, events } = openSwitch(3)
    await m.flush()
    await m.click('Sign out and switch')
    expect(m.calls.filter((c) => c.name === 'clear_session')).toHaveLength(1)
    expect(events).toEqual(['switched'])
  })

  test('while the sign-out runs, neither button can be pressed again', async () => {
    let release: () => void = () => undefined
    const gate = new Promise<void>((resolve) => { release = resolve })
    const { m, events } = openSwitch(3, () => gate)
    await m.flush()
    await m.clickNoWait('Sign out and switch')
    const buttons = m.elements().filter((el) => el.type === 'button')
    expect(buttons.map((b) => [textOf(b.props.children).trim(), b.props.disabled])).toEqual([['Cancel', true], ['Sign out and switch', true]])
    release()
    await m.flush()
    expect(clearCalls(m)).toHaveLength(1)
    expect(events).toEqual(['switched'])
  })

  test('a failed sign-out is a toast, and the switch does not happen', async () => {
    const { m, events } = openSwitch(1, () => { throw new Error('engine busy') })
    await m.flush()
    await m.click('Sign out and switch')
    expect(m.toasts.map((t) => t.title)).toEqual(['Couldn’t sign out'])
    expect(m.toasts[0].message).toContain('engine busy')
    expect(events).toEqual([])
  })

  test('the confirm is the filled destructive button, never the amber one; Cancel is the plain button', async () => {
    const { m } = openSwitch(3)
    await m.flush()
    const classesOf = (label: string) =>
      String(m.elements().find((el) => el.type === 'button' && textOf(el.props.children).trim() === label)!.props.className).split(' ')
    expect(classesOf('Sign out and switch')).toEqual(expect.arrayContaining(['button', 'danger', 'filled']))
    expect(classesOf('Sign out and switch')).not.toContain('amber')
    expect(classesOf('Cancel')).toEqual(['button'])
  })
})

describe('accountSwitchCopy', () => {
  test('the exact strings, singular and plural, and the artefact draws them (design first)', () => {
    expect(switchCopy.ACCOUNT_SWITCH_TITLE).toBe('Switch to a different account?')
    expect(switchCopy.accountSwitchBody(0)).toBe('This Mac is signed in to another Beebeeb account. Switching signs that account out of this Mac.')
    expect(switchCopy.accountSwitchBody(1)).toBe('This Mac is signed in to another Beebeeb account, and 1 change on this Mac hasn’t uploaded yet. Switching signs that account out of this Mac and removes it.')
    expect(switchCopy.accountSwitchBody(3)).toBe('This Mac is signed in to another Beebeeb account, and 3 changes on this Mac haven’t uploaded yet. Switching signs that account out of this Mac and removes them.')
    expect([switchCopy.ACCOUNT_SWITCH_CANCEL, switchCopy.ACCOUNT_SWITCH_CONFIRM]).toEqual(['Cancel', 'Sign out and switch'])
    expect(switchCopy.ACCOUNT_SWITCH_FAILED).toBe('Couldn’t sign out')
    const html = readFileSync(new URL('../design/hifi/macos-settings-dialogs.html', import.meta.url), 'utf8')
    for (const text of [switchCopy.ACCOUNT_SWITCH_TITLE, switchCopy.accountSwitchBody(3), switchCopy.accountSwitchBody(0), switchCopy.ACCOUNT_SWITCH_CONFIRM, switchCopy.ACCOUNT_SWITCH_CANCEL, switchCopy.ACCOUNT_SWITCH_FAILED]) {
      expect(html).toContain(text)
    }
  })

  // The artefact draws the 3-change and 0-change variants and describes the single change in prose
  // (“1 change … hasn’t uploaded yet … removes it.”); hold the singular to those three phrases.
  test('the fail-closed sign-in sentence lives here too, and says nothing about what changed', () => {
    expect(switchCopy.SIGN_IN_OUTCOME_UNREADABLE).toBe('Beebeeb couldn’t read the result of that sign-in. Try signing in again.')
  })

  test('the singular body keeps the phrases the artefact’s caption names', () => {
    const html = readFileSync(new URL('../design/hifi/macos-settings-dialogs.html', import.meta.url), 'utf8')
    expect(html).toContain('“1 change … hasn’t uploaded yet … removes it.”')
    const body = switchCopy.accountSwitchBody(1)
    for (const phrase of ['1 change', 'hasn’t uploaded yet', 'removes it.']) expect(body).toContain(phrase)
  })
})

describe('the filled destructive class has CSS behind it, from the 1683 tokens only', () => {
  const css = (file: string) => readFileSync(new URL(`../src/${file}`, import.meta.url), 'utf8')
  /** The declarations of the rule whose selector is exactly `selector`, as a sorted list. */
  const declarations = (text: string, selector: string) => {
    const open = text.indexOf(`\n${selector} {`)
    if (open < 0) return null
    const body = text.slice(text.indexOf('{', open) + 1, text.indexOf('}', open))
    return body.split(';').map((d) => d.trim().replace(/\s+/g, ' ')).filter(Boolean).sort()
  }

  test('`.button.danger.filled` paints exactly what `.ms-btn--danger` paints: --red fill, --red-ink label', () => {
    const mine = declarations(css('design.css'), '.button.danger.filled')
    const settings = declarations(css('macSettings.css'), '.ms-btn--danger')
    expect(mine).not.toBeNull()
    expect(mine).toEqual(['background: var(--red)', 'border-color: var(--red)', 'color: var(--red-ink)'])
    expect(settings).toEqual(mine)
  })

  test('hovering it keeps the red fill (the plain .button hover would turn it paper)', () => {
    const hover = declarations(css('design.css'), '.button.danger.filled:hover:not(:disabled)')
    expect(hover).not.toBeNull()
    expect(hover).toEqual(expect.arrayContaining(['background: var(--red)', 'color: var(--red-ink)']))
  })
})

describe('wiring', () => {
  const source = (file: string) => readFileSync(new URL(`../src/${file}`, import.meta.url), 'utf8')

  test('main.tsx opens the macOS onboarding window in the mode its URL names, and only that window', () => {
    const main = source('main.tsx')
    expect(main).toContain("<Onboarding mode={params.get('mode') === 'reauth' ? 'reauth' : 'setup'} />")
    expect(main).toContain('<WindowsFirstRun />')
  })

  test('the banner and the review action both go through forceReauth; no component names open_reauth_window itself', () => {
    for (const file of ['AuthExpiredBanner.tsx', 'pages/VersionCenter.tsx']) expect(source(file)).toContain('forceReauth')
    const callers = ['Onboarding.tsx', 'AuthExpiredBanner.tsx', 'pages/VersionCenter.tsx', 'MacSettings.tsx'].filter((file) => source(file).includes('open_reauth_window'))
    expect(callers).toEqual([])
  })
})
