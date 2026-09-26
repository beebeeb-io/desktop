/**
 * Task 1546 finding 1: the macOS Account page (`src/pages/Account.tsx`)
 * showed identity/vault-lock/diagnostics/sign-out only — no plan, no trial
 * state, no quota, no renewal date, no upgrade path anywhere. A repo-wide
 * grep showed `accountSubscription()`/`accountUsage()` were called ONLY from
 * Windows-only files (WindowsApp.tsx, InsightsView.tsx, AccountView.tsx),
 * never from the macOS component tree App.tsx actually renders.
 *
 * This is a source-scan test (no jsdom/render harness exists in this repo —
 * see `tests/noHardcodedThemeColors.test.ts` for the established pattern)
 * asserting the macOS Account page now fetches the subscription and offers
 * an upgrade/billing path, and that VersionCenter's quota-blocked rows now
 * do too.
 */
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

function readSrc(relPath: string): string {
  return readFileSync(join(import.meta.dir, '..', 'src', relPath), 'utf8')
}

describe('macOS Account page plan UI', () => {
  test('fetches the account subscription (not just identity/vault-lock)', () => {
    const src = readSrc('pages/Account.tsx')
    expect(src).toContain('accountSubscription')
  })

  test('offers a billing/upgrade path', () => {
    const src = readSrc('pages/Account.tsx')
    expect(src).toContain('BILLING_URL')
  })

  test('shows the subscription status, including a Trialing state', () => {
    // Reuses planStatusTone (tested directly in planPresentation.test.ts),
    // which already treats "trialing" as the live/good state — this just
    // confirms the page renders the status value at all, not a hardcoded one.
    const src = readSrc('pages/Account.tsx')
    expect(src).toContain('titleCasePlan(plan.sub.status)')
  })
})

describe('Versions & conflicts quota row', () => {
  test('a quota-blocked review row is wired to an actionable button, not left inert', () => {
    const src = readSrc('pages/VersionCenter.tsx')
    expect(src).toContain('reviewEntryAction')
  })
})
