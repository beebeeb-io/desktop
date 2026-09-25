import { describe, expect, test } from 'bun:test'
import { planRenewalCopy, planStatusTone, quotaPercent, titleCasePlan } from '../src/planPresentation'

describe('planPresentation', () => {
  test('titleCasePlan capitalizes the first letter only', () => {
    expect(titleCasePlan('active')).toBe('Active')
    expect(titleCasePlan('past_due')).toBe('Past_due')
    expect(titleCasePlan('')).toBe('')
  })

  test('planStatusTone reads active/trialing as the live state, everything else neutral', () => {
    expect(planStatusTone('active')).toBe('green')
    expect(planStatusTone('Trialing')).toBe('green')
    expect(planStatusTone('past_due')).toBe('neutral')
    expect(planStatusTone('canceled')).toBe('neutral')
    expect(planStatusTone('')).toBe('neutral')
  })

  test('quotaPercent clamps to 0..100 and treats a zero/unknown quota as 0%', () => {
    expect(quotaPercent(50, 100)).toBe(50)
    expect(quotaPercent(150, 100)).toBe(100)
    expect(quotaPercent(-10, 100)).toBe(0)
    expect(quotaPercent(10, 0)).toBe(0)
  })

  test('planRenewalCopy names an honest "no renewal" line for the free-plan null branch', () => {
    expect(planRenewalCopy(null)).toBe("No renewal — this plan doesn't expire")
    expect(planRenewalCopy(undefined)).toBe("No renewal — this plan doesn't expire")
  })

  test('planRenewalCopy shows the absolute date plus a relative phrase for an upcoming renewal', () => {
    const now = new Date('2026-09-25T00:00:00Z')
    expect(planRenewalCopy('2026-09-25T00:00:00Z', now)).toBe('25 Sep 2026 · today')
    expect(planRenewalCopy('2026-09-26T00:00:00Z', now)).toBe('26 Sep 2026 · tomorrow')
    expect(planRenewalCopy('2026-10-05T00:00:00Z', now)).toBe('5 Oct 2026 · in 10 days')
  })

  test('planRenewalCopy never fabricates a date for a malformed timestamp', () => {
    expect(planRenewalCopy('not-a-date')).toBe("No renewal — this plan doesn't expire")
  })

  // Regression coverage for two bugs found while chasing a CI-only failure
  // (task 1546 follow-up, 2026-09-26): the month abbreviation came from
  // Intl/CLDR data that differs across bun builds ("Sep" vs "Sept" for the
  // exact same en-GB request — confirmed same bun version, darwin/arm64 vs
  // linux/amd64), and the days-until math rounded a raw millisecond
  // duration instead of counting calendar-day boundaries, so "today" /
  // "tomorrow" / "in N days" could disagree with the printed absolute date.
  // Both bugs are timezone- and runtime-dependent, so a single-TZ pass
  // proves little — these were verified green under TZ=UTC, TZ=Europe/
  // Amsterdam and TZ=Pacific/Auckland (see commit notes / task file), and
  // are written to hold under any TZ by construction (fixed local-calendar
  // math, no Intl formatting).

  test('planRenewalCopy month abbreviation is a fixed 3-letter form, never runtime/locale-dependent "Sept"', () => {
    const now = new Date('2026-09-01T00:00:00Z')
    expect(planRenewalCopy('2026-09-25T00:00:00Z', now)).toContain('25 Sep 2026')
    expect(planRenewalCopy('2026-09-25T00:00:00Z', now)).not.toContain('Sept')
  })

  test('planRenewalCopy counts calendar-day boundaries, not a rounded raw-millisecond duration', () => {
    // Built from LOCAL date parts (not UTC 'Z' strings) so "1 calendar day
    // apart" holds under every TZ the test runs in: now=01:00, renewal=
    // 23:00 the next LOCAL day is a ~46h / 1.92 ms-day gap regardless of
    // offset (Math.round(1.92) -> 2 under the old code) but exactly one
    // calendar day -> "tomorrow".
    const now = new Date(2026, 8, 25, 1, 0, 0) // 25 Sep 2026, 01:00 local
    const renewal = new Date(2026, 8, 26, 23, 0, 0) // 26 Sep 2026, 23:00 local
    expect(planRenewalCopy(renewal.toISOString(), now)).toBe('26 Sep 2026 · tomorrow')
  })

  test('planRenewalCopy near a local midnight boundary keeps the absolute date on the same local day as the relative phrase', () => {
    // now and the renewal are exactly 24h apart in wall-clock time, at
    // 10:30 UTC — far enough from UTC midnight that every real timezone
    // (UTC-12 .. UTC+14, including Pacific/Auckland's NZDT) still reads
    // both as day-of-month `upcoming.getDate()` locally, so this pins the
    // absolute date and the "tomorrow" label to the SAME local day
    // regardless of the test runner's TZ.
    const now = new Date('2026-09-25T10:30:00Z')
    const upcoming = new Date('2026-09-26T10:30:00Z')
    const copy = planRenewalCopy(upcoming.toISOString(), now)
    expect(copy).toBe(`${upcoming.getDate()} Sep 2026 · tomorrow`)
  })
})
