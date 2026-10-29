/**
 * Task 1747 Part 2 (plan D4, items 7, 9, 10 and 14 at the window): a signed-in, locked Mac with no vault key anywhere opens on
 * Set up this Mac BY STATE, with no click and no flash of Home; with the key in the Keychain it opens on Home's locked card.
 * The interrupted first run (a password sign-in on a Mac with no key, the window closed or the app quit before the phrase)
 * is exactly the first case, and it is reproduced here without any pending step in Rust: after a quit there is none.
 *
 * Mounted with the real `react-dom` on the minimal DOM and on bun's fake clock, like `macAppLockFlash.test.tsx`: `vault_unlock_route`
 * and `pending_setup` take half a second to answer here, so a flash is something a frame can show, and every frame is checked.
 * What this does NOT prove: anything on a Mac (the Keychain read itself is `lib.rs`'s `keychain_vault_key_present`).
 */
import { afterAll, afterEach, beforeAll, describe, expect, jest, test } from 'bun:test'
import { IDLE_FLOW } from '../src/signInModel'
import { getSignInFlow, resetSignInFlow, setSignInFlow } from '../src/signInFlowStore'
import { allElements, click, installMiniDom, type MiniDomInstall, type MiniElement } from './fixtures/miniDom'

let dom: MiniDomInstall
let React: typeof import('react')
let client: typeof import('react-dom/client')

beforeAll(async () => {
  dom = installMiniDom('?window=main&platform=macos')
  React = await import('react')
  client = await import('react-dom/client')
  ;(globalThis as any).IS_REACT_ACT_ENVIRONMENT = true
})
afterEach(() => {
  jest.useRealTimers()
  delete dom.window.__TAURI_INTERNALS__
  resetSignInFlow()
})
afterAll(() => dom.restore())

const SET_UP = 'Set up this Mac'
const LOCKED = 'Your vault is locked'

const account = (unlocked: boolean) => ({ email: 'sam@example.eu', logged_in: true, vault_unlocked: unlocked, auth_expired: false })
const snapshot = (unlocked: boolean) => ({
  phase: unlocked ? 'synced' : 'locked', generated_at: 1000, paused: false, account: account(unlocked),
  engine: { state: 'idle', files_remaining: 0, bytes_total: 0, bytes_done: 0, last_tick_ok_at: null },
  reason: null, finder: { setup: 'ready', reason: null, reason_line: null },
  storage: unlocked ? { used_bytes: 84_300_000_000, quota_bytes: 200_000_000_000, fetched_at: 900, stale: false } : null,
  storage_full: false, pending_changes: 0, conflicts: { count: 0, files: [] }, activity: [],
})

interface Script {
  /** Whether the n-th `popover_snapshot` answers unlocked; the last entry repeats. */
  reads: boolean[]
  route: 'keychain' | 'recovery_phrase' | (() => never)
  pending?: { key_replaced: boolean } | null
  /** What Finder reports; `adding` keeps the Finder screen up until the person moves on. */
  finder?: 'ready' | 'adding'
}

function installBackend(script: Script) {
  const calls: string[] = []
  let read = 0
  let callbackId = 0
  const slow = <T,>(value: () => T) => new Promise<T>((resolve, reject) => setTimeout(() => { try { resolve(value()) } catch (e) { reject(e) } }, 500))
  const handlers: Record<string, (args: any) => unknown> = {
    popover_snapshot: () => snapshot(script.reads[Math.min(read++, script.reads.length - 1)]),
    pending_setup: () => slow(() => script.pending ?? null),
    vault_unlock_route: () => slow(() => (typeof script.route === 'function' ? script.route() : script.route)),
    set_pending_setup: () => null,
    account_subscription: () => ({ plan: 'basic', billing_cycle: 'yearly', status: 'active', seats: 1, region: 'eu', current_period_end: null, quota_bytes: 1, used_bytes: 0 }),
    consume_main_navigation: () => null,
    consume_menu_update_check: () => false,
    consume_sign_out_flow_request: () => false,
    main_window_state: () => null,
    get_desktop_config: () => ({ upload_kbps_limit: 0, download_kbps_limit: 0, pause_sync: false, notify_conflicts: true, notify_sync_complete: false, notify_quota_warnings: true, theme: 'dark', locale: 'en' }),
    finder_setup_state: () => ({ setup: script.finder ?? 'ready', reason: null, launch_location: 'applications', attempt: 1, max_attempts: 1, last_failure: null }),
    sync_status: () => ({ logged_in: true, session_revision: 1, vault_unlocked: true, engine: 'running', sync_root: null, syncing: 0, cloud_only: 0, conflicts: 0, auth_expired: false, engine_refusal: null }),
    kept_unsent_folder: () => null,
    kept_unsynced_folder: () => null,
    'plugin:event|listen': () => ++callbackId,
    'plugin:event|unlisten': () => null,
  }
  dom.window.__TAURI_INTERNALS__ = {
    transformCallback: () => ++callbackId,
    unregisterCallback() {},
    invoke: async (name: string, args: unknown) => {
      calls.push(name)
      const handler = handlers[name]
      if (!handler) throw new Error(`unknown command ${name}`)
      return handler(args)
    },
  }
  dom.window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} }
  return calls
}

async function mountMacApp(): Promise<{ container: MiniElement; unmount: () => Promise<void> }> {
  const { default: MacApp } = await import('../src/MacApp')
  const { ToastProvider } = await import('../src/windows/ui')
  const container = dom.document.createElement('div')
  dom.document.body.appendChild(container)
  const root = client.createRoot(container as never)
  await React.act(async () => {
    root.render(<React.StrictMode><ToastProvider><MacApp /></ToastProvider></React.StrictMode>)
  })
  return { container, unmount: () => React.act(async () => root.unmount()) }
}

/** Move the fake clock on in 250 ms steps, letting React and the backend's promises run, and hand every frame's text to `frame`. */
async function pass(ms: number, container: MiniElement, frame: (text: string) => void) {
  for (let t = 0; t < ms; t += 250) {
    await React.act(async () => {
      jest.advanceTimersByTime(250)
      for (let i = 0; i < 10; i++) await Promise.resolve()
    })
    frame(container.textContent)
  }
}

describe('a window that opens signed in and locked follows the Keychain route (1747 Part 2)', () => {
  // Item 7 at the window, and the interrupted first run: a session in the Keychain, no key, no pending step in Rust (a quit
  // forgot it), a fresh flow store. Today this is Home's locked card with an Unlock vault that has no key to read.
  test('no key anywhere: Set up this Mac, by state, with no click; never Home, never a frame of the locked card', async () => {
    jest.useFakeTimers()
    const calls = installBackend({ reads: [false], route: 'recovery_phrase' })
    const { container, unmount } = await mountMacApp()
    const frames: string[] = []
    await pass(2000, container, (text) => frames.push(text))
    expect(container.textContent).toContain(SET_UP)
    expect(frames.filter((text) => text.includes(LOCKED))).toEqual([])
    expect(frames.some((text) => text.includes('Loading…'))).toBe(true)
    expect(calls.filter((name) => name === 'vault_unlock_route')).toHaveLength(1)
    expect(calls.filter((name) => name === 'unlock_vault')).toEqual([])
    await unmount()
  })

  // Item 8 at the window
  test('the key in the Keychain: Home’s locked card, never Set up this Mac', async () => {
    jest.useFakeTimers()
    installBackend({ reads: [false], route: 'keychain' })
    const { container, unmount } = await mountMacApp()
    const frames: string[] = []
    await pass(2000, container, (text) => frames.push(text))
    expect(container.textContent).toContain(LOCKED)
    expect(frames.filter((text) => text.includes(SET_UP))).toEqual([])
    await unmount()
  })

  // Item 9 at the window
  test('while the route is out: Loading…, never Home and never the card; the route is asked ONCE, not on every 5 s read', async () => {
    jest.useFakeTimers()
    const calls = installBackend({ reads: [false], route: 'keychain' })
    const { container, unmount } = await mountMacApp()
    const early: string[] = []
    await pass(250, container, (text) => early.push(text)) // the answers land at 500 ms
    expect(early).toEqual(['Loading…'])
    await pass(12000, container, () => {})
    expect(calls.filter((name) => name === 'vault_unlock_route')).toHaveLength(1)
    await unmount()
  })

  // Item 10 at the window: a step Rust remembers (it carries the reason line) wins over the route.
  test('a pending set-up step wins over the route (it carries why the phrase is asked)', async () => {
    jest.useFakeTimers()
    installBackend({ reads: [false], route: 'keychain', pending: { key_replaced: true } })
    const { container, unmount } = await mountMacApp()
    await pass(2000, container, () => {})
    expect(container.textContent).toContain(SET_UP)
    expect(container.textContent).not.toContain(LOCKED)
    await unmount()
  })

  test('a route that cannot be read is the Keychain unlock: Home’s locked card, not a window stuck on Loading…', async () => {
    jest.useFakeTimers()
    installBackend({ reads: [false], route: () => { throw new Error('the Keychain service did not answer') } })
    const { container, unmount } = await mountMacApp()
    await pass(2000, container, () => {})
    expect(container.textContent).toContain(LOCKED)
    expect(container.textContent).not.toContain(SET_UP)
    await unmount()
  })

  test('a window that OPENS unlocked and is then locked asks nothing: Home’s locked card at once (the route is for a window that opens locked)', async () => {
    jest.useFakeTimers()
    const calls = installBackend({ reads: [true, false], route: 'recovery_phrase' })
    const { container, unmount } = await mountMacApp()
    await pass(1000, container, () => {})
    expect(container.textContent).toContain('Up to date')
    const after: string[] = []
    await pass(6000, container, (text) => after.push(text))
    expect(container.textContent).toContain(LOCKED)
    expect(after.filter((text) => text.includes('Loading…') || text.includes(SET_UP))).toEqual([])
    expect(calls.filter((name) => name === 'vault_unlock_route')).toEqual([])
    await unmount()
  })

  // The route belongs to the Mac as it was when the window opened. Once the keys have arrived (the phrase restored them) a
  // later Lock must not take the person back to Set up this Mac.
  test('after the keys arrived, a later Lock shows Home’s locked card, not Set up this Mac', async () => {
    jest.useFakeTimers()
    installBackend({ reads: [false, true, false], route: 'recovery_phrase' })
    const { container, unmount } = await mountMacApp()
    await pass(2000, container, () => {})
    expect(container.textContent).toContain(SET_UP)
    await pass(5000, container, () => {}) // the keys arrive: the next read is unlocked
    expect(container.textContent).not.toContain(SET_UP)
    const afterLock: string[] = []
    await pass(5500, container, (text) => afterLock.push(text)) // then it is locked again
    expect(container.textContent).toContain(LOCKED)
    expect(afterLock.filter((text) => text.includes(SET_UP))).toEqual([])
    await unmount()
  })

  // The same thing without the unlocked read in between: the phrase restored the keys, the window moved on to Finder and then
  // Home, and the snapshot it holds has not caught up with the unlock yet (it is read every 5 s).
  test('Finder then Home on a snapshot that still reads locked does not bounce back to Set up this Mac', async () => {
    jest.useFakeTimers()
    installBackend({ reads: [false], route: 'recovery_phrase', finder: 'adding' })
    const { container, unmount } = await mountMacApp()
    await pass(2000, container, () => {})
    expect(container.textContent).toContain(SET_UP)
    await React.act(async () => { setSignInFlow({ ...IDLE_FLOW, step: 'finder' }) })
    await pass(250, container, () => {})
    expect(getSignInFlow().step).toBe('finder')
    await React.act(async () => { setSignInFlow(IDLE_FLOW) }) // “Continue to Home”
    expect(container.textContent).not.toContain(SET_UP)
    expect(container.textContent).toContain(LOCKED)
    await unmount()
  })

  test('the same when Finder is ready and the window moves on by itself', async () => {
    jest.useFakeTimers()
    installBackend({ reads: [false], route: 'recovery_phrase', finder: 'ready' })
    const { container, unmount } = await mountMacApp()
    await pass(2000, container, () => {})
    expect(container.textContent).toContain(SET_UP)
    const frames: string[] = []
    await React.act(async () => { setSignInFlow({ ...IDLE_FLOW, step: 'finder' }) })
    await pass(750, container, (text) => frames.push(text))
    expect(getSignInFlow().step).toBeNull()
    expect(frames.at(-1)).toContain(LOCKED)
    expect(frames.at(-1)).not.toContain(SET_UP)
    await unmount()
  })

  // Review P2 (window): the route asked at a click in Settings › Account must actually show the card. Settings is a mode of
  // the one window and the card is drawn in `main`, so the callback has to leave Settings; otherwise the click did nothing
  // visible and the person stayed on the locked Account tab.
  test('Settings › Account Unlock with no key anywhere leaves Settings and shows Set up this Mac', async () => {
    jest.useFakeTimers()
    const script: { reads: boolean[]; route: 'keychain' | 'recovery_phrase' } = { reads: [false], route: 'keychain' }
    const calls = installBackend(script)
    const { container, unmount } = await mountMacApp()
    await pass(2000, container, () => {})
    // The window opened locked with the key in the Keychain: Home's locked card, not the card.
    expect(container.textContent).toContain(LOCKED)
    const gear = allElements(container).find((element) => element.getAttribute('id') === 'mw-gear')
    expect(gear).toBeDefined()
    await React.act(async () => { click(container, gear as MiniElement) })
    await pass(500, container, () => {})
    const accountRow = allElements(container).find((element) => element.getAttribute('id') === 'mw-settings-account')
    expect(accountRow).toBeDefined()
    await React.act(async () => { click(container, accountRow as MiniElement) })
    await pass(1500, container, () => {})
    expect(container.textContent).toContain('Vault is locked')
    // The Keychain was read at the open; at this click it answers that there is no key anywhere.
    script.route = 'recovery_phrase'
    const unlock = allElements(container).find((element) => element.tagName === 'BUTTON' && element.textContent === 'Unlock')
    expect(unlock).toBeDefined()
    await React.act(async () => { click(container, unlock as MiniElement) })
    await pass(1500, container, () => {})
    expect(calls.filter((name) => name === 'vault_unlock_route')).toHaveLength(2)
    expect(calls.filter((name) => name === 'unlock_vault')).toEqual([])
    // The card, not the locked Account tab: Settings is left.
    expect(container.textContent).toContain(SET_UP)
    expect(container.textContent).not.toContain('Vault is locked')
    await unmount()
  })
})
