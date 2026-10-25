/**
 * Follow-up to PR #134, auto-review thread 1: the Beebeeb window reads the account and sync state every 5 s, and a read
 * can be slower than that. Before task 2179, `popover_snapshot` awaited the storage fetch (8 s timeout, no cache of a
 * failure); since then it waits for it 1 s at most, but a read can still be slow. A tick must never cancel the read in
 * flight: while the API answered slower than the tick, every result used to be thrown away and the window stayed on
 * "Loading…".
 *
 * Mounted with the real `react-dom` on the minimal DOM (React's own effect cleanup is what cancelled the read; the
 * component harness does not run cleanups on a dependency change) and on bun's fake clock, so 20 s pass in an instant.
 * What this does NOT prove: anything on a Mac.
 */
import { afterAll, afterEach, beforeAll, describe, expect, jest, test } from 'bun:test'
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
})
afterAll(() => dom.restore())

const snapshot = {
  phase: 'synced', sync_phase: 'synced', generated_at: 1000, paused: false,
  account: { email: 'sam@example.eu', logged_in: true, vault_unlocked: true, auth_expired: false },
  engine: { state: 'idle', files_remaining: 0, bytes_total: 0, bytes_done: 0, last_tick_ok_at: null },
  reason: null, finder: { setup: 'ready', reason: null, reason_line: null },
  storage: { used_bytes: 84_300_000_000, quota_bytes: 200_000_000_000, fetched_at: 900, stale: false },
  storage_full: false, pending_changes: 0, conflicts: { count: 0, files: [] }, activity: [],
}

/** Every command the window sends; `popover_snapshot` answers after `readMs` on the (fake) clock. */
function installBackend(readMs: number) {
  const calls: string[] = []
  let callbackId = 0
  const handlers: Record<string, (args: any) => unknown> = {
    popover_snapshot: () => new Promise((resolve) => setTimeout(() => resolve(snapshot), readMs)),
    account_subscription: () => ({ plan: 'basic', billing_cycle: 'yearly', status: 'active', seats: 1, region: 'eu', current_period_end: null, quota_bytes: 1, used_bytes: 0 }),
    consume_main_navigation: () => null,
    consume_menu_update_check: () => false,
    consume_sign_out_flow_request: () => false,
    main_window_state: () => null,
    get_desktop_config: () => ({ upload_kbps_limit: 0, download_kbps_limit: 0, pause_sync: false, notify_conflicts: true, notify_sync_complete: false, notify_quota_warnings: true, theme: 'dark', local_cache_limit_bytes: 0 }),
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

/** Move the fake clock on in 1 s steps, letting React and the backend's promises run in between. */
async function pass(ms: number) {
  for (let t = 0; t < ms; t += 1000) {
    await React.act(async () => {
      jest.advanceTimersByTime(1000)
      for (let i = 0; i < 10; i++) await Promise.resolve()
    })
  }
}

describe('a snapshot read slower than the 5 s tick', () => {
  test('the control: a 1 s read lands (so the instrument, fake clock and all, can show a landed read)', async () => {
    jest.useFakeTimers()
    installBackend(1000)
    const { container, unmount } = await mountMacApp()
    try {
      await pass(2000)
      expect(container.textContent).toContain('Up to date')
    } finally {
      await unmount()
    }
  })

  test('an 8 s read with 5 s ticks still lands: the window shows Home, and the ticks did not pile reads up', async () => {
    jest.useFakeTimers()
    const calls = installBackend(8000)
    const { container, unmount } = await mountMacApp()
    try {
      expect(container.textContent).toContain('Loading…')
      await pass(9000)
      expect(container.textContent).not.toContain('Loading…')
      expect(container.textContent).toContain('Up to date')
      // The first read was never cancelled or doubled: the ticks at 5 s (while it was in flight) started nothing.
      expect(calls.filter((name) => name === 'popover_snapshot')).toHaveLength(1)
      await pass(11000)
      // After it landed, the ticks read again, one at a time.
      const reads = calls.filter((name) => name === 'popover_snapshot').length
      expect(reads).toBeGreaterThanOrEqual(2)
      expect(reads).toBeLessThanOrEqual(3)
      expect(container.textContent).toContain('Up to date')
    } finally {
      await unmount()
    }
  })
})
