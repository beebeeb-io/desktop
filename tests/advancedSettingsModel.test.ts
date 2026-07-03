import { describe, expect, test } from 'bun:test'
import {
  CACHE_LIMIT_OPTIONS,
  DEFAULT_LOCAL_CACHE_LIMIT_BYTES,
  buildCacheLimitView,
  normalizeThemePreference,
  resolveThemePreference,
} from '../src/windows/advancedSettingsModel'

describe('advancedSettingsModel', () => {
  test('normalizes and resolves desktop theme preferences', () => {
    expect(normalizeThemePreference('light')).toBe('light')
    expect(normalizeThemePreference('dark')).toBe('dark')
    expect(normalizeThemePreference('system')).toBe('system')
    expect(normalizeThemePreference('unknown')).toBe('system')

    expect(resolveThemePreference('system', true)).toBe('dark')
    expect(resolveThemePreference('system', false)).toBe('light')
    expect(resolveThemePreference('dark', false)).toBe('dark')
  })

  test('builds total local cache usage and warns when pinned bytes exceed the cap', () => {
    expect(DEFAULT_LOCAL_CACHE_LIMIT_BYTES).toBe(100_000_000_000)
    expect(CACHE_LIMIT_OPTIONS.map((option) => option.label)).toEqual(['5 GB', '25 GB', '100 GB', 'Unlimited'])

    const overPinned = buildCacheLimitView({
      cacheBytes: 3_000_000_000,
      pinnedBytes: 7_000_000_000,
      limitBytes: 5_000_000_000,
    })

    expect(overPinned.totalBytes).toBe(10_000_000_000)
    expect(overPinned.capBytes).toBe(5_000_000_000)
    expect(overPinned.pinnedExceedsCap).toBe(true)
    expect(overPinned.warning).toContain('Pinned files use')

    const unlimited = buildCacheLimitView({
      cacheBytes: 3_000_000_000,
      pinnedBytes: 7_000_000_000,
      limitBytes: 0,
    })
    expect(unlimited.capBytes).toBeNull()
    expect(unlimited.percentUsed).toBeNull()
    expect(unlimited.warning).toBeNull()
  })
})
