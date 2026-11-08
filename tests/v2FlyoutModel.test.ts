import { describe, expect, test } from 'bun:test'
import { SCENARIOS, NOW } from '../src/v2/tray/fixtures'
import { flyoutView } from '../src/v2/tray/model'

const EXPECTED: Record<string, string> = {
  'up-to-date': 'up-to-date', syncing: 'syncing', paused: 'paused', 'needs-decision': 'needs-decision', offline: 'offline',
  stopped: 'stopped', 'almost-full': 'storage-almost-full', 'storage-full': 'storage-full', update: 'update-available',
  locked: 'locked', checking: 'checking', 'no-plan': 'no-plan', 'update-required': 'update-required',
  'finder-adding': 'finder-adding', 'finder-failed': 'finder-failed', 'signed-out': 'signed-out', 'last-50': 'up-to-date',
  longest: 'finder-failed', metered: 'metered',
}

describe('flyout state precedence', () => {
  for (const [name, kind] of Object.entries(EXPECTED)) {
    test(`${name} is ${kind}`, () => {
      const s = SCENARIOS[name]
      expect(flyoutView(s.snapshot, s.extras, NOW).kind).toBe(kind)
    })
  }
  test('update required wins over locked and paused', () => {
    const s = SCENARIOS.locked
    expect(flyoutView(s.snapshot, { updateRequired: true }, NOW).kind).toBe('update-required')
  })
  test('locked hides every row', () => {
    const s = SCENARIOS.locked
    expect(flyoutView(s.snapshot, s.extras, NOW).list.kind).toBe('hidden')
  })
  test('paused beats a conflict', () => {
    const s = SCENARIOS.paused
    const withConflict = { ...s.snapshot, conflicts: { count: 2, files: [] } }
    expect(flyoutView(withConflict, s.extras, NOW).kind).toBe('paused')
  })
  test('no state has more than one amber action', () => {
    for (const [name, s] of Object.entries(SCENARIOS)) {
      const v = flyoutView(s.snapshot, s.extras, NOW)
      expect(name && [v.action].filter((a) => a?.primary).length).toBeLessThanOrEqual(1)
    }
  })
})
