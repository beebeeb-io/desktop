/**
 * `ShellOrFinderPanel` (src/windows/views/SettingsView.tsx) is the one place the main window's
 * integration panel is chosen: macOS reads the reconciler (`MacFinderIntegrationPanel`), every other
 * host keeps the Explorer/shell panel and must still receive the live sync status (task 1646).
 *
 * The production dispatcher is mounted with `usePlatform` scripted per host, and the two panels
 * replaced by tagged stand-ins so the test sees WHICH one was chosen and WHAT it was given.
 * tests/syncRoot.test.ts only proves `renderPanel` hands the status to the dispatcher, and the
 * source contract only pins the macOS arm; this is the proof of the other arm.
 */
import { afterEach, describe, expect, test } from 'bun:test'
import { mount, type Mounted } from './fixtures/componentHarness'

const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close() })

const status = { logged_in: true, engine: 'running', sync_root: 'D:\\Private files\\Sync', syncing: 0, cloud_only: 0, conflicts: 0 }

function open(platform: { name: string | null; resolved: boolean }, props: { status: unknown } = { status }) {
  const m = mount('windows/views/SettingsView.tsx', 'ShellOrFinderPanel', {
    backend: {},
    props,
    bindings: {
      usePlatform: () => platform,
      MacFinderIntegrationPanel: 'mac-finder-panel',
      ExplorerIntegrationPanel: 'explorer-panel',
    },
  })
  mounted.push(m)
  return m.tree() as { type: string; props: Record<string, unknown> }
}

describe('ShellOrFinderPanel dispatches by the resolved host', () => {
  for (const name of ['windows', 'linux']) {
    test(`${name}: the Explorer/shell panel, given the real status, and no Mac panel`, () => {
      const panel = open({ name, resolved: true })
      expect(panel.type).toBe('explorer-panel')
      expect(panel.props.status).toBe(status)
    })
  }

  test('a host that has not resolved yet: the Explorer/shell panel (it calls no command until it does), given the status', () => {
    const panel = open({ name: null, resolved: false })
    expect(panel.type).toBe('explorer-panel')
    expect(panel.props.status).toBe(status)
  })

  test('a platform name that is not resolved is not macOS yet: still the Explorer/shell panel', () => {
    const panel = open({ name: 'macos', resolved: false })
    expect(panel.type).toBe('explorer-panel')
    expect(panel.props.status).toBe(status)
  })

  test('the Explorer/shell panel gets a missing status as missing, not as something else', () => {
    const panel = open({ name: 'windows', resolved: true }, { status: null })
    expect(panel.type).toBe('explorer-panel')
    expect(panel.props.status).toBeNull()
  })

  test('macOS (resolved): the Mac Finder panel, and no Explorer/shell panel', () => {
    const panel = open({ name: 'macos', resolved: true })
    expect(panel.type).toBe('mac-finder-panel')
    expect('status' in panel.props).toBe(false)
  })
})
