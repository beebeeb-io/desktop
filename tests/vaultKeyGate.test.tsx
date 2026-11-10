// A password sign-in on Windows stores the session and leaves the vault locked. The gate used to treat every
// locked window as Windows Hello, and a focus check unmounted the 12-word step the sign-in had just opened.
// No key on this PC is that step. A key, or a store that does not answer, stays on the lock.
import { afterEach, describe, expect, test } from 'bun:test'
import { command } from '../src/desktopApi'
import { loadComponent, mount, textOf, type Mounted } from './fixtures/componentHarness'

const mounted: Mounted[] = []
afterEach(() => {
  while (mounted.length) mounted.pop()!.close()
})

type Key = boolean | 'fail'

function open(world: { loggedIn: boolean; unlocked: boolean; key: Key }) {
  const readGate = loadComponent('v2/auth/Gate.tsx', 'readGate', { command })
  const m = mount('v2/auth/Gate.tsx', 'AuthGate', {
    backend: {
      sync_status: () => ({ logged_in: world.loggedIn, vault_unlocked: world.unlocked }),
      auth_vault_key_present: () => {
        if (world.key === 'fail') throw new Error('credential store unreadable')
        return world.key
      },
      account_subscription: () => ({ plan_state: 'active' }),
    },
    bindings: {
      readGate,
      AccountEntry: 'account-entry',
      LockedWindow: 'locked-window',
      NoPlanWindow: 'no-plan-window',
      SignInFlow: 'sign-in-flow',
      SignedOutWarning: 'signed-out-warning',
    },
    props: { children: 'drive' },
    expand: true,
  })
  mounted.push(m)
  return m
}

const has = (m: Mounted, type: string) => m.elements().some((el) => el.type === type)

describe('a locked window asks for the 12 words only when this PC has no vault key', () => {
  test('no key is the phrase step, not the Hello lock', async () => {
    const m = open({ loggedIn: true, unlocked: false, key: false })
    await m.flush()
    expect(has(m, 'locked-window')).toBe(false)
    const flow = m.elements().find((el) => el.type === 'sign-in-flow')
    expect(flow?.props.startAt).toBe('phrase')
  })

  test('a key on this PC stays on the Hello lock', async () => {
    const m = open({ loggedIn: true, unlocked: false, key: true })
    await m.flush()
    expect(has(m, 'locked-window')).toBe(true)
    expect(has(m, 'sign-in-flow')).toBe(false)
  })

  test('a key store that cannot be read stays on the Hello lock', async () => {
    const m = open({ loggedIn: true, unlocked: false, key: 'fail' })
    await m.flush()
    expect(has(m, 'locked-window')).toBe(true)
    expect(has(m, 'sign-in-flow')).toBe(false)
  })

  test('a focus event while signing in does not replace that screen with the lock', async () => {
    const world = { loggedIn: false, unlocked: false, key: false as Key }
    const m = open(world)
    await m.flush()
    expect(has(m, 'account-entry')).toBe(true)
    world.loggedIn = true
    ;(globalThis as { window: { dispatchEvent: (ev: Event) => void } }).window.dispatchEvent(new Event('focus'))
    await m.flush()
    expect(has(m, 'account-entry')).toBe(true)
    expect(has(m, 'locked-window')).toBe(false)
    expect(has(m, 'sign-in-flow')).toBe(false)
  })
})

describe('the Hello lock can still open with the recovery phrase', () => {
  test('Unlock stays, and the phrase control is the one the lock was given', async () => {
    const used: string[] = []
    const m = mount('v2/auth/Gate.tsx', 'LockedView', {
      backend: {},
      props: { prompt: false, onUnlock() { used.push('unlock') }, onUsePhrase() { used.push('phrase') } },
      hookModules: [{
        file: 'v2/auth/privateScreen.ts',
        name: 'usePrivateScreen',
        bindings: { hasTauri: () => false, command: async () => ({ ok: true }) },
      }],
      bindings: { GateWindow: 'gate-window', BBButton: 'bb-button', Icon: 'icon', HelloPrompt: 'hello-prompt' },
    })
    mounted.push(m)
    const buttons = m.elements().filter((el) => el.type === 'bb-button')
    const phrase = buttons.find((el) => textOf(el.props.children).includes('Use your recovery phrase'))
    expect(phrase).toBeTruthy()
    phrase!.props.onClick()
    expect(used).toEqual(['phrase'])
    expect(buttons.some((el) => textOf(el.props.children).includes('Unlock'))).toBe(true)
  })
})
