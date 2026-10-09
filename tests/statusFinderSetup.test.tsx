/**
 * Spec 2026-10-06 §10: on macOS the Status page takes the Finder row from the reconciler
 * (`finder_setup_state` + `finder-setup-changed`, through the shared `useFinderSetup`, lead ruling
 * 7b) instead of polling `finder_location_state` every 3 s. Windows and Linux are unchanged.
 *
 * The page runs the REAL hook declaration (tests/fixtures/finderSetupHarness.ts) against a scripted
 * backend and event bus. The platform comes from `desktop_platform`, and when that fails or says
 * 'unknown', from the capability snapshot's host OS, exactly as Onboarding does (Task 15): a Mac
 * whose `desktop_platform` read fails is still a Mac and must never reach the install-era command.
 */
import { afterEach, describe, expect, test } from 'bun:test'
import * as desktopApi from '../src/desktopApi'
import * as copy from '../src/finderSetupCopy'
import { loadComponent, mount, textOf, type Mounted } from './fixtures/componentHarness'
import { finderBus, finderView, tick, useFinderSetupModule } from './fixtures/finderSetupHarness'
import { rustStr } from './fixtures/rustConstants'

const finderSetupState = loadComponent('pages/Status.tsx', 'finderSetupState', {})
const macFinderSetupState = loadComponent('pages/Status.tsx', 'macFinderSetupState', { ...copy })

const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close() })

const missing = { installed: false, path: null, status: 'missing', last_error: null, last_attempt_at: null, reason_category: null }

interface Opening {
  /** What `desktop_platform` does: answer with a name, or fail. */
  platform: string | 'fails'
  /** What `finder_setup_state` does: answer with a view (or anything), or reject with an Error. */
  finder?: unknown
  /** The capability snapshot's host OS, or none. */
  caps?: string | null
  /** What `sync_status` answers (default: signed out, engine stopped). */
  syncStatus?: Record<string, unknown>
}

function openStatus({ platform, finder = finderView(), caps = null, syncStatus }: Opening) {
  const bus = finderBus()
  const intervals: Array<() => Promise<void>> = []
  const m = mount('pages/Status.tsx', 'Status', {
    expand: true,
    backend: {
      desktop_platform: () => { if (platform === 'fails') throw new Error('desktop_platform is down'); return platform },
      sync_status: () => syncStatus ?? ({ logged_in: false, engine: 'stopped', sync_root: null, syncing: 0, cloud_only: 0, conflicts: 0 }),
      finder_setup_state: () => { if (finder instanceof Error) throw finder; return finder },
      finder_location_state: () => missing,
    },
    hookModules: [useFinderSetupModule(bus)],
    bindings: {
      ...desktopApi,
      finderSetupState,
      macFinderSetupState,
      useCapabilities: () => (caps ? { host_os: caps } : null),
    },
  })
  mounted.push(m)
  ;(globalThis as any).window.setInterval = (fn: () => Promise<void>) => { intervals.push(fn); return intervals.length }
  const calls = (name: string) => m.calls.filter((c) => c.name === name).length
  return { m, bus, intervals, calls }
}

/** Effects that wait on state set by an earlier effect need a render between them, as React does. */
async function settle(m: Mounted) {
  for (let i = 0; i < 6; i++) { await m.flush(); await tick() }
}

describe('Status page, Finder row, on macOS', () => {
  test('it reads the reconciler, never finder_location_state, and follows the event instead of polling', async () => {
    const { m, bus, calls } = openStatus({ platform: 'macos' })
    await settle(m)
    expect(calls('finder_location_state')).toBe(0)
    expect(calls('finder_setup_state')).toBe(1)
    expect(bus.registered).toBe(1)
    expect(textOf(m.tree())).toContain('Adding')
    bus.emit(finderView({ setup: 'ready' }))
    await m.flush()
    expect(textOf(m.tree())).toContain('Installed')
    expect(calls('finder_setup_state')).toBe(1)
  })

  test('the 3 s poll goes on for the sync status and no longer touches the Finder state', async () => {
    const { m, intervals, calls } = openStatus({ platform: 'macos' })
    await settle(m)
    expect(intervals).toHaveLength(1)
    const statusReads = calls('sync_status')
    await intervals[0]()
    await intervals[0]()
    expect(calls('sync_status')).toBe(statusReads + 2)
    expect(calls('finder_location_state')).toBe(0)
    expect(calls('finder_setup_state')).toBe(1)
  })

  test('a failure shows its §6.2 sentence', async () => {
    const { m } = openStatus({ platform: 'macos', finder: finderView({ setup: 'failed', reason: 'folder_taken' }) })
    await settle(m)
    expect(textOf(m.tree())).toContain(copy.FINDER_REASON_COPY.folder_taken.sentence)
    expect(textOf(m.tree())).toContain('Setup blocked')
  })

  // FA-I2: a loaded Missing is a quiet row. The Status line is a slot that must show text, so it
  // shows the one sentence; there is no Finder pill, and nothing says "Checking".
  test('a loaded Missing: no Finder pill, the one sentence, no "Checking" and no activity', async () => {
    const { m } = openStatus({ platform: 'macos', finder: finderView({ setup: 'missing' }) })
    await settle(m)
    const text = textOf(m.tree())
    expect(text).toContain(copy.FINDER_RESTING_LINE)
    expect(text).not.toMatch(/Checking|Adding/)
    // The only pill left is the page's own sync-health pill.
    expect(m.elements().filter((el) => el.props.className === 'status-pill')).toHaveLength(1)
  })

  test('a Missing that carries a reason (D7) is that notice, with the notice\'s pill, never "Checking"', async () => {
    const { m } = openStatus({ platform: 'macos', finder: finderView({ setup: 'missing', reason: 'not_in_applications', launch_location: 'disk_image' }) })
    await settle(m)
    const text = textOf(m.tree())
    expect(text).toContain(copy.FINDER_REASON_COPY.not_in_applications.sentence)
    expect(text).toContain('Setup blocked')
    expect(text).not.toContain('Checking')
  })

  test('before the first answer the Finder line claims nothing, and no ad-hoc "Checking" string is shown', async () => {
    const { m } = openStatus({ platform: 'macos', finder: new Promise(() => {}) })
    await settle(m)
    expect(textOf(m.tree())).not.toContain('Checking')
  })

  test('a state that cannot be read says so, and is never "Adding" (lead ruling 7a)', async () => {
    const { m } = openStatus({ platform: 'macos', finder: new Error('no reconciler') })
    await settle(m)
    const text = textOf(m.tree())
    expect(text).toContain(copy.FINDER_UNAVAILABLE_LINE)
    expect(text).toContain(copy.FINDER_STATUS_PILL_UNAVAILABLE)
    expect(text).not.toContain('Adding')
  })

  test('the sentence is human text, not set in the mono face a path gets', async () => {
    const { m } = openStatus({ platform: 'macos', finder: finderView({ setup: 'ready' }) })
    await settle(m)
    expect(textOf(m.tree())).toContain(copy.FINDER_READY_LINE)
    const mono = m.elements().filter((el) => el.props.className === 'mono').map((el) => textOf(el.props.children))
    expect(mono.filter((text) => text.includes(copy.FINDER_READY_LINE))).toEqual([])
  })

  test('a Mac whose desktop_platform read fails is still a Mac: the snapshot decides, and nothing install-era is called', async () => {
    for (const platform of ['fails', 'unknown']) {
      const { m, calls } = openStatus({ platform, caps: 'macos' })
      await settle(m)
      expect(calls('finder_location_state')).toBe(0)
      expect(calls('finder_setup_state')).toBe(1)
      expect(textOf(m.tree())).toContain('Adding')
    }
  })

  test('the snapshot is also the first answer: no read of the Windows/Linux state while desktop_platform is still out', async () => {
    const { m, calls } = openStatus({ platform: 'macos', caps: 'macos' })
    await m.flush()
    expect(calls('finder_location_state')).toBe(0)
  })
})

/**
 * Must-render rows 8 and 9 (FT-I5): `sync_status.engine_refusal` is why sync did not start. Status
 * shows its sentence on the sync-health line (macOS and Linux), and on a Mac the Finder row of a
 * `failed` + `unknown` says it instead of the generic "couldn’t be added".
 */
describe('Status page: the engine refusal (rows 8 and 9)', () => {
  const IDENTITY = { code: 'identity_unknown', sentence: rustStr('account_binding.rs', 'IDENTITY_UNKNOWN') }
  const signedIn = (engine_refusal: unknown) => ({ logged_in: true, vault_unlocked: true, engine: 'stopped', sync_root: null, syncing: 0, cloud_only: 0, conflicts: 0, engine_refusal })
  const healthLine = (m: Mounted) =>
    m.elements().filter((el) => el.type === 'p' && el.props.className === 'page-copy').map((el) => textOf(el.props.children)).at(-1)

  for (const platform of ['macos', 'linux']) {
    test(`${platform}: the refusal's sentence is the sync-health line`, async () => {
      const { m } = openStatus({ platform, caps: platform, syncStatus: signedIn(IDENTITY), finder: finderView({ setup: 'ready' }) })
      await settle(m)
      expect(healthLine(m)).toBe(IDENTITY.sentence)
    })
  }

  test('no refusal: the sync-health line is what it was', async () => {
    const { m } = openStatus({ platform: 'macos', syncStatus: signedIn(null), finder: finderView({ setup: 'ready' }) })
    await settle(m)
    expect(healthLine(m)).not.toBe(IDENTITY.sentence)
    expect(textOf(m.tree())).not.toContain(IDENTITY.sentence)
  })

  test('macOS: a failed + unknown Finder row says the refusal, not the generic sentence', async () => {
    const { m } = openStatus({ platform: 'macos', syncStatus: signedIn(IDENTITY), finder: finderView({ setup: 'failed', reason: 'unknown' }) })
    await settle(m)
    const finderRow = m.elements().filter((el) => el.props.className === 'row-detail').map((el) => textOf(el.props.children))[0]
    expect(finderRow).toContain(IDENTITY.sentence)
    expect(textOf(m.tree())).not.toContain(copy.FINDER_REASON_COPY.unknown.sentence)
  })
})

describe('Status page, Finder row, off macOS (unchanged)', () => {
  test('Linux still reads finder_location_state, polls it, and never touches the reconciler', async () => {
    const { m, bus, intervals, calls } = openStatus({ platform: 'linux', caps: 'linux' })
    await settle(m)
    expect(calls('finder_location_state')).toBe(1)
    expect(textOf(m.tree())).toContain('Needs install')
    await intervals[0]()
    expect(calls('finder_location_state')).toBe(2)
    expect(calls('finder_setup_state')).toBe(0)
    expect(bus.registered).toBe(0)
  })

  test('Windows is the same', async () => {
    const { m, bus, calls } = openStatus({ platform: 'windows', caps: 'windows' })
    await settle(m)
    expect(calls('finder_location_state')).toBe(1)
    expect(calls('finder_setup_state')).toBe(0)
    expect(bus.registered).toBe(0)
  })

  test('when neither desktop_platform nor the snapshot can say, the page is the Windows/Linux one, as Onboarding is', async () => {
    const { m, calls } = openStatus({ platform: 'fails', caps: null })
    await settle(m)
    expect(calls('finder_location_state')).toBe(1)
    expect(calls('finder_setup_state')).toBe(0)
  })
})
