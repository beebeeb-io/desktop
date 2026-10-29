/**
 * How long ago a moment was: the arithmetic, once (PR #140 review, P3). The Mac Security rows ("Active 2 hours ago", as the
 * hi-fi draws them) and the Windows Devices view ("2h ago") said it with two copies of the same buckets; the words stay
 * each caller's own, because the design draws the Mac's spelled out and the Windows copy is left alone (spec §10.2 #25).
 * No imports: both sides load it.
 */
export type Age = { unit: 'now' } | { unit: 'minute' | 'hour' | 'day'; count: number } | { unit: 'date'; date: Date }

/**
 * Under 45 seconds is "now" (a clock that is ahead of ours too); then whole minutes up to an hour, whole hours up to a day,
 * whole days until `dateAfterDays`, and the date itself after that. Each step rounds, as the Windows view always has.
 */
export function ageOf(then: Date, now: number, dateAfterDays: number): Age {
  const seconds = Math.round((now - then.getTime()) / 1000)
  if (seconds < 45) return { unit: 'now' }
  const minutes = Math.round(seconds / 60)
  if (minutes < 60) return { unit: 'minute', count: minutes }
  const hours = Math.round(minutes / 60)
  if (hours < 24) return { unit: 'hour', count: hours }
  const days = Math.round(hours / 24)
  if (days < dateAfterDays) return { unit: 'day', count: days }
  return { unit: 'date', date: then }
}
