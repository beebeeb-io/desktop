/**
 * When something happened (task 1683 slice 4): the parser that reads the three shapes of a time the app receives, and the
 * day and clock labels Review and Trash print. `tests/reviewModel.test.ts` and `tests/trashModel.test.ts` pin the
 * sentences built from them; this pins the readers themselves, so a change to one shape cannot hide behind a sentence.
 */
import { describe, expect, test } from 'bun:test'
import { clockLabel, dayLabel, parseWhen, whenLabel } from '../src/whenFormat'

const noon = new Date(2026, 9, 12, 12, 0, 0)

describe('parseWhen: seconds, milliseconds, the same digits as text, an ISO date', () => {
  test('all four spellings of one moment are one moment', () => {
    const seconds = Math.floor(noon.getTime() / 1000)
    for (const value of [seconds, noon.getTime(), String(seconds), String(noon.getTime()), noon.toISOString()]) {
      expect(parseWhen(value)?.getTime()).toBe(seconds * 1000)
    }
  })

  test('surrounding spaces are not part of the time', () => {
    expect(parseWhen(`  ${Math.floor(noon.getTime() / 1000)} `)?.getTime()).toBe(Math.floor(noon.getTime() / 1000) * 1000)
  })

  test('the epoch is "never" (the engine\'s way of saying it has no time), and so are text, nothing and not-a-number', () => {
    for (const value of [0, '0', -5, '', '   ', 'yesterday-ish', null, undefined, Number.NaN, Number.POSITIVE_INFINITY]) {
      expect(parseWhen(value as never)).toBeNull()
    }
  })

  test('a date late in the century is still seconds, and the first millisecond count is milliseconds', () => {
    expect(parseWhen(9_999_999_999)?.getFullYear()).toBe(2286)
    expect(parseWhen(10_000_000_001)?.getFullYear()).toBe(1970)
  })
})

describe('the labels', () => {
  const now = new Date(2026, 9, 25, 14, 30)

  test('the clock is 24-hour and padded', () => {
    expect(clockLabel(new Date(2026, 0, 1, 9, 5))).toBe('09:05')
    expect(clockLabel(new Date(2026, 0, 1, 0, 0))).toBe('00:00')
    expect(clockLabel(new Date(2026, 0, 1, 23, 59))).toBe('23:59')
  })

  test('a day is today, yesterday, the day and month, and the year too when it is not this one', () => {
    expect(dayLabel(new Date(2026, 9, 25, 0, 1), now)).toBe('today')
    expect(dayLabel(new Date(2026, 9, 24, 23, 59), now)).toBe('yesterday')
    expect(dayLabel(new Date(2026, 9, 23, 12, 0), now)).toBe('23 Oct')
    expect(dayLabel(new Date(2026, 0, 3, 12, 0), now)).toBe('3 Jan')
    expect(dayLabel(new Date(2025, 11, 31, 12, 0), now)).toBe('31 Dec 2025')
  })

  test('whenLabel adds the clock to today and yesterday only', () => {
    const at = (d: Date) => Math.floor(d.getTime() / 1000)
    expect(whenLabel(at(new Date(2026, 9, 25, 9, 12)), now)).toBe('today 09:12')
    expect(whenLabel(at(new Date(2026, 9, 24, 18, 3)), now)).toBe('yesterday 18:03')
    expect(whenLabel(at(new Date(2026, 9, 1, 18, 3)), now)).toBe('1 Oct')
    expect(whenLabel(null, now)).toBeNull()
  })
})
