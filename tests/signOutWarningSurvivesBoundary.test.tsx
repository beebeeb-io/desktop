/**
 * FB-24 row 12 on a Mac, in React itself: a sign-out that happened but could not confirm that Beebeeb was
 * removed from Finder must still say so after `AccountSessionBoundary` remounts the window.
 *
 * A sign-out always moves the session revision (`set_auth_present(false)` and `set_auth_email(None)` in Rust),
 * the boundary's one-second status poll observes it, and the boundary remounts everything under it. A sentence
 * held in component state was gone within that second, and the remount also put each window back on its
 * first tab or page.
 *
 * These tests mount each window exactly as `main.tsx` does (StrictMode, AccountSessionBoundary, ToastProvider,
 * CapabilityProvider, then the window) with the real `react-dom` on a minimal DOM. They sign out through the
 * real buttons, let the boundary's own poll observe the new revision, check that the window really was
 * remounted (the nodes it showed before are detached), and only then read the screen.
 *
 * What this does NOT prove: layout, or anything on a Mac. The device rung is "Settings › Account sign-out with
 * an unconfirmed Finder removal: the sentence stays".
 */
import { afterAll, afterEach, beforeAll, describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import type { ReactElement } from 'react'
import { allElements, click, installMiniDom, type MiniDomInstall, type MiniElement } from './fixtures/miniDom'
import { rustStr } from './fixtures/rustConstants'

const SIGN_OUT_WARNING = { code: 'finder_removal_unconfirmed', sentence: rustStr('lib.rs', 'FINDER_SIGN_OUT_UNCONFIRMED_WARNING') }
const capabilities = JSON.parse(readFileSync(new URL('./fixtures/desktop-capabilities.json', import.meta.url), 'utf8')).macos

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
  for (const root of roots) await React.act(async () => root.unmount())
  roots = []
  delete dom.window.__TAURI_INTERNALS__
})
afterAll(() => dom.restore())

/** What Rust holds for this WebView: whether an account is signed in, and the session revision. */
interface Native {
  loggedIn: boolean
  revision: number
  calls: string[]
}

/** The commands both windows send, answered from `native`; anything else is an IPC error, as an unknown command is. */
function installBackend(native: Native, extra: Record<string, (args: any) => unknown> = {}) {
  let callbackId = 0
  const snapshot = () => ({
    phase: 'synced',
    generated_at: 1000,
    account: { email: native.loggedIn ? 'sam@example.eu' : null, logged_in: native.loggedIn, vault_unlocked: native.loggedIn, auth_expired: false },
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
    desktop_capabilities: () => capabilities,
    'plugin:event|listen': () => callbackId,
    'plugin:event|unlisten': () => null,
    get_desktop_config: () => ({ upload_kbps_limit: 0, download_kbps_limit: 0, pause_sync: false, notify_conflicts: true, notify_sync_complete: false, notify_quota_warnings: true, theme: 'dark', local_cache_limit_bytes: 0 }),
    consume_menu_update_check: () => false,
    popover_snapshot: snapshot,
    account_subscription: () => ({ plan: 'basic', billing_cycle: 'yearly', status: 'active', seats: 1, region: 'eu', current_period_end: '2026-10-14T12:00:00Z', quota_bytes: 1, used_bytes: 0 }),
    account_email: () => (native.loggedIn ? 'sam@example.eu' : null),
    autostart_enabled: () => false,
    desktop_platform: () => 'macos',
    app_version: () => '0.1.0',
    // A sign-out that happened but could not confirm the Finder removal. Like `clear_session_impl`, it moves the
    // revision twice as its last steps (auth flag, then the email) and answers Ok with the warning.
    clear_session: () => {
      native.loggedIn = false
      native.revision += 2
      return { warning: SIGN_OUT_WARNING }
    },
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
}

/** Mount `window` under the same providers, in the same order, as main.tsx. */
async function mountWindow(search: string, window: ReactElement) {
  const { default: AccountSessionBoundary } = await import('../src/AccountSessionBoundary')
  const { ToastProvider } = await import('../src/windows/ui')
  const { CapabilityProvider } = await import('../src/capabilities')
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

/** Let React and the backend run (real timers: the boundary's poll is a real one-second timeout). */
async function waitFor(what: string, done: () => boolean, ms = 3000) {
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
const statusLines = (container: MiniElement) =>
  elements(container).filter((el) => el.getAttribute('role') === 'status').map((el) => el.textContent.trim())
const press = (container: MiniElement, tag: string, text: string) => React.act(async () => { click(container, byText(container, tag, text)) })

/** The window really was remounted: the node it showed before the sign-out is no longer in it. */
async function waitForRemount(container: MiniElement, before: MiniElement, native: Native) {
  const { accountSessionRevision } = await import('../src/accountSession')
  await waitFor('the boundary to observe the sign-out and remount the window', () => accountSessionRevision() === native.revision && !container.contains(before))
}

/** Whatever the remounted window shows, it has resolved its capabilities and finished loading. */
async function waitForSettled(container: MiniElement, shown: (container: MiniElement) => boolean) {
  await waitFor('the remounted window to settle', () =>
    shown(container) && !container.textContent.includes('Checking device support…') && !container.textContent.includes('Loading…'))
}

/** Each test waits for at least one real status poll (one second); bun's default five seconds is too tight. */
const SLOW = 15_000

describe('a sign-out with an unconfirmed Finder removal, through AccountSessionBoundary', () => {
  test('main.tsx mounts both windows under the boundary in the order these tests use', () => {
    const main = readFileSync(new URL('../src/main.tsx', import.meta.url), 'utf8')
    expect(main).toContain('<AccountSessionBoundary><ToastProvider><CapabilityProvider>{component}</CapabilityProvider></ToastProvider></AccountSessionBoundary>')
    expect(main).toContain("which === 'settings-v2' && platform === 'macos'")
    expect(main).toMatch(/<StrictMode>\s*\{which === 'onboarding'/)
  })

  test('Settings › Account: the sentence is still on screen after the boundary remounts the window', async () => {
    const native: Native = { loggedIn: true, revision: 10, calls: [] }
    installBackend(native)
    const { default: MacSettings } = await import('../src/MacSettings')
    const container = await mountWindow('?window=settings-v2&platform=macos', <MacSettings />)
    await waitFor('the window', () => elements(container).some((el) => el.getAttribute('role') === 'tab'))
    await press(container, 'button', 'Account')
    await waitFor('the account', () => container.textContent.includes('sam@example.eu'))

    // Only the boundary replaces this node; the sign-out itself re-renders it in place.
    const shownBefore = elements(container).find((el) => el.className === 'ms-pane')!
    await press(container, 'button', 'Sign out…')
    await press(container, 'button', 'Sign out')
    await waitFor('the sign-out', () => native.calls.includes('clear_session') && container.textContent.includes('You’re signed out'))
    expect(statusLines(container)).toContain(SIGN_OUT_WARNING.sentence)

    await waitForRemount(container, shownBefore, native)
    await waitForSettled(container, (c) => elements(c).some((el) => el.getAttribute('role') === 'tab'))

    expect(statusLines(container)).toContain(SIGN_OUT_WARNING.sentence)
    const selected = elements(container).filter((el) => el.getAttribute('role') === 'tab' && el.getAttribute('aria-selected') === 'true')
    expect(selected.map((el) => el.textContent.trim())).toEqual(['Account'])
    // Said neutrally: no alert, no "Couldn’t …".
    expect(elements(container).filter((el) => el.getAttribute('role') === 'alert')).toEqual([])
    expect(container.textContent).not.toContain('Couldn’t sign out')
  }, SLOW)

  test('the compact window’s Account page: the sentence is still on screen after the boundary remounts the window', async () => {
    const native: Native = { loggedIn: true, revision: 20, calls: [] }
    installBackend(native)
    const { default: App } = await import('../src/App')
    const container = await mountWindow('?platform=macos', <App />)
    await waitFor('the window', () => container.textContent.includes('Account & security'))
    await React.act(async () => {
      click(container, elements(container).find((el) => el.tagName === 'BUTTON' && el.textContent.includes('Account & security'))!)
    })
    await waitFor('the account', () => container.textContent.includes('sam@example.eu'))

    const shownBefore = elements(container).find((el) => el.tagName === 'SECTION' && el.className === 'page')!
    await press(container, 'button', 'Sign out')
    await waitFor('the sign-out', () => native.calls.includes('clear_session') && statusLines(container).includes(SIGN_OUT_WARNING.sentence))

    await waitForRemount(container, shownBefore, native)
    await waitForSettled(container, (c) => elements(c).some((el) => el.tagName === 'SECTION' && el.className === 'page'))

    expect(statusLines(container)).toContain(SIGN_OUT_WARNING.sentence)
    expect(byText(container, 'h1', 'Account & security')).toBeDefined()
    expect(container.textContent).not.toContain('Couldn’t sign out')
  }, SLOW)

  test('the next sign-in clears the sentence (and the window opens where it normally does)', async () => {
    const native: Native = { loggedIn: true, revision: 30, calls: [] }
    installBackend(native)
    const { default: MacSettings } = await import('../src/MacSettings')
    const container = await mountWindow('?window=settings-v2&platform=macos', <MacSettings />)
    await waitFor('the window', () => elements(container).some((el) => el.getAttribute('role') === 'tab'))
    await press(container, 'button', 'Account')
    await waitFor('the account', () => container.textContent.includes('sam@example.eu'))
    const signedOut = elements(container).find((el) => el.className === 'ms-pane')!
    await press(container, 'button', 'Sign out…')
    await press(container, 'button', 'Sign out')
    await waitFor('the sign-out', () => native.calls.includes('clear_session'))
    await waitForRemount(container, signedOut, native)
    await waitFor('the remounted window', () => statusLines(container).includes(SIGN_OUT_WARNING.sentence))

    // A sign-in in another window: Rust moves the revision again and the account is signed in.
    const beforeSignIn = elements(container).find((el) => el.className === 'ms-pane')!
    native.loggedIn = true
    native.revision += 2
    await waitForRemount(container, beforeSignIn, native)
    await waitFor('the window after the sign-in', () => elements(container).some((el) => el.getAttribute('role') === 'tab'))

    expect(container.textContent).not.toContain(SIGN_OUT_WARNING.sentence)
    const selected = elements(container).filter((el) => el.getAttribute('role') === 'tab' && el.getAttribute('aria-selected') === 'true')
    expect(selected.map((el) => el.textContent.trim())).toEqual(['General'])
  }, SLOW)
})
