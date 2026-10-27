/**
 * The Windows main window's Home strip (0.8.15): State and "Not sent" follow the sync activity model, never `sync_status`
 * alone. On Guus's PC Home said "Up to date · Syncing 0" while 2,924 changes were on hold and 43 had given up.
 *
 * Two levels: `homeSyncMetrics` as a pure function, and the REAL `HomeView` declaration executed with the component
 * harness and the REAL `useSyncActivitySummary`, against a scripted backend that answers `sync_activity_summary` with the
 * Rust fixture (`tests/fixtures/sync-activity-summary.on-hold.json`, the JSON `sync_activity.rs` serialises).
 * What this does not prove: layout or WebView2 (`tests/render-sync-activity.mjs` renders Home in a browser).
 */
import { afterEach, describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import * as desktopApi from '../src/desktopApi'
import type { SyncStatus } from '../src/desktopApi'
import { homeSyncMetrics, notSentFigure } from '../src/windows/homeSyncMetrics'
import { parseSyncActivitySummary, type SyncActivitySummary } from '../src/windows/syncActivityModel'
import { ENGINE_STATUS_EVENT } from '../src/popoverContract'
import { pollWhileVisible, windowVisible } from '../src/windows/visiblePoll'
import { mount, textOf, type Mounted } from './fixtures/componentHarness'

const raw: unknown = JSON.parse(readFileSync(new URL('./fixtures/sync-activity-summary.on-hold.json', import.meta.url), 'utf8'))
const fixture = parseSyncActivitySummary(raw)
if (!fixture) throw new Error('fixture did not parse')

const signedIn: SyncStatus = { logged_in: true, engine: 'running', sync_root: 'C:\\Users\\Fixture\\Beebeeb', syncing: 0, cloud_only: 0, conflicts: 0 }
/** The fixture once its startup check has finished: 2,924 held for `auth`, 43 given up, nothing moving. */
const onHold: SyncActivitySummary = { ...fixture, engine_state: 'idle', startup_check: null }
const calm: SyncActivitySummary = {
  paused: false,
  engine_state: 'idle',
  files_remaining: null,
  transfers_active: 0,
  last_tick_ok_at: 1_793_000_000,
  queue: { waiting: 0, gave_up: 0, held: [] },
  startup_check: null,
}
/** 2,924 held + 43 given up + 0 waiting. */
const NOT_SENT = '2,967'

const figures = (status: SyncStatus | null, summary: SyncActivitySummary | null | undefined, pausedFallback?: boolean) =>
  Object.fromEntries(homeSyncMetrics(status, summary, { device: 'this PC', pausedFallback }).map(({ k, v }) => [k, v]))

describe('homeSyncMetrics: the figures follow the model', () => {
  test('the on-hold fixture says "Not in sync" with 2,967 not sent, never "Up to date" or "Syncing"', () => {
    const f = figures(signedIn, onHold)
    expect(f).toEqual({ State: 'Not in sync', 'Not sent': NOT_SENT, Conflicts: '0' })
    // `sync_status.syncing` is not what Home counts: a stale count there changes nothing.
    expect(figures({ ...signedIn, syncing: 5 }, onHold)).toEqual(f)
  })

  test('the fixture as Rust wrote it (startup check still running) says "Checking", with the same 2,967 not sent', () => {
    const f = figures(signedIn, fixture)
    expect(f.State).toBe('Checking')
    expect(f['Not sent']).toBe(NOT_SENT)
  })

  test('the held and given-up counts never read as calm, whatever the engine is doing', () => {
    for (const summary of [
      onHold,
      fixture,
      { ...onHold, engine_state: 'running' },
      { ...onHold, engine_state: 'offline' },
      { ...onHold, transfers_active: 2 },
      { ...onHold, queue: { ...onHold.queue!, held: [] } },
      { ...onHold, queue: { waiting: 0, gave_up: 0, held: onHold.queue!.held } },
    ]) {
      const f = figures(signedIn, summary)
      expect(f.State).not.toBe('Up to date')
      expect(f['Not sent']).not.toBe('0')
    }
  })

  test('a calm summary (empty queue, finished check) still says "Up to date", with 0 not sent', () => {
    expect(figures(signedIn, calm)).toEqual({ State: 'Up to date', 'Not sent': '0', Conflicts: '0' })
  })

  test('without the summary there is no "Up to date" and no count: "…" before the first read, "—" when unreadable', () => {
    expect(figures(signedIn, undefined)).toEqual({ State: '…', 'Not sent': '…', Conflicts: '0' })
    const unreadable = figures(signedIn, null)
    expect(unreadable.State).not.toBe('Up to date')
    expect(unreadable['Not sent']).toBe('—')
    // The saved pause setting is the model's fallback only then.
    expect(figures(signedIn, null, true).State).toBe('Paused')
    expect(figures(signedIn, calm, true).State).toBe('Up to date')
  })

  test('"Not sent" counts waiting, every held group and given up', () => {
    expect(notSentFigure({ ...calm, queue: { waiting: 3, gave_up: 2, held: [{ reason: 'quota', count: 4, oldest_at: 0, newest_at: 0 }, { reason: 'auth', count: 1_000, oldest_at: 0, newest_at: 0 }] } })).toBe('1,009')
  })

  test('a summary without a queue (`queue: null`, no state DB yet) is no count: "—", never 0', () => {
    const noQueue: SyncActivitySummary = { ...calm, queue: null }
    expect(notSentFigure(noQueue)).toBe('—')
    expect(figures(signedIn, noQueue)['Not sent']).toBe('—')
  })
})

// ── The real HomeView ────────────────────────────────────────────────────────

const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close() })

const summaryHook = {
  file: 'windows/syncActivityUi.tsx',
  name: 'useSyncActivitySummary',
  bindings: {
    loadSyncActivitySummary: desktopApi.loadSyncActivitySummary,
    listen: async () => () => undefined,
    ENGINE_STATUS_EVENT,
    SYNC_ACTIVITY_POLL_MS: 3000,
    ENGINE_EVENT_COALESCE_MS: 250,
    currentPollWindow: () => null,
    pollWhileVisible,
    windowVisible,
  },
}

function openHome(summary: unknown, { status = signedIn, pauseSync = false }: { status?: SyncStatus; pauseSync?: boolean } = {}) {
  const m = mount('WindowsApp.tsx', 'HomeView', {
    backend: {
      sync_activity_summary: () => summary,
      desktop_config: () => ({ upload_kbps_limit: 0, download_kbps_limit: 0, pause_sync: pauseSync, notify_conflicts: true, notify_sync_complete: false, notify_quota_warnings: true }),
    },
    props: { status, usage: { used_bytes: 201_000_000_000, quota_bytes: 2_000_000_000_000 }, storage: null },
    hookModules: [summaryHook],
    bindings: {
      command: desktopApi.command,
      homeSyncMetrics,
      usePlatformName: () => 'windows',
      thisDeviceNoun: () => 'this PC',
      formatBytes: (n: number) => `${n} B`,
      accountProfile: async () => ({ ok: false, reason: 'signed out', unsupported: false }),
      accountActivity: async () => ({ ok: false, reason: 'signed out', unsupported: false }),
      homeRelativeTime: () => '',
      homeEventVisual: () => ({ icon: 'clock', dot: '' }),
      homeHumanizeType: (t: string) => t,
      T: new Proxy({}, { get: () => '' }),
      Card: 'card', Chip: 'chip', PageHeader: 'header', Skeleton: 'skeleton', NavIcon: 'icon',
    },
  })
  mounted.push(m)
  return m
}

/** The metric strip as a person reads it: each figure's label (the element's key) and its value. */
function strip(m: Mounted): Record<string, string> {
  const out: Record<string, string> = {}
  for (const el of m.elements()) {
    const key = (el as unknown as { key: string | null }).key
    if (key && ['State', 'Not sent', 'Syncing', 'Conflicts', 'In vault'].includes(key)) out[key] = textOf(el.props.children[1])
  }
  return out
}

const settle = async (m: Mounted) => { await m.flush(); await m.flush(); await m.flush() }

describe('HomeView: the strip renders the model', () => {
  test('the on-hold fixture: State "Not in sync", 2,967 not sent, no "Syncing" figure, no "Up to date" anywhere', async () => {
    const m = openHome(onHold)
    await settle(m)
    expect(strip(m)).toEqual({ State: 'Not in sync', 'Not sent': NOT_SENT, Conflicts: '0', 'In vault': '201000000000 B' })
    expect(textOf(m.tree())).not.toContain('Up to date')
    expect(m.calls.map((c) => c.name)).toContain('sync_activity_summary')
  })

  test('a calm summary still says "Up to date"', async () => {
    const m = openHome(calm)
    await settle(m)
    expect(strip(m)).toEqual({ State: 'Up to date', 'Not sent': '0', Conflicts: '0', 'In vault': '201000000000 B' })
  })

  test('an unreadable summary: the saved pause setting is the fallback, and the count is unknown, not 0', async () => {
    const m = openHome({ not: 'the contract' }, { pauseSync: true })
    await settle(m)
    expect(strip(m)).toEqual({ State: 'Paused', 'Not sent': '—', Conflicts: '0', 'In vault': '201000000000 B' })
  })
})
