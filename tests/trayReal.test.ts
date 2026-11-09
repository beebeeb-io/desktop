// The b's menu reflects the state the window shows (DW-1230, DW-1231): the label, the tooltip and
// the first (status) item all come from the same popover snapshot the flyout draws, and the two
// state-bearing action items (Pause/Resume, Lock/Unlock) follow it too.
import { describe, expect, test } from 'bun:test'
import { flyoutView, trayMenu } from '../src/v2/tray/model'
import { NOW, snapshot } from '../src/v2/tray/fixtures'

function menuFor(patch: Parameters<typeof snapshot>[0], extras = {}) {
  return trayMenu(flyoutView(snapshot(patch), { totalFiles: 1284, ...extras }, NOW), { paused: patch?.paused ?? false })
}

function ids(items: ReturnType<typeof menuFor>['items']): string[] {
  return items.filter((item) => !item.separator).map((item) => item.id)
}

describe('the b’s menu shows the real state', () => {
  test('syncing: the tooltip and the status item say Syncing', () => {
    const menu = menuFor({
      phase: 'syncing', sync_phase: 'syncing', engine: { state: 'syncing', files_remaining: 12, bytes_total: 120_000_000, bytes_done: 72_000_000, last_tick_ok_at: NOW },
    })
    expect(menu.icon).toBe('syncing')
    expect(menu.label).toBe('Syncing')
    expect(menu.tooltip).toBe('Beebeeb: syncing')
    expect(menu.items[0]).toEqual({ id: 'status', label: 'Syncing', disabled: true })
    expect(ids(menu.items)).toEqual(['status', 'open', 'explorer', 'pause', 'lock', 'check-updates', 'settings', 'quit'])
    expect(menu.items.find((item) => item.id === 'explorer')?.disabled).toBe(false)
    expect(menu.items.find((item) => item.id === 'pause')?.label).toBe('Pause syncing')
  })

  test('locked: the plain b, the tooltip says Locked and the menu offers Unlock', () => {
    const menu = menuFor({ phase: 'locked', account: { email: 'anna@example.eu', logged_in: true, vault_unlocked: false, auth_expired: false } })
    expect(menu.icon).toBe('up-to-date')
    expect(menu.label).toBe('Locked')
    expect(menu.tooltip).toBe('Beebeeb: locked')
    expect(menu.items[0]).toEqual({ id: 'status', label: 'Locked', disabled: true })
    expect(menu.items.find((item) => item.id === 'unlock')?.label).toBe('Unlock Beebeeb')
    expect(menu.items.some((item) => item.id === 'lock')).toBe(false)
  })

  test('a conflict: the tooltip and the status item say Needs a decision', () => {
    const menu = menuFor({ conflicts: { count: 2, files: [{ file_id: 'c1', file_name: 'Budget 2026.xlsx' }, { file_id: 'c2', file_name: 'Roadmap.pdf' }] } })
    expect(menu.icon).toBe('needs-decision')
    expect(menu.label).toBe('Needs a decision')
    expect(menu.tooltip).toBe('Beebeeb: needs a decision')
    expect(menu.items[0]).toEqual({ id: 'status', label: 'Needs a decision', disabled: true })
  })

  test('up to date: the tooltip and the status item say Up to date', () => {
    const menu = menuFor({})
    expect(menu.icon).toBe('up-to-date')
    expect(menu.label).toBe('Up to date')
    expect(menu.tooltip).toBe('Beebeeb: up to date')
    expect(menu.items[0]).toEqual({ id: 'status', label: 'Up to date', disabled: true })
    expect(menu.items.find((item) => item.id === 'pause')?.label).toBe('Pause syncing')
  })
})
