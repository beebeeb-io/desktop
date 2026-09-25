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
})
