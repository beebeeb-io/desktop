// The sidebar's storage meter is billing usage, not a board example: the words under the bar come from the
// account's used_bytes and quota_bytes in decimal units (1 GB = 1000³), and the bar stays neutral grey.
// With no storage read the meter is absent, never a zeroed or example bar.
import { afterEach, describe, expect, test } from 'bun:test'
import { Suspense } from 'react'
import { command } from '../src/desktopApi'
import { Sidebar, storageLine } from '../src/v2/chrome/Sidebar'
import { PROFILE_TIMEOUT_MS, accountRowLabel } from '../src/v2/sections/settings/account/data'
import { mount, textOf, type Mounted } from './fixtures/componentHarness'

const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close() })

const SECTIONS = ['files', 'shared', 'photos', 'backups', 'trash', 'sync-history', 'available-offline', 'search', 'settings', 'open'] as const
const TITLES = Object.fromEntries(SECTIONS.map((section) => [section, section]))
const SECTION_COMPONENTS = Object.fromEntries(SECTIONS.map((section) => [section, () => null]))

const nav = () => ({ route: { section: 'files' }, go() {}, back() {}, forward() {} })

function shell(storage: unknown): Mounted {
  const source = {
    storage: async () => storage,
    sync: async () => null,
    subscribe: () => () => {},
    trash: async () => [],
  }
  const m = mount('v2/V2App.tsx', 'V2Shell', {
    backend: {
      account_display_name: () => ({ display_name: 'Anna Berg' }),
      account_profile: () => ({ user_id: 'u-1', email: 'anna@example.eu', email_verified: true, created_at: '2026-01-01T00:00:00Z' }),
    },
    expand: true,
    bindings: {
      command,
      useDriveSource: () => source,
      useNavigation: () => nav(),
      useSidebarCounts: () => ({}),
      useShortcuts: () => {},
      SECTION_COMPONENTS,
      TITLES,
      sidebarRow: (section: string) => section,
      accountRowLabel,
      PROFILE_TIMEOUT_MS,
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

const summary = (usedBytes: number, quotaBytes: number) => ({
  usedBytes,
  quotaBytes,
  filesBytes: usedBytes,
  photosBytes: 0,
  backupsBytes: 0,
})

describe('the sidebar storage meter uses the account bytes', () => {
  test('1.5 GB of 2 GB, in decimal units', () => {
    expect(storageLine(1_500_000_000, 2_000_000_000)).toBe('1.5 GB of 2 GB')
  })

  test('zero used reads 0 B of the quota', () => {
    expect(storageLine(0, 2_000_000_000)).toBe('0 B of 2 GB')
  })

  test('no storage read means no meter', async () => {
    const m = shell(null)
    await m.flush()
    expect(m.elements().filter((el) => el.props.role === 'meter').length).toBe(0)
  })

  test('a storage read draws a neutral grey bar, never amber', async () => {
    const m = shell(summary(1_500_000_000, 2_000_000_000))
    await m.flush()
    const meter = m.elements().find((el) => el.props.role === 'meter')
    expect(meter).toBeDefined()
    // The track is bg-paper-3; the fill inside it is the bar, and it is bg-ink-3 (never the amber accent).
    const bar = m.elements().find((el) => String(el.props.className ?? '').includes('bg-ink-3'))
    expect(bar).toBeDefined()
    expect(String(bar!.props.className)).not.toContain('amber')
    const sidebar = m.elements().find((el) => el.type === 'nav')
    expect(textOf(sidebar!.props.children)).toContain('1.5 GB of 2 GB')
  })
})
