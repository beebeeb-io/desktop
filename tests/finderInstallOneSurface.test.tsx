/**
 * One Finder-install failure renders exactly ONE error surface (task 1683 slice 5, problem
 * P4 / screenshot 1, decision D1).
 *
 * The defect: `install_finder_location` saved its failure (`finder_install_last_error`, read
 * back by `finder_location_state` and painted as a red banner) AND returned it as an `Err`,
 * which `SyncFolder` turned into a toast. Two surfaces, one string, and the banner outlived
 * the 6 s toast. D1 (Guus, 2026-09-30): keep the house rule "a transient action failure is a
 * toast, a failure that GATES a control is inline" and fix only the double render. A Finder
 * install failure gates "Open in Finder", so it is inline, once, never a toast.
 *
 * These tests execute the production components (SyncFolder, and the Settings panel
 * `ExplorerIntegrationPanel` that shares the same action) against a scripted backend that
 * models both the saved and the returned half of the old behaviour. `visibleErrorSurfaces`
 * counts inline error notices plus error toasts, so "exactly 1" means 1 in total.
 *
 * Mutation checks (red first, then reverted) are recorded in the task Notes, 2026-09-30.
 */
import { afterEach, describe, expect, test } from 'bun:test'
import * as desktopApi from '../src/desktopApi'
import * as finderInstallCard from '../src/finderInstallCard'
import { T } from '../src/windows/ui'
import { mount, visibleErrorSurfaces, type Mounted } from './fixtures/componentHarness'

const TIMEOUT = 'Timed out waiting for the Beebeeb File Provider domain to become available'
const OTHER = 'Finder location must be absolute: relative/path'

const missing = { installed: false, path: null, status: 'missing', last_error: null, last_attempt_at: null, reason_category: null }
const failed = (message: string) => ({ installed: false, path: null, status: 'error', last_error: message, last_attempt_at: 2, reason_category: 'timeout' })
const installedState = { installed: true, path: 'Beebeeb in Finder', status: 'installed', last_error: null, last_attempt_at: 3, reason_category: null }

/**
 * How the backend answers a failing install.
 *  - 'err'      old shape: failure saved AND returned as a rejected command (the screenshot-1 bug)
 *  - 'state'    D1 shape: failure saved and returned as an Ok state (installed: false, last_error set)
 *  - 'unsaved'  a failure before anything is persisted (validation), returned as a rejected command
 *  - 'success'  the install works
 */
type Shape = 'err' | 'state' | 'unsaved' | 'success'

function finderBackend(shape: Shape, persistedBefore: string | null, gate?: { release: Promise<void> }) {
  let persisted: any = persistedBefore ? failed(persistedBefore) : missing
  return {
    desktop_platform: () => 'macos',
    sync_status: () => ({ logged_in: true, engine: 'running', sync_root: '/Users/fixture/Library/CloudStorage/Beebeeb', syncing: 0, cloud_only: 0, conflicts: 0 }),
    finder_location_state: () => persisted,
    install_windows_shell_integration: () => { throw new Error('windows-only command called on macOS') },
    install_finder_location: async () => {
      if (gate) await gate.release
      if (shape === 'success') { persisted = installedState; return installedState }
      if (shape === 'unsaved') throw new Error(OTHER)
      persisted = failed(TIMEOUT)
      if (shape === 'err') throw new Error(TIMEOUT)
      return persisted
    },
  }
}

const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close() })

async function openFinderPane(shape: Shape, persistedBefore: string | null, gate?: { release: Promise<void> }) {
  const m = mount('pages/SyncFolder.tsx', 'SyncFolder', {
    backend: finderBackend(shape, persistedBefore, gate),
    bindings: { ...desktopApi, ...finderInstallCard },
  })
  mounted.push(m)
  await m.flush()
  return m
}

describe('SyncFolder (Finder location pane)', () => {
  test('a failed install saved AND returned as an error renders one inline banner and no toast', async () => {
    const m = await openFinderPane('err', null)
    expect(visibleErrorSurfaces(m)).toEqual([])
    await m.click('Install in Finder')
    expect(visibleErrorSurfaces(m)).toEqual([`inline: ${TIMEOUT}`])
    expect(m.toasts).toEqual([])
  })

  test('screenshot 1: opening the pane with the earlier failure already saved, then failing again, is still ONE surface', async () => {
    const m = await openFinderPane('err', TIMEOUT)
    expect(visibleErrorSurfaces(m)).toEqual([`inline: ${TIMEOUT}`]) // the banner from the saved failure
    await m.click('Install in Finder')
    expect(visibleErrorSurfaces(m)).toEqual([`inline: ${TIMEOUT}`])
    expect(m.toasts).toEqual([])
  })

  test('D1 shape (failure saved and returned as a state, not an error) renders one inline banner and no toast', async () => {
    const m = await openFinderPane('state', null)
    await m.click('Install in Finder')
    expect(visibleErrorSurfaces(m)).toEqual([`inline: ${TIMEOUT}`])
    expect(m.toasts).toEqual([])
  })

  test('a failure that was never saved (validation) is also exactly one inline surface, not a toast', async () => {
    const m = await openFinderPane('unsaved', null)
    await m.click('Install in Finder')
    expect(visibleErrorSurfaces(m)).toEqual([`inline: ${OTHER}`])
    expect(m.toasts).toEqual([])
  })

  test('starting a new attempt clears the previous failure while it runs, then shows the new result once', async () => {
    let release!: () => void
    const gate = { release: new Promise<void>((resolve) => { release = resolve }) }
    const m = await openFinderPane('err', TIMEOUT, gate)
    expect(visibleErrorSurfaces(m)).toEqual([`inline: ${TIMEOUT}`])
    await m.clickNoWait('Install in Finder')
    expect(visibleErrorSurfaces(m)).toEqual([]) // in flight: the stale failure is gone
    release()
    await m.flush()
    expect(visibleErrorSurfaces(m)).toEqual([`inline: ${TIMEOUT}`])
    expect(m.toasts).toEqual([])
  })

  test('success shows no error surface and swaps Install for Open in Finder', async () => {
    const m = await openFinderPane('success', TIMEOUT)
    await m.click('Install in Finder')
    expect(visibleErrorSurfaces(m)).toEqual([])
    expect(m.toasts).toEqual([])
    expect(m.elements().some((el) => el.type === 'button' && /Open in Finder/.test(String(el.props.children)))).toBe(true)
  })

  test('a transient failure that gates nothing still toasts: the folder picker on the non-macOS pane', async () => {
    const w = mount('pages/SyncFolder.tsx', 'SyncFolder', {
      backend: { ...finderBackend('success', null), desktop_platform: () => 'windows', pick_sync_root: () => { throw new Error('picker crashed') } },
      bindings: { ...desktopApi, ...finderInstallCard },
    })
    mounted.push(w)
    await w.flush()
    await w.click('Choose location')
    expect(w.toasts.map((t) => t.title)).toEqual(['Couldn’t open the folder picker'])
    expect(visibleErrorSurfaces(w)).toEqual([`toast: Couldn’t open the folder picker — picker crashed`])
  })
})

describe('Settings panel (ExplorerIntegrationPanel), same action, same rule', () => {
  async function openPanel(platform: 'macos' | 'windows', installBehaviour: 'err' | 'state' | 'success') {
    const windows = platform === 'windows'
    const installName = windows ? 'install_windows_shell_integration' : 'install_finder_location'
    const stateName = windows ? 'windows_shell_integration_state' : 'finder_location_state'
    let persisted: any = missing
    const m = mount('windows/views/SettingsView.tsx', 'ExplorerIntegrationPanel', {
      backend: {
        [stateName]: () => persisted,
        [installName]: () => {
          if (installBehaviour === 'success') { persisted = installedState; return installedState }
          persisted = failed(TIMEOUT)
          if (installBehaviour === 'err') throw new Error(TIMEOUT)
          return persisted
        },
      },
      bindings: {
        ...desktopApi,
        ...finderInstallCard,
        T,
        usePlatform: () => ({ name: platform, resolved: true }),
        useRegionLabel: () => 'Stored in the EU',
        shellIntegrationLabel: () => (windows ? 'Explorer integration' : 'Finder integration'),
        thisDeviceNoun: () => (windows ? 'this PC' : 'this Mac'),
        shellIntegrationCommandsFor: () => ({ state: stateName, install: installName }),
        SettingsSectionShell: 'section', PageHeader: 'header', Card: 'card', PrimaryBtn: 'button', Chip: 'chip',
      },
    })
    mounted.push(m)
    await m.flush()
    return m
  }

  for (const platform of ['macos', 'windows'] as const) {
    const verb = platform === 'macos' ? 'Install' : 'Enable'
    test(`${platform}: a failed ${verb.toLowerCase()} returned as an error is one inline surface, not a toast`, async () => {
      const m = await openPanel(platform, 'err')
      await m.click(verb)
      expect(visibleErrorSurfaces(m)).toEqual([`inline: ${TIMEOUT}`])
      expect(m.toasts).toEqual([])
    })
    test(`${platform}: a failed ${verb.toLowerCase()} returned as a state is one inline surface, not a silent nothing`, async () => {
      const m = await openPanel(platform, 'state')
      await m.click(verb)
      expect(visibleErrorSurfaces(m)).toEqual([`inline: ${TIMEOUT}`])
      expect(m.toasts).toEqual([])
    })
    test(`${platform}: success shows no error surface`, async () => {
      const m = await openPanel(platform, 'success')
      await m.click(verb)
      expect(visibleErrorSurfaces(m)).toEqual([])
      expect(m.toasts).toEqual([])
    })
  }
})
