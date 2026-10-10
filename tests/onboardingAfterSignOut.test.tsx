/**
 * After a sign-out, "Sign in" reaches the sign-in step, whatever step the onboarding window was left on.
 *
 * The onboarding window is reused, not rebuilt: closing it only hides it (the macOS close policy is Hide for every
 * label), and `open_onboarding_window`, which every "Sign in" calls (Status, the Finder row, the menu), shows and
 * focuses the existing WebView. Its React state survives both, and the onboarding window is not under
 * AccountSessionBoundary (a sign-in in it moves the session revision, so a remount would throw the flow away). So a
 * flow left on "Choose offline folders" came back on that step after a sign-out, with no way to the sign-in form.
 *
 * These tests mount the onboarding window as main.tsx does (StrictMode, ToastProvider, CapabilityProvider, then the
 * capability-gated `Onboarding`) with the real `react-dom` on a minimal DOM, let the window route itself to step 4 the
 * way it does after a sign-in, sign out natively, and reopen the window: a reopen is what a WebView receives when its
 * hidden window is shown and focused again (`visibilitychange`, then `focus`).
 *
 * What this does NOT prove: that a WebView on a Mac delivers those two events on `show()` + `set_focus()`. The device
 * rung is "sign out with onboarding left on step 4, then Status › Sign in: the sign-in form".
 */
import { afterAll, afterEach, beforeAll, describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { allElements, installMiniDom, MiniNode, type MiniDomInstall, type MiniElement } from './fixtures/miniDom'

const capabilities = JSON.parse(readFileSync(new URL('./fixtures/desktop-capabilities.json', import.meta.url), 'utf8')).macos
const LAST_SIGNED_IN = 'sam@example.eu'

// The listeners the app puts on `window`, so a test can deliver what the WebView delivers on a reopen.
const windowEvents = new MiniNode()
let dom: MiniDomInstall
let React: typeof import('react')
let client: typeof import('react-dom/client')
let roots: Array<{ unmount: () => void }> = []

beforeAll(async () => {
  dom = installMiniDom('?window=onboarding', {
    addEventListener: windowEvents.addEventListener.bind(windowEvents),
    removeEventListener: windowEvents.removeEventListener.bind(windowEvents),
  })
  // react-dom decides at load time whether it runs with a DOM, so it is loaded after the DOM is installed.
  React = await import('react')
  client = await import('react-dom/client')
})
afterEach(async () => {
  for (const root of roots) await React.act(async () => root.unmount())
  roots = []
  delete dom.window.__TAURI_INTERNALS__
})
afterAll(() => dom.restore())

/** What Rust holds: whether an account is signed in and unlocked, and the session revision. */
interface Native {
  loggedIn: boolean
  revision: number
  /** How many times `finder_setup_state` was read: the first read says Adding, every later one Ready. */
  finderReads: number
  calls: string[]
}

const finderView = (setup: 'adding' | 'ready') => ({ setup, reason: null, launch_location: 'applications', attempt: 1, max_attempts: 0, last_failure: null })

function installBackend(native: Native) {
  let callbackId = 0
  const handlers: Record<string, (args: any) => unknown> = {
    desktop_capabilities: () => capabilities,
    'plugin:event|listen': () => callbackId,
    'plugin:event|unlisten': () => null,
    desktop_platform: () => 'macos',
    sync_status: () => ({
      logged_in: native.loggedIn,
      session_revision: native.revision,
      vault_unlocked: native.loggedIn,
      engine: native.loggedIn ? 'running' : 'stopped',
      sync_root: null,
      syncing: 0,
      cloud_only: 0,
      conflicts: 0,
      auth_expired: false,
      engine_refusal: null,
    }),
    finder_setup_state: () => finderView(native.finderReads++ === 0 ? 'adding' : 'ready'),
    list_remote_tree: () => [],
    last_signed_in_email: () => LAST_SIGNED_IN,
  }
  dom.window.__TAURI_INTERNALS__ = {
    transformCallback: () => ++callbackId,
    unregisterCallback() {},
    invoke: async (name: string, args: unknown) => {
      native.calls.push(name)
      const handler = handlers[name]
      if (!handler) throw new Error(`unknown command ${name}`)
      return handler(args)
    },
  }
  dom.window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} }
}

/** Mount the onboarding window under the same providers, in the same order, as main.tsx. */
async function mountOnboarding() {
  const { default: Onboarding } = await import('../src/Onboarding')
  const { ToastProvider } = await import('../src/windows/ui')
  const { CapabilityProvider, CapabilityGate } = await import('../src/capabilities')
  const container = dom.document.createElement('div')
  dom.document.body.appendChild(container)
  const root = client.createRoot(container as never)
  roots.push(root)
  await React.act(async () => {
    root.render(
      <React.StrictMode>
        <ToastProvider>
          <CapabilityProvider>
            <CapabilityGate route="onboarding">
              <Onboarding mode="setup" />
            </CapabilityGate>
          </CapabilityProvider>
        </ToastProvider>
      </React.StrictMode>,
    )
  })
  return container
}

async function waitFor(what: string, done: () => boolean, ms = 3000) {
  const deadline = Date.now() + ms
  for (;;) {
    await React.act(async () => { await new Promise((resolve) => setTimeout(resolve, 20)) })
    if (done()) return
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}`)
  }
}

const headings = (container: MiniElement) => allElements(container).filter((el) => el.tagName === 'H1').map((el) => el.textContent.trim())
const activeRail = (container: MiniElement) =>
  allElements(container).filter((el) => el.className.split(' ').includes('step-row') && el.className.split(' ').includes('active')).map((el) => el.textContent)
/** The input of the field labelled "Email" (React sets an input's `type` as a property, so the label finds it). */
const emailField = (container: MiniElement) =>
  allElements(container)
    .filter((el) => el.tagName === 'LABEL' && el.textContent.startsWith('Email'))
    .flatMap((label) => allElements(label).filter((el) => el.tagName === 'INPUT'))[0]

const STEP_4 = 'Start online-only'
const SIGN_IN = 'Welcome back'

/** What a WebView receives when its hidden window is shown and focused again (`open_onboarding_window`). */
async function reopen() {
  await React.act(async () => {
    for (const type of ['visibilitychange', 'focus']) {
      for (const listener of windowEvents.listenersFor(type, false)) listener({ type })
    }
  })
}

/** The window opened after a sign-in routes itself through the Finder step to step 4, as on the device. */
async function onStep4(native: Native) {
  installBackend(native)
  const container = await mountOnboarding()
  await waitFor('step 4, "Choose offline folders"', () => headings(container).includes(STEP_4))
  expect(native.calls).toContain('finder_setup_state')
  return container
}

describe('the onboarding window after a sign-out', () => {
  test('left on step 4, signed out, reopened: the sign-in form, prefilled with the last signed-in address', async () => {
    const native: Native = { loggedIn: true, revision: 10, finderReads: 0, calls: [] }
    const container = await onStep4(native)

    // Settings › Account › Sign out: `clear_session_impl` ends the session and moves the revision twice.
    native.loggedIn = false
    native.revision += 2
    await reopen()
    await waitFor('the sign-in step', () => headings(container).includes(SIGN_IN), 2000).catch(() => undefined)

    expect(headings(container)).toEqual([SIGN_IN])
    expect(activeRail(container)).toHaveLength(1)
    expect(activeRail(container)[0]).toContain('Sign in')
    // The plain sign-out keeps the prefill: the form already knows the address.
    await waitFor('the prefill', () => emailField(container)?.value === LAST_SIGNED_IN, 2000).catch(() => undefined)
    expect(emailField(container)?.value).toBe(LAST_SIGNED_IN)
  })

  test('closed and reopened a second time, still signed out: still the sign-in form', async () => {
    const native: Native = { loggedIn: true, revision: 20, finderReads: 0, calls: [] }
    const container = await onStep4(native)
    native.loggedIn = false
    native.revision += 2
    await reopen()
    await waitFor('the sign-in step', () => headings(container).includes(SIGN_IN), 2000).catch(() => undefined)
    await reopen()
    await waitFor('a second reopen to settle', () => false, 200).catch(() => undefined)
    expect(headings(container)).toEqual([SIGN_IN])
  })

  test('still signed in: a reopen leaves the flow where it was', async () => {
    const native: Native = { loggedIn: true, revision: 30, finderReads: 0, calls: [] }
    const container = await onStep4(native)
    const reads = native.calls.filter((name) => name === 'sync_status').length
    await reopen()
    await waitFor('the reopen to settle', () => false, 300).catch(() => undefined)
    expect(native.calls.filter((name) => name === 'sync_status').length).toBeGreaterThan(reads)
    expect(headings(container)).toEqual([STEP_4])
  })
})

/**
 * The reset's edges, through `OnboardingView`'s real handlers on the component harness (its steps are stand-ins
 * whose props can be read): what it clears, what it must leave alone, and what it is not evidence of.
 */
describe('what the reset after a sign-out clears and keeps', () => {
  const STEP_NAMES = ['SignInStep', 'UnlockStep', 'MacFinderStep', 'FinderInstallStep', 'PinningStep', 'ReadyStep', 'AccountSwitchStep']

  async function openView(status: { current: unknown }) {
    const { mount } = await import('./fixtures/componentHarness')
    const desktopApi = await import('../src/desktopApi')
    const { loadFinderSetup } = await import('../src/finderSetup')
    const byName: Record<string, (props: any) => null> = {}
    for (const name of STEP_NAMES) byName[name] = () => null
    const m = mount('Onboarding.tsx', 'OnboardingView', {
      backend: {
        sync_status: () => { if (status.current instanceof Error) throw status.current; return status.current },
        desktop_platform: () => 'macos',
        finder_setup_state: () => finderView('ready'),
      },
      props: { mode: 'setup' },
      bindings: {
        ...desktopApi,
        loadFinderSetup,
        ...byName,
        getCurrentWindow: () => ({ close: async () => {} }),
        useCapabilities: () => ({ host_os: 'macos' }),
        STEPS: [],
        Wordmark: () => null,
        useRegionLabel: () => 'Stored in the EU',
        FINDER_RAIL_TITLE: '',
        FINDER_RAIL_DETAIL: '',
      },
    })
    // The harness's `window` ignores listeners; record them, so a test can deliver a reopen.
    const listeners: Array<{ type: string; fn: () => void }> = []
    const harnessWindow = (globalThis as any).window
    harnessWindow.addEventListener = (type: string, fn: () => void) => listeners.push({ type, fn })
    harnessWindow.removeEventListener = (type: string, fn: () => void) => {
      const at = listeners.findIndex((l) => l.type === type && l.fn === fn)
      if (at >= 0) listeners.splice(at, 1)
    }
    await m.flush(); await m.flush()
    const find = (name: string) => m.elements().find((el) => el.type === byName[name])
    const shown = () => STEP_NAMES.filter((name) => find(name) !== undefined)
    const reopen = async () => {
      // The harness runs the effects a render queued on the next flush; React runs them right after the commit.
      await m.flush()
      for (const listener of listeners.filter((l) => l.type === 'visibilitychange' || l.type === 'focus')) listener.fn()
      await m.flush(); await m.flush()
    }
    return { m, find, shown, reopen, listeners, close: () => m.close() }
  }

  const status = (loggedIn: boolean) => ({ logged_in: loggedIn, vault_unlocked: loggedIn, session_revision: 1, engine: 'idle', sync_root: null, syncing: 0, cloud_only: 0, conflicts: 0 })

  test('it listens for the reopen on window, and stops when the window unmounts', async () => {
    const v = await openView({ current: status(false) })
    try {
      expect(v.listeners.map((l) => l.type).sort()).toEqual(['focus', 'visibilitychange'])
      v.m.unmount()
      expect(v.listeners).toEqual([])
    } finally { v.close() }
  })

  test('a switch’s typed address survives a reopen: the switch already sits on sign-in (FT-I4)', async () => {
    const backend = { current: status(false) as unknown }
    const v = await openView(backend)
    try {
      v.find('SignInStep')!.props.onDone({ kind: 'account_mismatch', pendingChanges: 2 }, 'b@beebeeb.io')
      await v.m.flush()
      // The warning is up; a reopen while it is up leaves it.
      await v.reopen()
      expect(v.shown()).toEqual(['AccountSwitchStep'])
      v.find('AccountSwitchStep')!.props.onSwitched()
      await v.m.flush()
      expect(v.find('SignInStep')!.props.initialEmail).toBe('b@beebeeb.io')
      const reads = v.m.calls.filter((c) => c.name === 'sync_status').length
      await v.reopen()
      expect(v.shown()).toEqual(['SignInStep'])
      expect(v.find('SignInStep')!.props.initialEmail).toBe('b@beebeeb.io')
      expect(v.m.calls.filter((c) => c.name === 'sync_status').length).toBe(reads)
    } finally { v.close() }
  })

  test('a reset forgets an address an earlier switch carried; the next sign-in sets the rest of the flow afresh', async () => {
    const backend = { current: status(false) as unknown }
    const v = await openView(backend)
    try {
      // A switch carried an address, then the same account signed in with a replaced key: the recovery step says why.
      v.find('SignInStep')!.props.onDone({ kind: 'account_mismatch', pendingChanges: 1 }, 'b@beebeeb.io')
      await v.m.flush()
      v.find('AccountSwitchStep')!.props.onSwitched()
      await v.m.flush()
      backend.current = status(true)
      v.find('SignInStep')!.props.onDone({ kind: 'reauthenticated', vaultUnlocked: false, keyReplaced: true }, 'b@beebeeb.io')
      await v.m.flush()
      expect(v.find('UnlockStep')!.props.keyReplaced).toBe(true)
      // Signed out elsewhere, then reopened.
      backend.current = status(false)
      await v.reopen()
      expect(v.shown()).toEqual(['SignInStep'])
      expect(v.find('SignInStep')!.props.initialEmail).toBeUndefined()
      v.find('SignInStep')!.props.onDone({ kind: 'fresh' }, 'c@beebeeb.io')
      await v.m.flush()
      expect(v.find('UnlockStep')!.props.keyReplaced).toBe(false)
    } finally { v.close() }
  })

  test('a status that cannot be read, or one that is still signed in, is no reason to start over', async () => {
    const backend = { current: status(true) as unknown }
    const v = await openView(backend)
    try {
      expect(v.shown()).toEqual(['ReadyStep'])
      backend.current = new Error('no answer')
      await v.reopen()
      expect(v.shown()).toEqual(['ReadyStep'])
      backend.current = { ...status(true), logged_in: undefined }
      await v.reopen()
      expect(v.shown()).toEqual(['ReadyStep'])
      backend.current = status(true)
      await v.reopen()
      expect(v.shown()).toEqual(['ReadyStep'])
      expect(v.m.calls.filter((c) => c.name === 'sync_status').length).toBeGreaterThanOrEqual(4)
    } finally { v.close() }
  })
})
