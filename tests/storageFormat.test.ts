import { describe, expect, test } from 'bun:test'
import { UNKNOWN_SIZE, formatStorageSize, formatStorageUsage, systemLocale } from '../src/storageFormat'

describe('formatStorageSize (task 1683 slice 2)', () => {
  test('84,3 GB in Dutch and 84.3 GB in English, from the same number of bytes', () => {
    expect(formatStorageSize(84_300_000_000, 'nl')).toBe('84,3 GB')
    expect(formatStorageSize(84_300_000_000, 'nl-NL')).toBe('84,3 GB')
    expect(formatStorageSize(84_300_000_000, 'en')).toBe('84.3 GB')
    expect(formatStorageSize(84_300_000_000, 'en-GB')).toBe('84.3 GB')
  })

  test('a trailing zero decimal is trimmed, so a round quota reads 200 GB', () => {
    expect(formatStorageSize(200_000_000_000, 'nl')).toBe('200 GB')
    expect(formatStorageSize(200_000_000_000, 'en')).toBe('200 GB')
    expect(formatStorageSize(1_000_000_000_000, 'en')).toBe('1 TB')
  })

  test('at most one decimal, in every unit including 1 TB and up (formatBytes drops them in GB)', () => {
    expect(formatStorageSize(1_234_567_890_000, 'en')).toBe('1.2 TB')
    expect(formatStorageSize(1_234_567_890_000, 'nl')).toBe('1,2 TB')
    expect(formatStorageSize(1_250_000_000, 'en')).toBe('1.3 GB')
    expect(formatStorageSize(12_340_000, 'en')).toBe('12.3 MB')
    expect(formatStorageSize(1_500, 'en')).toBe('1.5 KB')
  })

  test('bytes below 1 KB have no decimal', () => {
    expect(formatStorageSize(0, 'en')).toBe('0 B')
    expect(formatStorageSize(999, 'en')).toBe('999 B')
    expect(formatStorageSize(1000, 'en')).toBe('1 KB')
  })

  test('a value that rounds up to 1,000 moves to the next unit', () => {
    expect(formatStorageSize(999_960_000, 'en')).toBe('1 GB')
    expect(formatStorageSize(999_949_000, 'en')).toBe('999.9 MB')
    expect(formatStorageSize(999_960_000_000, 'nl')).toBe('1 TB')
  })

  test('large numbers group by the locale and never overflow the unit list', () => {
    // 1.5e18 bytes (1.5 EB) stays in PB, the top unit, and groups its thousands: 1,500 PB.
    expect(formatStorageSize(1_500_000_000_000_000_000, 'en')).toBe('1,500 PB')
    expect(formatStorageSize(1_500_000_000_000_000_000, 'nl')).toBe('1.500 PB')
  })

  test('not a number, infinite or negative is the unknown dash, never NaN', () => {
    for (const bad of [NaN, Infinity, -Infinity, -1, null, undefined]) {
      expect(formatStorageSize(bad as number, 'en')).toBe(UNKNOWN_SIZE)
    }
  })

  test('an invalid locale tag falls back to English instead of throwing', () => {
    expect(formatStorageSize(84_300_000_000, 'not a locale')).toBe('84.3 GB')
  })

  test('the default locale is the webview language, else English', () => {
    expect(typeof systemLocale()).toBe('string')
    expect(systemLocale().length).toBeGreaterThan(0)
  })

  test('usage formats both numbers in the same locale', () => {
    expect(formatStorageUsage(84_300_000_000, 200_000_000_000, 'nl')).toEqual({ used: '84,3 GB', quota: '200 GB' })
    expect(formatStorageUsage(null, 200_000_000_000, 'en')).toEqual({ used: UNKNOWN_SIZE, quota: '200 GB' })
  })
})
