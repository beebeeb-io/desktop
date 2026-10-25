/**
 * The sign-in flow under the REAL `AccountSessionBoundary` (task 1683 slice 2, fix round after the /code-review of PR #143).
 *
 * The boundary remounts everything under it whenever the session revision moves, and a sign-out and a sign-in both move
 * it. The component harness (`componentHarness.ts`) executes one declaration with its own hooks, so it cannot see a
 * remount; these tests mount `MacApp` exactly as `main.tsx` does (StrictMode, boundary, toasts, capabilities) with the
 * real `react-dom` on the mini DOM of `signOutWarningSurvivesBoundary.test.tsx`, move the revision as Rust does, wait
 * for the boundary's own one-second poll to remount the window, and only then read the screen.
 *
 * - P2-1: “Sign out and switch” with an unconfirmed Finder removal (FB-24) held its sentence in a toast, which the
 *   remount wiped. The sentence is held (accountSession.ts) and the signed-out card shows it.
 * - P2-2: the account switch card asks about the account that is still on this Mac. When that account signed out
 *   another way (the app menu), the remount must not bring the stale card back; the address typed for the next
 *   sign-in stays.
 * - P3-1: `open_reauth_window` asks for “Sign in again” whatever the session says.
 *
 * What this does NOT prove: layout, or anything on a Mac. The device rung is the same sign-out, on the signed build.
 */
import { afterAll, afterEach, beforeAll, describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import type { ReactElement } from 'react'
import { accountSessionRevision, clearSignOutWarning, heldSignOutWarning } from '../src/accountSession'
import { IDLE_FLOW } from '../src/signInModel'
import { resetSignInFlow, setSignInFlow } from '../src/signInFlowStore'
import { allElements, click, installMiniDom, type MiniDomInstall, type MiniElement } from './fixtures/miniDom'
import { rustStr } from './fixtures/rustConstants'

const SENTENCE = rustStr('lib.rs', 'FINDER_SIGN_OUT_UNCONFIRMED_WARNING')
const capabilities = JSON.parse(readFileSync(new URL('./fixtures/desktop-capabilities.json', import.meta.url), 'utf8')).macos
const ASK = { askId: 'a1', files: 2, copiedFiles: 2, others: 1, missingFiles: 0, earlierFolders: [] as string[] }

let dom: MiniDomInstall
let React: typeof import('react')
let client: typeof import('react-dom/client')
let roots: Array<{ unmount: () => void }> = []

beforeAll(async () => {
  dom = installMiniDom('')
  // react-dom decides at load time whether it runs with a DOM, so it is loaded after the DOM is installed.
  React = await import('react')
  client = await import('react-dom/client')
})
afterEach(async () => {
  ;(globalThis as any).IS_REACT_ACT_ENVIRONMENT = true
  for (const root of roots) await React.act(async () => root.unmount())
  roots = []
  delete dom.window.__TAURI_INTERNALS__
  clearSignOutWarning()
  resetSignInFlow()
})
afterAll(() => dom.restore())

/** What Rust holds for this WebView. */
interface Native {
  loggedIn: boolean
  authExpired: boolean
  revision: number
  calls: string[]
  /** The next `main:navigate` request the window pulls. */
  nav: unknown
  /** Signed in with the vault locked (no key in memory). */
  locked?: boolean
  /** What Rust's `pending_setup` answers. */
  pending?: { key_replaced: boolean } | null
}

const statusAnswer = (n: Native) => ({
  logged_in: n.loggedIn,
  session_revision: n.revision,
  vault_unlocked: n.loggedIn && !n.locked,
  engine: n.loggedIn ? 'running' : 'stopped',
  sync_root: null,
  syncing: 0,
  cloud_only: 0,
  conflicts: 0,
  auth_expired: n.authExpired,
  engine_refusal: null,
})

function installBackend(native: Native, extra: Record<string, (args: any) => unknown> = {}) {
  let callbackId = 0
  const snapshot = () => ({
    phase: 'synced',
    // Rust's sync part of the state (task 2179): the contract refuses a snapshot without it.
    sync_phase: 'synced',
    generated_at: 1000,
    account: { email: native.loggedIn ? 'sam@example.eu' : null, logged_in: native.loggedIn, vault_unlocked: native.loggedIn && !native.locked, auth_expired: native.authExpired },
    paused: false,
    engine: { state: 'idle', files_remaining: 0, bytes_total: 0, bytes_done: 0, last_tick_ok_at: 990 },
    reason: null,
    finder: { setup: 'ready', reason: null, reason_line: null },
    storage: native.loggedIn ? { used_bytes: 84_300_000_000, quota_bytes: 200_000_000_000, fetched_at: 900, stale: false } : null,
    storage_full: false,
    pending_changes: 0,
    conflicts: { count: 0, files: [] },
    activity: [],
  })
  const handlers: Record<string, (args: any) => unknown> = {
    sync_status: () => statusAnswer(native),
    desktop_capabilities: () => capabilities,
    'plugin:event|listen': () => callbackId,
    'plugin:event|unlisten': () => null,
    get_desktop_config: () => ({ upload_kbps_limit: 0, download_kbps_limit: 0, pause_sync: false, notify_conflicts: true, notify_sync_complete: false, notify_quota_warnings: true, theme: 'dark', locale: 'en' }),
    consume_menu_update_check: () => false,
    consume_main_navigation: () => {
      const nav = native.nav
      native.nav = null
      return nav
    },
    main_window_state: () => null,
    finder_setup_state: () => ({ setup: 'ready', reason: null, launch_location: 'applications', attempt: 1, max_attempts: 1, last_failure: null }),
    kept_unsynced_folder: () => null,
    kept_unsent_folder: () => null,
    popover_snapshot: snapshot,
    account_subscription: () => ({ plan: 'basic', billing_cycle: 'yearly', status: 'active', seats: 1, region: 'eu', current_period_end: '2026-10-14T12:00:00Z', quota_bytes: 1, used_bytes: 0 }),
    account_email: () => (native.loggedIn ? 'sam@example.eu' : null),
    last_signed_in_email: () => null,
    pending_setup: () => native.pending ?? null,
    set_pending_setup: () => null,
    autostart_enabled: () => false,
    desktop_platform: () => 'macos',
    app_version: () => '0.1.0',
    consume_sign_out_flow_request: () => false,
    sign_out_count: () => ({ askId: 'a0', files: 0, copiedFiles: 0, others: 0, missingFiles: 0, earlierFolders: [], canSend: false, notSending: null }),
    sign_out_result_ack: () => null,
    ...extra,
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
  return handlers
}

/** Mount `window` under the same providers, in the same order, as main.tsx. */
async function mountWindow(search: string, window: ReactElement) {
  const { default: AccountSessionBoundary } = await import('../src/AccountSessionBoundary')
  const { ToastProvider } = await import('../src/windows/ui')
  const { CapabilityProvider } = await import('../src/capabilities')
  expect(heldSignOutWarning()).toBeNull()
  dom.window.location.search = search
  const container = dom.document.createElement('div')
  dom.document.body.appendChild(container)
  const root = client.createRoot(container as never)
  roots.push(root)
  await React.act(async () => {
    root.render(
      <React.StrictMode>
        <AccountSessionBoundary><ToastProvider><CapabilityProvider>{window}</CapabilityProvider></ToastProvider></AccountSessionBoundary>
      </React.StrictMode>,
    )
  })
  return container
}

async function waitFor(what: string, done: () => boolean, ms = 4000) {
  const deadline = Date.now() + ms
  for (;;) {
    await React.act(async () => { await new Promise((resolve) => setTimeout(resolve, 20)) })
    if (done()) return
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}`)
  }
}

const elements = (container: MiniElement) => allElements(container)
const byText = (container: MiniElement, tag: string, text: string) => {
  const hits = elements(container).filter((el) => el.tagName === tag.toUpperCase() && el.textContent.trim() === text)
  if (hits.length !== 1) throw new Error(`expected one <${tag}> "${text}", found ${hits.length}`)
  return hits[0]
}
const press = (container: MiniElement, tag: string, text: string) => React.act(async () => { click(container, byText(container, tag, text)) })
const headings = (container: MiniElement) => elements(container).filter((el) => el.tagName === 'H1').map((el) => el.textContent.trim())
const statusLines = (container: MiniElement) => elements(container).filter((el) => el.getAttribute('role') === 'status').map((el) => el.textContent.trim())
const isSignInCard = (el: MiniElement) => String(el.className).split(/\s+/).includes('mw-scard')
const card = (container: MiniElement) => elements(container).find(isSignInCard)

/** The window really was remounted: the node it showed before is no longer in it. */
async function waitForRemount(container: MiniElement, before: MiniElement, native: Native) {
  await waitFor('the boundary to observe the new revision and remount the window', () => accountSessionRevision() === native.revision && !container.contains(before))
}
async function settled(container: MiniElement, shown: (container: MiniElement) => boolean) {
  await waitFor('the remounted window to settle', () => shown(container) && !container.textContent.includes('Checking device support…') && !container.textContent.includes('Loading…'))
}

const SLOW = 15_000
const SWITCH_TITLE = 'Switch accounts with changes not sent?'

describe('the account switch under the boundary (P2-1, P2-2)', () => {
  // P2-1. Rust's clear_session answers Ok with the FB-24 warning, and the revision has moved by then.
  test('“Sign out and switch” with an unconfirmed Finder removal: the sentence is on the signed-out card after the remount', async () => {
    const native: Native = { loggedIn: true, authExpired: true, revision: 10, calls: [], nav: null }
    installBackend(native, {
      clear_session: () => {
        native.loggedIn = false
        native.authExpired = false
        native.revision += 2
        return { warning: { code: 'finder_removal_unconfirmed', sentence: SENTENCE } }
      },
    })
    setSignInFlow({ ...IDLE_FLOW, step: 'switch', switch: { pendingChanges: 2, unsent: ASK, unsentError: null }, email: 'b@example.eu' })
    const { default: MacApp } = await import('../src/MacApp')
    const container = await mountWindow('?window=main&platform=macos', <MacApp />)
    await waitFor('the switch card', () => headings(container).some((h) => h.length > 0) && card(container) !== undefined)
    const shownBefore = card(container)!

    await press(container, 'button', 'Sign out and switch')
    await waitFor('the sign-out', () => native.calls.includes('clear_session'))
    await waitForRemount(container, shownBefore, native)
    await settled(container, (c) => card(c) !== undefined && !c.textContent.includes(SWITCH_TITLE))

    expect(heldSignOutWarning()).toBe(SENTENCE)
    expect(statusLines(container)).toContain(SENTENCE)
    // Said neutrally, above the sign-in card, not left behind in a toast.
    const text = container.textContent
    expect(text.indexOf(SENTENCE)).toBeLessThan(text.indexOf('Sign in'))
    expect(elements(container).filter((el) => el.getAttribute('role') === 'alert')).toEqual([])
    // The window did not leave the card for Settings to show it (MacApp's follow-the-note rule is for a window with no card).
    expect(headings(container)).not.toContain('Account')
    expect(native.calls.filter((name) => name === 'sign_out_result_ack')).toHaveLength(1)
  }, SLOW)

  test('a sign-out that failed AFTER the removal holds its sentence too: the account is out, the toast would be gone', async () => {
    const failure = 'Sign-out stopped after Beebeeb was removed from Finder. Restart Beebeeb and sign in again.'
    const native: Native = { loggedIn: true, authExpired: true, revision: 20, calls: [], nav: null }
    installBackend(native, {
      clear_session: () => {
        native.loggedIn = false
        native.authExpired = false
        native.revision += 2
        throw failure
      },
    })
    setSignInFlow({ ...IDLE_FLOW, step: 'switch', switch: { pendingChanges: 2, unsent: ASK, unsentError: null }, email: 'b@example.eu' })
    const { default: MacApp } = await import('../src/MacApp')
    const container = await mountWindow('?window=main&platform=macos', <MacApp />)
    await waitFor('the switch card', () => card(container) !== undefined)
    const shownBefore = card(container)!

    await press(container, 'button', 'Sign out and switch')
    await waitFor('the sign-out', () => native.calls.includes('clear_session'))
    await waitForRemount(container, shownBefore, native)
    await settled(container, (c) => card(c) !== undefined && !c.textContent.includes(SWITCH_TITLE))

    expect(heldSignOutWarning()).toBe(failure)
    expect(statusLines(container)).toContain(failure)
  }, SLOW)

  // P2-2. The account left another way (the app menu's Sign Out): nothing in this card ran.
  test('the account signs out another way while the switch card is up: the remount shows sign-in, not the stale card, with the typed address', async () => {
    const native: Native = { loggedIn: true, authExpired: true, revision: 30, calls: [], nav: null }
    installBackend(native)
    setSignInFlow({ ...IDLE_FLOW, step: 'switch', switch: { pendingChanges: 3, unsent: ASK, unsentError: null }, email: 'b@example.eu' })
    const { default: MacApp } = await import('../src/MacApp')
    const container = await mountWindow('?window=main&platform=macos', <MacApp />)
    await waitFor('the switch card', () => card(container) !== undefined && container.textContent.includes('Sign out and switch'))
    const shownBefore = card(container)!

    // Rust: the app menu signed account A out. The revision moves; the flow in the store still says “switch”.
    native.loggedIn = false
    native.authExpired = false
    native.revision += 2
    await waitForRemount(container, shownBefore, native)
    await settled(container, (c) => card(c) !== undefined && !c.textContent.includes(SWITCH_TITLE))

    expect(container.textContent).not.toContain(SWITCH_TITLE)
    expect(container.textContent).not.toContain('Sign out and switch')
    // FT-I4: the address typed for the next sign-in is still there: the sign-in opens on the email form (“Welcome back”) with it.
    expect(headings(container)).toContain('Welcome back')
    const emailField = elements(container).find((el) => el.tagName === 'INPUT' && (el as any).value === 'b@example.eu')
    expect(emailField).toBeDefined()
  }, SLOW)

  test('the account is still here after an unrelated revision change: the switch question stands', async () => {
    const native: Native = { loggedIn: true, authExpired: true, revision: 40, calls: [], nav: null }
    installBackend(native)
    setSignInFlow({ ...IDLE_FLOW, step: 'switch', switch: { pendingChanges: 3, unsent: ASK, unsentError: null }, email: 'b@example.eu' })
    const { default: MacApp } = await import('../src/MacApp')
    const container = await mountWindow('?window=main&platform=macos', <MacApp />)
    await waitFor('the switch card', () => card(container) !== undefined && container.textContent.includes('Sign out and switch'))
    const shownBefore = card(container)!

    native.revision += 2 // e.g. an engine restart that renames the session: still signed in
    await waitForRemount(container, shownBefore, native)
    await settled(container, (c) => card(c) !== undefined)

    expect(container.textContent).toContain('Sign out and switch')
  }, SLOW)
})

describe('a window reopened while Set up this Mac was owed (P2-3)', () => {
  // ⌘W destroyed the earlier window and its flow store; the session stayed and the key was never delivered. The new window
  // (this mount: a fresh module store, as a fresh WebView would have) asks Rust and resumes there instead of drawing a Home
  // whose locked card has nothing to unlock from.
  test('Rust still holds the step: the window opens on Set up this Mac, with no Home drawn first', async () => {
    const native: Native = { loggedIn: true, authExpired: false, revision: 70, calls: [], nav: null, locked: true, pending: { key_replaced: false } }
    installBackend(native)
    const { default: MacApp } = await import('../src/MacApp')
    const container = await mountWindow('?window=main&platform=macos', <MacApp />)
    await settled(container, (c) => card(c) !== undefined)
    expect(headings(container)).toContain('Set up this Mac')
    expect(container.textContent).not.toContain('Your vault is locked')
    expect(native.calls.filter((name) => name === 'pending_setup')).toHaveLength(1)
  }, SLOW)

  test('nothing is owed (keys are in the Keychain): Home with the locked card, and it asked once', async () => {
    const native: Native = { loggedIn: true, authExpired: false, revision: 80, calls: [], nav: null, locked: true, pending: null }
    installBackend(native)
    const { default: MacApp } = await import('../src/MacApp')
    const container = await mountWindow('?window=main&platform=macos', <MacApp />)
    await waitFor('Home', () => elements(container).some((el) => el.tagName === 'NAV') && container.textContent.includes('Your vault is locked'))
    expect(card(container)).toBeUndefined()
    expect(native.calls.filter((name) => name === 'pending_setup')).toHaveLength(1)
  }, SLOW)
})

describe('⌘W on Set up this Mac, reached from Home’s locked card (delta review P3-1)', () => {
  // The sign-in's own step tells Rust (`moveFlow`); “Use your recovery phrase” entered the same card with a direct
  // `setSignInFlow`, so Rust never learned of the step and a window reopened after ⌘W showed Home again.
  test('Unlock vault finds no key, “Use your recovery phrase” opens the card, ⌘W, and the reopened window resumes there', async () => {
    const noKey = 'This Mac does not have your encryption keys yet. Restore them to continue.'
    const native: Native = { loggedIn: true, authExpired: false, revision: 90, calls: [], nav: null, locked: true, pending: null }
    installBackend(native, {
      unlock_vault: () => { throw noKey },
      // Rust's `set_pending_setup`: the step is kept for the process.
      set_pending_setup: (args: { keyReplaced: boolean }) => { native.pending = { key_replaced: args.keyReplaced }; return null },
    })
    const { default: MacApp } = await import('../src/MacApp')
    const container = await mountWindow('?window=main&platform=macos', <MacApp />)
    await waitFor('Home’s locked card', () => container.textContent.includes('Your vault is locked'))
    expect(native.pending).toBeNull()

    await press(container, 'button', 'Unlock vault')
    await waitFor('the no-key offer', () => container.textContent.includes('Use your recovery phrase'))
    await press(container, 'button', 'Use your recovery phrase')
    await settled(container, (c) => card(c) !== undefined)
    expect(headings(container)).toContain('Set up this Mac')
    expect(native.pending).toEqual({ key_replaced: false })

    // ⌘W destroys the window and, with it, the WebView's module store: a window opened again starts with none.
    for (const root of roots) await React.act(async () => root.unmount())
    roots = []
    resetSignInFlow()
    native.calls.length = 0
    const reopened = await mountWindow('?window=main&platform=macos', <MacApp />)
    await settled(reopened, (c) => card(c) !== undefined)
    expect(headings(reopened)).toContain('Set up this Mac')
    expect(reopened.textContent).not.toContain('Your vault is locked')
    expect(native.calls.filter((name) => name === 'pending_setup')).toHaveLength(1)
  }, SLOW)
})

describe('“Sign in again” asked for by an opener (P3-1)', () => {
  test('a signed-in window whose session is not marked ended still shows the card when open_reauth_window asks', async () => {
    const native: Native = { loggedIn: true, authExpired: false, revision: 50, calls: [], nav: { to: 'reauth' } }
    installBackend(native)
    const { default: MacApp } = await import('../src/MacApp')
    const container = await mountWindow('?window=main&platform=macos', <MacApp />)
    await settled(container, (c) => card(c) !== undefined)

    expect(headings(container)).toContain('Sign in again')
    // Not a sign-in: the account is on this Mac, so the footer names it.
    expect(container.textContent).toContain('sam@example.eu')
  }, SLOW)

  // Delta review P3-2: the forced card has no Cancel; with a session that is fine it has a quiet way back.
  test('the card of a session that is fine has “Back to Home”, which leaves it; a really ended session has none', async () => {
    const native: Native = { loggedIn: true, authExpired: false, revision: 55, calls: [], nav: { to: 'reauth' } }
    installBackend(native)
    const { default: MacApp } = await import('../src/MacApp')
    const container = await mountWindow('?window=main&platform=macos', <MacApp />)
    await settled(container, (c) => card(c) !== undefined)
    expect(headings(container)).toContain('Sign in again')
    await press(container, 'button', 'Back to Home')
    await waitFor('Home', () => elements(container).some((el) => el.tagName === 'NAV'))
    expect(headings(container)).not.toContain('Sign in again')

    const ended: Native = { loggedIn: true, authExpired: true, revision: 56, calls: [], nav: { to: 'reauth' } }
    installBackend(ended)
    for (const root of roots) await React.act(async () => root.unmount())
    roots = []
    resetSignInFlow()
    const second = await mountWindow('?window=main&platform=macos', <MacApp />)
    await settled(second, (c) => card(c) !== undefined)
    expect(headings(second)).toContain('Sign in again')
    expect(elements(second).some((el) => el.tagName === 'BUTTON' && el.textContent.trim() === 'Back to Home')).toBe(false)
  }, SLOW)

  test('without the request the same window is Home', async () => {
    const native: Native = { loggedIn: true, authExpired: false, revision: 60, calls: [], nav: null }
    installBackend(native)
    const { default: MacApp } = await import('../src/MacApp')
    const container = await mountWindow('?window=main&platform=macos', <MacApp />)
    await waitFor('Home', () => elements(container).some((el) => el.tagName === 'NAV'))
    expect(headings(container)).not.toContain('Sign in again')
    expect(card(container)).toBeUndefined()
  }, SLOW)
})
