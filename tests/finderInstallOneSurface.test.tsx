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
 *
 * Task 17 (spec 2026-10-06 §10): on macOS nothing installs by hand any more. SyncFolder and the
 * main-window Settings panel follow the reconciler through `useFinderSetup`, so the install-failure
 * tests below now run on LINUX, where the install-era commands are unchanged (the proof that the
 * non-macOS pane did not move), and the macOS behaviour has its own describes at the end.
 */
import { afterEach, describe, expect, test } from 'bun:test'
import * as desktopApi from '../src/desktopApi'
import * as finderInstallCard from '../src/finderInstallCard'
import * as finderSetupCopy from '../src/finderSetupCopy'
import { T } from '../src/windows/ui'
import { mount, textOf, visibleErrorSurfaces, type Mounted } from './fixtures/componentHarness'
import { finderBus, finderView, tick, useFinderSetupModule } from './fixtures/finderSetupHarness'

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
    desktop_platform: () => 'linux',
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

/**
 * SyncFolder runs the REAL `useFinderSetup` (switched on only for a Mac) against a scripted bus.
 * `extra` is what a macOS test adds; the Windows/Linux tests bind none of the Finder-setup copy, so
 * a ReferenceError there would mean the non-macOS pane evaluated a macOS identifier.
 */
function mountSyncFolder(backend: Record<string, any>, opts: { caps?: string | null; extra?: Record<string, unknown> } = {}) {
  const bus = finderBus()
  const m = mount('pages/SyncFolder.tsx', 'SyncFolder', {
    expand: true,
    backend,
    hookModules: [useFinderSetupModule(bus)],
    bindings: { ...desktopApi, ...finderInstallCard, useCapabilities: () => (opts.caps ? { host_os: opts.caps } : null), ...opts.extra },
  })
  mounted.push(m)
  return { m, bus }
}

async function openFinderPane(shape: Shape, persistedBefore: string | null, gate?: { release: Promise<void> }) {
  const { m } = mountSyncFolder(finderBackend(shape, persistedBefore, gate))
  await m.flush()
  return m
}

describe('SyncFolder on Windows/Linux (unchanged): Finder location pane', () => {
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
    const { m: w } = mountSyncFolder({ ...finderBackend('success', null), desktop_platform: () => 'windows', pick_sync_root: () => { throw new Error('picker crashed') } })
    await w.flush()
    await w.click('Choose location')
    expect(w.toasts.map((t) => t.title)).toEqual(['Couldn’t open the folder picker'])
    expect(visibleErrorSurfaces(w)).toEqual([`toast: Couldn’t open the folder picker — picker crashed`])
  })

  test('it still installs through the install-era commands, and never reads or listens to the reconciler', async () => {
    const { m, bus } = mountSyncFolder(finderBackend('success', null), { caps: 'linux' })
    await m.flush(); await m.flush()
    await m.click('Install in Finder')
    const names = m.calls.map((c) => c.name)
    expect(names).toContain('finder_location_state')
    expect(names).toContain('install_finder_location')
    expect(names).not.toContain('finder_setup_state')
    expect(bus.registered).toBe(0)
  })
})

describe('SyncFolder on macOS follows the reconciler (spec §10)', () => {
  const settle = async (m: Mounted) => { for (let i = 0; i < 6; i++) { await m.flush(); await tick() } }
  const btns = (m: Mounted) => m.elements().filter((el) => el.type === 'button').map((el) => textOf(el.props.children).trim())
  const names = (m: Mounted) => m.calls.map((c) => c.name)
  const count = (m: Mounted, name: string) => m.calls.filter((c) => c.name === name).length
  const INSTALL_ERA = ['install_finder_location', 'finder_location_state', 'continue_without_finder_location', 'finder_domain_user_enabled']

  /** `platform: 'fails'` is a desktop_platform that rejects; `caps` is the capability snapshot's host OS. */
  function macBackend(over: { view?: unknown; platform?: string } = {}) {
    const { view = finderView(), platform = 'macos' } = over
    return {
      desktop_platform: () => { if (platform === 'fails') throw new Error('desktop_platform is down'); return platform },
      sync_status: () => ({ logged_in: true, engine: 'running', sync_root: null, syncing: 0, cloud_only: 0, conflicts: 0 }),
      finder_setup_state: () => { if (view instanceof Error) throw view; return view },
      finder_setup_retry: () => undefined,
      open_finder_location: () => undefined,
      open_login_items_and_extensions_settings: () => undefined,
      reset_macos_integration: () => ({ pending_operations_preserved: 0, removed_cache_files: 0, warnings: [] }),
    }
  }
  async function openMac(over: { view?: unknown; platform?: string } = {}, caps: string | null = 'macos') {
    const { m, bus } = mountSyncFolder(macBackend(over), { caps, extra: { finderStatusPill: finderSetupCopy.finderStatusPill } })
    await settle(m)
    return { m, bus }
  }

  test('no Install button in any state; Open in Finder only when Beebeeb is in Finder', async () => {
    for (const state of [finderView(), finderView({ setup: 'ready' }), finderView({ setup: 'failed', reason: 'timeout' }), finderView({ setup: 'user_disabled', reason: 'user_disabled' })]) {
      const { m } = await openMac({ view: state })
      expect(btns(m).join('|')).not.toMatch(/Install/)
      expect(btns(m).includes('Open in Finder')).toBe(state.setup === 'ready')
      expect(btns(m).includes('Choose location')).toBe(false)
      expect(names(m).filter((n) => INSTALL_ERA.includes(n))).toEqual([])
    }
  })

  test('the pill follows the reconciler: Adding, then Installed on the event, with no second read', async () => {
    const { m, bus } = await openMac()
    expect(textOf(m.tree())).toContain('Adding Beebeeb to Finder…')
    expect(m.elements().filter((el) => el.props.className === 'status-pill').map((el) => textOf(el.props.children).trim())).toEqual(['Adding'])
    bus.emit(finderView({ setup: 'ready' }))
    await m.flush()
    expect(m.elements().filter((el) => el.props.className === 'status-pill').map((el) => textOf(el.props.children).trim())).toEqual(['Installed'])
    expect(btns(m)).toContain('Open in Finder')
    expect(count(m, 'finder_setup_state')).toBe(1)
    expect(bus.registered).toBe(1)
  })

  test('adding is a neutral status line, not an error, with no action', async () => {
    const { m } = await openMac()
    expect(visibleErrorSurfaces(m)).toEqual([])
    const notice = m.elements().filter((el) => el.props.role === 'status')
    expect(notice.map((el) => textOf(el.props.children))).toEqual([finderSetupCopy.FINDER_ADDING_LINE])
    expect(btns(m)).not.toContain('Try again')
  })

  test('a failure is one inline alert with its sentence, and Try again only there', async () => {
    const { m } = await openMac({ view: finderView({ setup: 'failed', reason: 'timeout' }) })
    expect(visibleErrorSurfaces(m)).toEqual([`inline: ${finderSetupCopy.FINDER_REASON_COPY.timeout.sentence}Try again`])
    expect(btns(m).filter((b) => b === 'Try again')).toHaveLength(1)
    expect(m.toasts).toEqual([])
    expect(m.elements().filter((el) => el.props['data-error-surface'] === 'finder-setup').map((el) => el.props.role)).toEqual(['alert'])
  })

  test('a failure notice\'s Try again asks the reconciler (finder_setup_retry) and does not re-read the state', async () => {
    const { m } = await openMac({ view: finderView({ setup: 'failed', reason: 'timeout' }) })
    await m.click('Try again')
    expect(count(m, 'finder_setup_retry')).toBe(1)
    expect(count(m, 'finder_setup_state')).toBe(1)
  })

  test('a failed action is one toast, and adds no second inline surface', async () => {
    const backend = { ...macBackend({ view: finderView({ setup: 'failed', reason: 'timeout' }) }), finder_setup_retry: () => { throw new Error('no reconciler') } }
    const { m } = mountSyncFolder(backend, { caps: 'macos', extra: { finderStatusPill: finderSetupCopy.finderStatusPill } })
    await settle(m)
    await m.click('Try again')
    expect(m.toasts.map((t) => t.title)).toEqual(['Couldn’t try again'])
    expect(visibleErrorSurfaces(m).filter((surface) => surface.startsWith('inline:'))).toHaveLength(1)
  })

  test('a user-disabled extension is a neutral status with Open System Settings, never a red error', async () => {
    const { m } = await openMac({ view: finderView({ setup: 'user_disabled', reason: 'user_disabled' }) })
    expect(visibleErrorSurfaces(m)).toEqual([])
    const notice = m.elements().filter((el) => el.props.role === 'status')
    expect(notice).toHaveLength(1)
    expect(textOf(notice[0].props.children)).toContain(finderSetupCopy.FINDER_REASON_COPY.user_disabled.sentence)
    await m.click('Open System Settings')
    expect(count(m, 'open_login_items_and_extensions_settings')).toBe(1)
  })

  test('a state that cannot be read is a neutral line with ONE Try again that reads again; never "Adding" (lead ruling 7a)', async () => {
    const { m } = await openMac({ view: new Error('no reconciler') })
    const text = textOf(m.tree())
    expect(text).toContain(finderSetupCopy.FINDER_UNAVAILABLE_LINE)
    expect(text).not.toContain('Adding')
    expect(m.elements().filter((el) => el.props.className === 'status-pill').map((el) => textOf(el.props.children).trim())).toEqual([finderSetupCopy.FINDER_STATUS_PILL_UNAVAILABLE])
    expect(visibleErrorSurfaces(m)).toEqual([])
    expect(btns(m).filter((b) => b === 'Try again')).toHaveLength(1)
    await m.click('Try again')
    await settle(m)
    expect(count(m, 'finder_setup_state')).toBe(2)
    expect(count(m, 'finder_setup_retry')).toBe(0)
  })

  test('Open in Finder opens the managed location, with no path to choose', async () => {
    const { m } = await openMac({ view: finderView({ setup: 'ready' }) })
    await m.click('Open in Finder')
    expect(m.calls.filter((c) => c.name === 'open_finder_location').map((c) => c.args)).toEqual([{ path: null }])
    expect(textOf(m.tree())).toContain('Beebeeb in Finder')
  })

  test('Reset reads the reconciler again, never finder_location_state', async () => {
    const { m } = await openMac({ view: finderView({ setup: 'ready' }) })
    await m.click('Reset Finder integration…')
    await m.click('Reset Finder integration')
    await settle(m)
    expect(count(m, 'reset_macos_integration')).toBe(1)
    expect(count(m, 'finder_setup_state')).toBe(2)
    expect(names(m).filter((n) => INSTALL_ERA.includes(n))).toEqual([])
  })

  test('a Mac whose desktop_platform read fails is still a Mac: the snapshot decides, and nothing install-era is offered or called', async () => {
    for (const platform of ['fails', 'unknown']) {
      const { m } = await openMac({ platform }, 'macos')
      expect(btns(m).join('|')).not.toMatch(/Install|Choose location/)
      expect(names(m).filter((n) => INSTALL_ERA.includes(n))).toEqual([])
      expect(count(m, 'finder_setup_state')).toBe(1)
    }
  })

  test('the first paint of a Mac is already the Mac pane: no moment with an Install button', () => {
    const { m } = mountSyncFolder(macBackend(), { caps: 'macos', extra: { finderStatusPill: finderSetupCopy.finderStatusPill } })
    expect(btns(m).join('|')).not.toMatch(/Install|Choose location/)
    expect(textOf(m.tree())).toContain('system-managed Finder location')
  })

  test('only when neither desktop_platform nor the snapshot can say does the pane take the Windows/Linux path, as Onboarding does', async () => {
    const { m } = mountSyncFolder({ ...macBackend({ platform: 'fails' }), finder_location_state: () => missing }, { caps: null })
    await settle(m)
    expect(names(m)).toContain('finder_location_state')
    expect(names(m)).not.toContain('finder_setup_state')
  })
})

describe('Settings panel: Windows (ExplorerIntegrationPanel) is unchanged; macOS (MacFinderIntegrationPanel) follows the reconciler', () => {
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

  describe('macOS: MacFinderIntegrationPanel shows the reconciler\'s state and offers no install (R5)', () => {
    const settle = async (m: Mounted) => { for (let i = 0; i < 6; i++) { await m.flush(); await tick() } }
    const btns = (m: Mounted) => m.elements().filter((el) => el.type === 'button').map((el) => textOf(el.props.children).trim())
    const chips = (m: Mounted) => m.elements().filter((el) => el.type === 'chip').map((el) => textOf(el.props.children).trim())
    const count = (m: Mounted, name: string) => m.calls.filter((c) => c.name === name).length

    async function openMacPanel(view: unknown, over: Record<string, any> = {}) {
      const bus = finderBus()
      const m = mount('windows/views/SettingsView.tsx', 'MacFinderIntegrationPanel', {
        expand: true,
        backend: {
          finder_setup_state: () => { if (view instanceof Error) throw view; return view },
          finder_setup_retry: () => undefined,
          open_login_items_and_extensions_settings: () => undefined,
          ...over,
        },
        hookModules: [useFinderSetupModule(bus)],
        bindings: {
          ...desktopApi,
          ...finderSetupCopy,
          T,
          useRegionLabel: () => 'Stored in the EU',
          SettingsSectionShell: 'section', PageHeader: 'header', Card: 'card', PrimaryBtn: 'button', Chip: 'chip',
        },
      })
      mounted.push(m)
      await settle(m)
      return { m, bus }
    }

    test('a failure is one inline alert with its one action, never a toast', async () => {
      const { m } = await openMacPanel(finderView({ setup: 'failed', reason: 'timeout' }))
      expect(visibleErrorSurfaces(m)).toEqual([`inline: ${finderSetupCopy.FINDER_REASON_COPY.timeout.sentence}Try again`])
      expect(btns(m).filter((b) => b === 'Try again')).toHaveLength(1)
      expect(m.toasts).toEqual([])
      expect(m.elements().filter((el) => el.props['data-error-surface'] === 'finder-setup').map((el) => el.props.role)).toEqual(['alert'])
    })

    test('a failure notice\'s Try again asks the reconciler (finder_setup_retry) and does not re-read the state', async () => {
      const { m } = await openMacPanel(finderView({ setup: 'failed', reason: 'timeout' }))
      await m.click('Try again')
      expect(count(m, 'finder_setup_retry')).toBe(1)
      expect(count(m, 'finder_setup_state')).toBe(1)
    })

    test('a failed action is one toast, and adds no second inline surface', async () => {
      const { m } = await openMacPanel(finderView({ setup: 'failed', reason: 'timeout' }), { finder_setup_retry: () => { throw new Error('no reconciler') } })
      await m.click('Try again')
      expect(m.toasts.map((t) => t.title)).toEqual(['Couldn’t try again'])
      expect(visibleErrorSurfaces(m).filter((surface) => surface.startsWith('inline:'))).toHaveLength(1)
    })

    test('a user-disabled extension is a neutral status with Open System Settings, not a red error', async () => {
      const { m } = await openMacPanel(finderView({ setup: 'user_disabled', reason: 'user_disabled' }))
      expect(visibleErrorSurfaces(m)).toEqual([])
      expect(m.toasts).toEqual([])
      const notices = m.elements().filter((el) => el.props.role === 'status')
      expect(notices).toHaveLength(1)
      expect(textOf(notices[0].props.children)).toContain(finderSetupCopy.FINDER_REASON_COPY.user_disabled.sentence)
      await m.click('Open System Settings')
      expect(count(m, 'open_login_items_and_extensions_settings')).toBe(1)
    })

    test('ready shows no error surface and an Active chip', async () => {
      const { m } = await openMacPanel(finderView({ setup: 'ready' }))
      expect(visibleErrorSurfaces(m)).toEqual([])
      expect(chips(m)).toEqual(['Active'])
      expect(textOf(m.tree())).toContain(finderSetupCopy.FINDER_READY_LINE)
    })

    test('adding is the adding line with no chip and no button; the event then makes it Active, with no second read', async () => {
      const { m, bus } = await openMacPanel(finderView())
      expect(textOf(m.tree())).toContain(finderSetupCopy.FINDER_ADDING_LINE)
      expect(chips(m)).toEqual([])
      expect(btns(m)).toEqual([])
      bus.emit(finderView({ setup: 'ready' }))
      await m.flush()
      expect(chips(m)).toEqual(['Active'])
      expect(count(m, 'finder_setup_state')).toBe(1)
    })

    test('a state that cannot be read says so with ONE Try again that reads again; never "Adding" (lead ruling 7a)', async () => {
      const { m } = await openMacPanel(new Error('no reconciler'))
      expect(textOf(m.tree())).toContain(finderSetupCopy.FINDER_UNAVAILABLE_LINE)
      expect(textOf(m.tree())).not.toContain('Adding')
      expect(visibleErrorSurfaces(m)).toEqual([])
      expect(btns(m).filter((b) => b === 'Try again')).toHaveLength(1)
      await m.click('Try again')
      await settle(m)
      expect(count(m, 'finder_setup_state')).toBe(2)
      expect(count(m, 'finder_setup_retry')).toBe(0)
    })

    test('no state offers Install or Enable, and no install-era command is called', async () => {
      for (const state of [finderView(), finderView({ setup: 'ready' }), finderView({ setup: 'failed', reason: 'unknown' }), finderView({ setup: 'user_disabled', reason: 'user_disabled' }), finderView({ setup: 'missing' }), new Error('x')]) {
        const { m } = await openMacPanel(state)
        expect(btns(m).join('|')).not.toMatch(/Install|Enable|Add to Finder/)
        expect(m.calls.map((c) => c.name).filter((n) => ['install_finder_location', 'finder_location_state', 'continue_without_finder_location', 'finder_domain_user_enabled'].includes(n))).toEqual([])
      }
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
