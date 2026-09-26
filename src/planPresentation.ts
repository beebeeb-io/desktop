/**
 * Pure presentation helpers for subscription/plan data, shared by the macOS
 * Account page's new Plan & subscription panel (task 1546 finding 1). Mirrors
 * the shape of the equivalent local (unexported, untested) helpers in
 * `windows/views/AccountView.tsx` — factored out here so the macOS surface
 * gets the same logic AND direct unit coverage, without touching the
 * already-shipped Windows view.
 */

/** Title-cases a free-form plan/status string from the server (e.g. "active" -> "Active"). */
export function titleCasePlan(value: string): string {
  if (!value) return value
  return value.charAt(0).toUpperCase() + value.slice(1)
}

/**
 * Maps a subscription status to a chip tone. "active"/"trialing" read as the
 * good/live state; everything else (past_due, canceled, frozen, ...) stays
 * neutral rather than borrowing the brand's single amber accent for a
 * non-encryption, non-primary-action surface.
 */
export function planStatusTone(status: string): 'green' | 'neutral' {
  const s = status.toLowerCase()
  return s === 'active' || s === 'trialing' ? 'green' : 'neutral'
}

/** Clamped storage-used percentage for the quota bar; 0 when quota is unknown/zero. */
export function quotaPercent(used: number, quota: number): number {
  if (!(quota > 0)) return 0
  return Math.min(100, Math.max(0, (used / quota) * 100))
}

const MONTH_ABBREVIATIONS = [
  'Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun',
  'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec',
] as const

/**
 * Integer index for a Date's LOCAL calendar day (its year/month/date in the
 * runtime's local timezone), independent of time-of-day. Diffing two of
 * these gives the exact number of local calendar-day boundaries crossed —
 * unlike a raw millisecond difference divided by 86_400_000 (the previous
 * approach), which drifts whenever the two timestamps aren't both at the
 * same time of day. Example: a renewal at 23:00 the day after `now` at
 * 01:00 is 1.92 ms-days away, which `Math.round` pushes to 2 — "in 2 days"
 * for something that is calendar-wise tomorrow.
 */
function localDayIndex(date: Date): number {
  return Math.floor(Date.UTC(date.getFullYear(), date.getMonth(), date.getDate()) / 86_400_000)
}

/**
 * Renewal copy for the Plan panel: an absolute date plus a short relative
 * phrase for a real upcoming renewal, or an honest "doesn't expire" line for
 * the free-plan default branch, where `current_period_end` is null (per the
 * `Subscription` DTO's own doc comment in desktopApi.ts). Never fabricates a
 * date.
 *
 * The absolute date is formatted by hand (day/MONTH_ABBREVIATIONS/year)
 * instead of via `toLocaleDateString`/`Intl`: the abbreviated-month string
 * for a given locale is CLDR data bundled with the JS runtime, and that
 * bundle differs across bun builds — confirmed 2026-09-26 on CI (bun 1.4.2
 * linux/amd64: "Sept") vs a local bun 1.4.2 darwin/arm64 build ("Sep") for
 * the exact same `en-GB` short-month request. A hand-rolled table is
 * deterministic across every runtime/platform/version.
 *
 * The absolute date and the relative phrase both derive from `localDayIndex`
 * so they always describe the same local calendar day — see that function's
 * doc comment for the rounding bug this avoids.
 */
export function planRenewalCopy(currentPeriodEnd: string | null | undefined, now: Date = new Date()): string {
  if (!currentPeriodEnd) return "No renewal — this plan doesn't expire"
  const d = new Date(currentPeriodEnd)
  if (Number.isNaN(d.getTime())) return "No renewal — this plan doesn't expire"
  const abs = `${d.getDate()} ${MONTH_ABBREVIATIONS[d.getMonth()]} ${d.getFullYear()}`
  const days = localDayIndex(d) - localDayIndex(now)
  if (days < 0) return abs
  if (days === 0) return `${abs} · today`
  if (days === 1) return `${abs} · tomorrow`
  if (days < 45) return `${abs} · in ${days} days`
  const months = Math.round(days / 30)
  if (months < 12) return `${abs} · in ${months} month${months === 1 ? '' : 's'}`
  return abs
}
