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
import { mount, textOf, visibleErrorSurfaces, type Mounted } from './fixtures/componentHarness'

const TIMEOUT = 'Timed out waiting for the Beebeeb File Provider domain to become available'
const OTHER = 'Finder location must be absolute: relative/path'

const missing = { installed: false, path: null, status: 'missing', last_error: null, last_attempt_at: null, reason_category: null }
const failed = (message: string) => ({ installed: false, path: null, status: 'error', last_error: message, last_attempt_at: 2, reason_category: 'timeout' })
const userDisabledMessage = 'Beebeeb is turned off in System Settings. Open Login Items & Extensions, turn on Beebeeb under File Providers, then try again.'
const userDisabled = { installed: false, path: null, status: 'error', last_error: userDisabledMessage, last_attempt_at: 4, reason_category: 'user_disabled' }
const installedState = { installed: true, path: 'Beebeeb in Finder', status: 'installed', last_error: null, last_attempt_at: 3, reason_category: null }

/**
 * How the backend answers a failing install.
 *  - 'err'      old shape: failure saved AND returned as a rejected command (the screenshot-1 bug)
 *  - 'state'    D1 shape: failure saved and returned as an Ok state (installed: false, last_error set)
 *  - 'unsaved'  a failure before anything is persisted (validation), returned as a rejected command
 *  - 'success'  the install works
 */
type Shape = 'err' | 'state' | 'unsaved' | 'success' | 'user_disabled'

function finderBackend(shape: Shape, persistedBefore: string | null, gate?: { release: Promise<void> }) {
  let persisted: any = persistedBefore ? failed(persistedBefore) : missing
  return {
    desktop_platform: () => 'macos',
    sync_status: () => ({ logged_in: true, engine: 'running', sync_root: '/Users/fixture/Library/CloudStorage/Beebeeb', syncing: 0, cloud_only: 0, conflicts: 0 }),
    finder_location_state: () => persisted,
    install_windows_shell_integration: () => { throw new Error('windows-only command called on macOS') },
    open_login_items_and_extensions_settings: () => undefined,
    install_finder_location: async () => {
      if (gate) await gate.release
      if (shape === 'success') { persisted = installedState; return installedState }
      if (shape === 'unsaved') throw new Error(OTHER)
      if (shape === 'user_disabled') { persisted = userDisabled; return userDisabled }
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

  test('the inline banner is a live region, so a screen reader is told the install failed (the removed toast was role=alert)', async () => {
    const m = await openFinderPane('state', null)
    await m.click('Install in Finder')
    const banner = m.elements().filter((el) => el.props['data-error-surface'] === 'finder-install')
    expect(banner.map((el) => el.props.role)).toEqual(['alert'])
  })

  test('a user-disabled extension is ONE distinct, fixable notice (not a red error) with a System Settings action', async () => {
    const m = await openFinderPane('user_disabled', null)
    await m.click('Install in Finder')
    expect(visibleErrorSurfaces(m)).toEqual([])
    expect(m.toasts).toEqual([])
    const notices = m.elements().filter((el) => el.props['data-finder-state'] === 'user_disabled')
    expect(notices.length).toBe(1)
    expect(textOf(notices[0].props.children)).toContain(userDisabledMessage)
    expect(notices[0].props.role).toBe('status')
    await m.click('Open Login Items & Extensions')
    expect(m.calls.filter((c) => c.name === 'open_login_items_and_extensions_settings').length).toBe(1)
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

describe('Settings panel (ExplorerIntegrationPanel), same action, same rule on macOS; Windows is unchanged', () => {
  type PanelBehaviour = 'err' | 'state' | 'success' | 'user_disabled'
  async function openPanel(platform: 'macos' | 'windows', installBehaviour: PanelBehaviour, persistedBefore: string | null = null) {
    const windows = platform === 'windows'
    const installName = windows ? 'install_windows_shell_integration' : 'install_finder_location'
    const stateName = windows ? 'windows_shell_integration_state' : 'finder_location_state'
    let persisted: any = persistedBefore ? failed(persistedBefore) : missing
    const m = mount('windows/views/SettingsView.tsx', 'ExplorerIntegrationPanel', {
      backend: {
        [stateName]: () => persisted,
        [installName]: () => {
          if (installBehaviour === 'success') { persisted = installedState; return installedState }
          if (installBehaviour === 'user_disabled') { persisted = userDisabled; return userDisabled }
          persisted = failed(TIMEOUT)
          if (installBehaviour === 'err') throw new Error(TIMEOUT)
          return persisted
        },
        open_login_items_and_extensions_settings: () => undefined,
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

  describe('macOS: a failed install gates the row, so it is one inline banner', () => {
    test('a failed install returned as an error is one inline surface, not a toast', async () => {
      const m = await openPanel('macos', 'err')
      await m.click('Install')
      expect(visibleErrorSurfaces(m)).toEqual([`inline: ${TIMEOUT}`])
      expect(m.toasts).toEqual([])
    })
    test('a failed install returned as a state is one inline surface, not a silent nothing', async () => {
      const m = await openPanel('macos', 'state')
      await m.click('Install')
      expect(visibleErrorSurfaces(m)).toEqual([`inline: ${TIMEOUT}`])
      expect(m.toasts).toEqual([])
    })
    test('the banner is a live region (role=alert)', async () => {
      const m = await openPanel('macos', 'state')
      await m.click('Install')
      expect(m.elements().filter((el) => el.props['data-error-surface'] === 'finder-install').map((el) => el.props.role)).toEqual(['alert'])
    })
    test('success shows no error surface', async () => {
      const m = await openPanel('macos', 'success', TIMEOUT)
      await m.click('Install')
      expect(visibleErrorSurfaces(m)).toEqual([])
      expect(m.toasts).toEqual([])
    })
    test('a user-disabled extension is one distinct notice with a System Settings action, not a red error', async () => {
      const m = await openPanel('macos', 'user_disabled')
      await m.click('Install')
      expect(visibleErrorSurfaces(m)).toEqual([])
      expect(m.toasts).toEqual([])
      const notices = m.elements().filter((el) => el.props['data-finder-state'] === 'user_disabled')
      expect(notices.length).toBe(1)
      expect(textOf(notices[0].props.children)).toContain(userDisabledMessage)
      expect(notices[0].props.role).toBe('status')
      await m.click('Open Login Items & Extensions')
      expect(m.calls.filter((c) => c.name === 'open_login_items_and_extensions_settings').length).toBe(1)
    })
  })

  describe('Windows: spec section 11 says the toast rule is macOS only, so the panel is unchanged', () => {
    test('a failed enable is one toast and NO inline banner', async () => {
      const m = await openPanel('windows', 'err')
      await m.click('Enable')
      expect(m.toasts.map((t) => t.title)).toEqual(['Couldn’t enable Explorer integration'])
      expect(visibleErrorSurfaces(m)).toEqual([`toast: Couldn’t enable Explorer integration — ${TIMEOUT}`])
      expect(m.elements().filter((el) => el.props['data-error-surface'] != null)).toEqual([])
    })
    test('a failure saved by the first-run flow is NOT replayed as a banner every time Settings opens', async () => {
      const m = await openPanel('windows', 'success', TIMEOUT)
      expect(visibleErrorSurfaces(m)).toEqual([])
      expect(m.elements().filter((el) => el.props.role === 'alert')).toEqual([])
      expect(m.toasts).toEqual([])
    })
    test('success shows no error surface', async () => {
      const m = await openPanel('windows', 'success')
      await m.click('Enable')
      expect(visibleErrorSurfaces(m)).toEqual([])
      expect(m.toasts).toEqual([])
    })
  })
})
