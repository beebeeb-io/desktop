// The bottom-left account row shows the person's name (DW-0401): the display name when it is set, the profile's
// email when the name is null or empty, and "Account" only while the read is in flight or failed. Rendered through
// the real V2Shell and Sidebar, with the account commands scripted.
import { afterEach, describe, expect, test } from 'bun:test'
import { Suspense } from 'react'
import { command } from '../src/desktopApi'
import { Sidebar } from '../src/v2/chrome/Sidebar'
import { PROFILE_TIMEOUT_MS, accountRowLabel } from '../src/v2/sections/settings/account/data'
import { mount, textOf, type Mounted } from './fixtures/componentHarness'

const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close() })

const SECTIONS = ['files', 'shared', 'photos', 'backups', 'trash', 'sync-history', 'available-offline', 'search', 'settings', 'open'] as const
const TITLES = Object.fromEntries(SECTIONS.map((section) => [section, section]))
const SECTION_COMPONENTS = Object.fromEntries(SECTIONS.map((section) => [section, () => null]))

interface Nav {
  route: { section: string }
  go(): void
  back(): void
  forward(): void
}

function shell(backend: Record<string, (args: unknown) => unknown>, nav: Nav): Mounted {
  const source = {
    storage: async () => null,
    sync: async () => null,
    subscribe: () => () => {},
    trash: async () => [],
  }
  const m = mount('v2/V2App.tsx', 'V2Shell', {
    backend,
    expand: true,
    bindings: {
      command,
      accountRowLabel,
      PROFILE_TIMEOUT_MS,
      useDriveSource: () => source,
      useNavigation: () => nav,
      useSidebarCounts: () => ({}),
      useShortcuts: () => {},
      SECTION_COMPONENTS,
      TITLES,
      sidebarRow: (section: string) => section,
      CaptionBar: () => null,
      Sidebar,
      Suspense,
      StatusLine: () => null,
      GuideHost: () => null,
    },
  })
  mounted.push(m)
  return m
}

const nav = (section = 'files'): Nav => ({ route: { section }, go() {}, back() {}, forward() {} })

/** A nav whose route the test can move, as the real navigation does. */
function movableNav(section = 'files'): Nav {
  const route = { section }
  return { route, go() {}, back() {}, forward() {} }
}

const profile = (email: string) => ({ user_id: 'u-1', email, email_verified: true, created_at: '2026-01-01T00:00:00Z' })

/** The window's text, with the account row's own span read out of the real Sidebar. */
function rowText(m: Mounted): string {
  const span = m.elements().find((el) => el.type === 'span' && String(el.props.className ?? '').includes('truncate text-[13px]'))
  return span ? textOf(span).trim() : ''
}

describe('the sidebar account row shows your name (DW-0401)', () => {
  test('a display name is the row', async () => {
    const m = shell({
      account_display_name: () => ({ display_name: 'Anna Berg' }),
      account_profile: () => profile('anna@example.eu'),
    }, nav())
    await m.flush()
    expect(rowText(m)).toBe('Anna Berg')
  })

  test('a null or empty display name falls back to the profile email', async () => {
    for (const display_name of [null, '', '   ']) {
      const m = shell({
        account_display_name: () => ({ display_name }),
        account_profile: () => profile('anna@example.eu'),
      }, nav())
      await m.flush()
      expect(rowText(m)).toBe('anna@example.eu')
    }
  })

  test('while the read is in flight the row says Account; a failed read with nothing known leaves it empty', async () => {
    const pending = shell({
      account_display_name: () => new Promise(() => {}),
      account_profile: () => new Promise(() => {}),
    }, nav())
    await pending.flush()
    expect(rowText(pending)).toBe('Account')

    // The account page holds the error; the row does not keep "Account" as if it were the person's name.
    const failed = shell({
      account_display_name: () => { throw new Error('network') },
      account_profile: () => { throw new Error('network') },
    }, nav())
    await failed.flush()
    expect(rowText(failed)).toBe('')
  })

  test('moving to another section re-reads, so a rename shows without a restart', async () => {
    let displayName = 'Anna Berg'
    const route = movableNav('files')
    const m = shell({
      account_display_name: () => ({ display_name: displayName }),
      account_profile: () => profile('anna@example.eu'),
    }, route)
    await m.flush()
    expect(rowText(m)).toBe('Anna Berg')

    displayName = 'Anna Berg-van Dijk'
    route.route.section = 'settings'
    m.render()
    await m.flush()
    expect(rowText(m)).toBe('Anna Berg-van Dijk')
  })
})
