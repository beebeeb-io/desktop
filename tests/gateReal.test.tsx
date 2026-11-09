/**
 * The sign-in gate's entry checks (slice 37): signed out, locked, no plan and a host that answers
 * nothing each name exactly one next step. The gate reads `sync_status` and `account_subscription`;
 * a failed read is not "ready" (which would show Files), and a locked vault with Windows Hello
 * unavailable offers the recovery phrase instead of a dead end. Run with the component harness
 * against a scripted backend, as the other gate tests do.
 */
import { afterEach, describe, expect, test } from 'bun:test'
import { command } from '../src/desktopApi'
import { unlockWithWindowsHello } from '../src/v2/auth/api'
import { loadComponent, mount, textOf, type Mounted } from './fixtures/componentHarness'

const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close() })

const hasNoPlan = loadComponent('v2/auth/plan.ts', 'hasNoPlan', {})
const readGate = loadComponent('v2/auth/Gate.tsx', 'readGate', { command, hasNoPlan })

function gate(backend: Record<string, (args?: any) => unknown>) {
  const m = mount('v2/auth/Gate.tsx', 'AuthGate', {
    backend,
    bindings: {
      readGate,
      AccountEntry: 'account-entry',
      LockedWindow: 'locked-window',
      NoPlanWindow: 'no-plan-window',
      SignedOutWarning: 'signed-out-warning',
      CheckFailedView: 'check-failed-view',
    },
    props: { children: 'files' },
  })
  mounted.push(m)
  return m
}

const GATE_SCREENS = ['account-entry', 'locked-window', 'no-plan-window', 'check-failed-view', 'signed-out-warning']
const screens = (m: Mounted) => m.elements().map((el) => String(el.type)).filter((t) => GATE_SCREENS.includes(t))
const buttons = (m: Mounted) => m.elements().filter((el) => el.type === 'bb-button')
const buttonLabels = (m: Mounted) => buttons(m).map((el) => textOf(el.props.children).trim())

const lockedBindings = {
  usePrivateScreen: () => {},
  GateWindow: 'gate-window',
  HelloPrompt: 'hello-prompt',
  BBButton: 'bb-button',
  Icon: 'icon',
}

describe('the sign-in gate names one next step', () => {
  test('signed out: the sign-in, not first run and not Files', async () => {
    const m = gate({ sync_status: () => ({ logged_in: false, vault_unlocked: false }) })
    await m.flush()
    expect(screens(m)).toEqual(['account-entry'])
    expect(m.elements().find((el) => el.type === 'account-entry')!.props.startAt).toBe('sign-in')
    expect(m.elements().some((el) => el.type === 'files')).toBe(false)
  })

  test('locked: Unlock with Windows Hello; with Hello unavailable, the recovery phrase', async () => {
    const m = gate({ sync_status: () => ({ logged_in: true, vault_unlocked: false }) })
    await m.flush()
    expect(screens(m)).toEqual(['locked-window'])

    const hello = mount('v2/auth/Gate.tsx', 'LockedView', {
      backend: {},
      bindings: lockedBindings,
      props: { prompt: false, mode: 'hello', onUnlock() {}, onPhrase() {} },
    })
    mounted.push(hello)
    expect(buttonLabels(hello)).toEqual(['Unlock with Windows Hello'])

    const phrase = mount('v2/auth/Gate.tsx', 'LockedView', {
      backend: {},
      bindings: lockedBindings,
      props: { prompt: false, mode: 'phrase', error: 'Windows Hello isn’t set up on this PC.', onUnlock() {}, onPhrase() {} },
    })
    mounted.push(phrase)
    expect(buttonLabels(phrase)).toEqual(['Unlock with the recovery phrase'])
    expect(textOf(phrase.elements().find((el) => el.props.role === 'alert')!.props.children)).toContain('Windows Hello isn’t set up')

    // The error map: `hello_unavailable` moves the lock to the phrase, never a second Hello prompt.
    const locked = mount('v2/auth/Gate.tsx', 'LockedWindow', {
      backend: { auth_unlock_with_windows_hello: () => { throw new Error('hello_unavailable') } },
      bindings: { unlockWithWindowsHello, LockedView: 'locked-view', PhraseUnlockView: 'phrase-unlock-view' },
      props: { onUnlocked() {} },
    })
    mounted.push(locked)
    await locked.flush()
    const view = locked.elements().find((el) => el.type === 'locked-view')!
    expect(view.props.mode).toBe('phrase')
    expect(view.props.error).toContain('Windows Hello isn’t set up')
  })

  test('no plan: says what is missing and links to the plans page, never a dead link', async () => {
    const m = gate({
      sync_status: () => ({ logged_in: true, vault_unlocked: true }),
      account_subscription: () => ({ plan_state: 'none' }),
    })
    await m.flush()
    expect(screens(m)).toEqual(['no-plan-window'])

    const view = mount('v2/auth/Gate.tsx', 'NoPlanView', {
      backend: {},
      bindings: { GateWindow: 'gate-window', BBButton: 'bb-button', PLANS_URL: 'https://beebeeb.io/pricing' },
      props: { onChoose() {}, onCheckAgain() {}, onSignOut() {} },
    })
    mounted.push(view)
    const text = textOf(view.tree())
    expect(text).toContain('Choose a plan')
    expect(text).toContain('Nothing syncs until a plan is chosen.')
    const links = view.elements().filter((el) => el.type === 'a')
    expect(links.length).toBeGreaterThan(0)
    expect(links.every((el) => el.props.href === 'https://beebeeb.io/pricing')).toBe(true)
    expect(links.some((el) => el.props.href === '#')).toBe(false)
  })

  test('a host that answers nothing: says it cannot check the session and offers Retry, not Files', async () => {
    const m = gate({ sync_status: () => { throw new Error('no answer') } })
    await m.flush()
    expect(screens(m)).toEqual(['check-failed-view'])
    expect(m.elements().some((el) => el.type === 'files')).toBe(false)

    const CheckFailedView = loadComponent('v2/auth/Gate.tsx', 'CheckFailedView', { GateWindow: 'gate-window', BBButton: 'bb-button', Icon: 'icon' })
    let retried = 0
    const view = mount('v2/auth/Gate.tsx', 'CheckFailedView', {
      backend: {},
      bindings: { GateWindow: 'gate-window', BBButton: 'bb-button', Icon: 'icon' },
      props: { onRetry: () => { retried += 1 } },
    })
    mounted.push(view)
    expect(CheckFailedView).toBeFunction()
    expect(textOf(view.tree())).toContain('can’t check the session')
    expect(buttonLabels(view)).toEqual(['Retry'])
    buttons(view)[0].props.onClick()
    expect(retried).toBe(1)
  })
})
