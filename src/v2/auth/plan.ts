// The plan check both the main window's gate and the b's flyout make. A module of its own, with no imports, so the
// flyout does not load the main window's chrome for it (#274 review).

/** No plan yet: the server's `plan_state` (server #422) says `none`. An older server without it: no plan or no quota. */
export function hasNoPlan(sub: { plan_state?: string | null; plan?: string | null; status?: string | null; quota_bytes?: number | null } | null): boolean {
  if (!sub) return false
  if (sub.plan_state) return sub.plan_state === 'none'
  const plan = (sub.plan ?? '').toLowerCase()
  return plan === '' || plan === 'none' || (sub.status ?? '').toLowerCase() === 'none' || (sub.quota_bytes ?? 1) === 0
}
