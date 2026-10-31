/**
 * Delta review P3-3 on PR #143 (follow-up): a window that opened UNLOCKED and is then locked (Lock now, from the menu or the
 * popover) must go straight to Home's locked card. The question "is Set up this Mac still owed?" (`pending_setup`) is for a
 * window that OPENS locked: a Lock can only follow keys that arrived, and keys arriving forget the step in Rust, so asking
 * after a Lock only paints "Loading…" for one round trip in between.
 *
 * Mounted with the real `react-dom` on the minimal DOM and on bun's fake clock, like `macAppSlowSnapshot.test.tsx`: the
 * window reads the account every 5 s, `pending_setup` takes half a second to answer here so a flash is something a frame
 * can show, and every frame is checked. What this does NOT prove: anything on a Mac.
 */
import { afterAll, afterEach, beforeAll, describe, expect, jest, test } from 'bun:test'
import { resetSignInFlow } from '../src/signInFlowStore'
import { installMiniDom, type MiniDomInstall, type MiniElement } from './fixtures/miniDom'

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

const account = (unlocked: boolean) => ({ email: 'sam@example.eu', logged_in: true, vault_unlocked: unlocked, auth_expired: false })
const snapshot = (unlocked: boolean) => ({
  phase: unlocked ? 'synced' : 'locked', sync_phase: unlocked ? 'synced' : 'locked', generated_at: 1000, paused: false,
  account: account(unlocked),
  engine: { state: 'idle', files_remaining: 0, bytes_total: 0, bytes_done: 0, last_tick_ok_at: null },
  reason: null, finder: { setup: 'ready', reason: null, reason_line: null },
  storage: unlocked ? { used_bytes: 84_300_000_000, quota_bytes: 200_000_000_000, fetched_at: 900, stale: false } : null,
  storage_full: false, pending_changes: 0, conflicts: { count: 0, files: [] }, activity: [],
})

/** `reads[n]` is whether the n-th `popover_snapshot` answers unlocked; the last entry repeats. */
function installBackend(reads: boolean[]) {
  const calls: string[] = []
  let read = 0
  let callbackId = 0
  const handlers: Record<string, (args: any) => unknown> = {
    popover_snapshot: () => snapshot(reads[Math.min(read++, reads.length - 1)]),
    pending_setup: () => new Promise((resolve) => setTimeout(() => resolve(null), 500)),
    // 1747 Part 2: this Mac's key is in the Keychain; the route is its own file's subject (`macAppVaultRoute.test.tsx`).
    vault_unlock_route: () => 'keychain',
    set_pending_setup: () => null,
    account_subscription: () => ({ plan: 'basic', billing_cycle: 'yearly', status: 'active', seats: 1, region: 'eu', current_period_end: null, quota_bytes: 1, used_bytes: 0 }),
    consume_main_navigation: () => null,
    consume_menu_update_check: () => false,
    consume_sign_out_flow_request: () => false,
    main_window_state: () => null,
    get_desktop_config: () => ({ upload_kbps_limit: 0, download_kbps_limit: 0, pause_sync: false, notify_conflicts: true, notify_sync_complete: false, notify_quota_warnings: true, theme: 'dark', locale: 'en' }),
    finder_setup_state: () => ({ setup: 'ready', reason: null, launch_location: 'applications', attempt: 1, max_attempts: 1, last_failure: null }),
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

describe('Lock after the window opened unlocked (P3-3)', () => {
  test('the control: a window that OPENS locked asks Rust once, and says Loading… until it answers', async () => {
    jest.useFakeTimers()
    const calls = installBackend([false])
    const { container, unmount } = await mountMacApp()
    const frames: string[] = []
    await pass(2000, container, (text) => frames.push(text))
    expect(calls.filter((name) => name === 'pending_setup')).toHaveLength(1)
    expect(frames.some((text) => text.includes('Loading…'))).toBe(true)
    expect(container.textContent).toContain('Your vault is locked')
    await unmount()
  })

  test('opened unlocked, locked at the next read: Home’s locked card at once, no question asked, no Loading… frame', async () => {
    jest.useFakeTimers()
    const calls = installBackend([true, false])
    const { container, unmount } = await mountMacApp()
    await pass(1000, container, () => {})
    expect(container.textContent).toContain('Up to date')
    const after: string[] = []
    await pass(6000, container, (text) => after.push(text))
    expect(container.textContent).toContain('Your vault is locked')
    expect(calls.filter((name) => name === 'pending_setup')).toHaveLength(0)
    expect(after.filter((text) => text.includes('Loading…'))).toEqual([])
    await unmount()
  })
})
