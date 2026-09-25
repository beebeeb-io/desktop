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

/**
 * Renewal copy for the Plan panel: an absolute date plus a short relative
 * phrase for a real upcoming renewal, or an honest "doesn't expire" line for
 * the free-plan default branch, where `current_period_end` is null (per the
 * `Subscription` DTO's own doc comment in desktopApi.ts). Never fabricates a
 * date.
 */
export function planRenewalCopy(currentPeriodEnd: string | null | undefined, now: Date = new Date()): string {
  if (!currentPeriodEnd) return "No renewal — this plan doesn't expire"
  const d = new Date(currentPeriodEnd)
  if (Number.isNaN(d.getTime())) return "No renewal — this plan doesn't expire"
  const abs = d.toLocaleDateString('en-GB', { day: 'numeric', month: 'short', year: 'numeric' })
  const days = Math.round((d.getTime() - now.getTime()) / 86_400_000)
  if (days < 0) return abs
  if (days === 0) return `${abs} · today`
  if (days === 1) return `${abs} · tomorrow`
  if (days < 45) return `${abs} · in ${days} days`
  const months = Math.round(days / 30)
  if (months < 12) return `${abs} · in ${months} month${months === 1 ? '' : 's'}`
  return abs
}
