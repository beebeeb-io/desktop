import { describe, expect, test } from 'bun:test'
import { COMPACT_NAV_ITEMS, compactPageFromString } from '../src/compactNavigation'

describe('compactNavigation', () => {
  test('hides shared roots while sending legacy shared navigation to the file surface', () => {
    expect(COMPACT_NAV_ITEMS.map((item) => item.id)).not.toContain('shared')
    expect(COMPACT_NAV_ITEMS.map((item) => item.label)).not.toContain('Shared roots')
    expect(compactPageFromString('shared')).toBe('finder')
  })
})
