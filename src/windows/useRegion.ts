/**
 * Task 1663 review ruling (Guus, 2026-09-30): product copy follows the
 * account's effective continent; unknown/loading/error makes no location claim.
 * Task 2050 (Guus, 2026-10-19): a country at most, never a city.
 */
import { useEffect, useState } from 'react'
import { fetchRegion, type UserRegionResponse } from '../desktopApi'

export function productRegionLabel(response: UserRegionResponse | null | undefined): string {
  const effective = response?.preferred_region
    ?? response?.regions.find(region => region.is_default)?.continent
  // Task 1919 / 2050: the stored preference can be a live country id (`fi`), or an older server's
  // datacenter id. A country (or datacenter) row in the account's own list is one of our European
  // locations, so it is the EU. One that is not in the list (it left it) or is not selectable makes
  // no location claim.
  const wanted = effective?.toLowerCase()
  const choice = response?.choices?.find(candidate => candidate.id.toLowerCase() === wanted)
  if (choice?.kind === 'country' || choice?.kind === 'datacenter') {
    return choice.selectable ? 'Stored in the EU' : 'End-to-end encrypted'
  }
  const region = response?.regions.find(region => region.continent === effective)
  if (region?.continent === 'europe') return 'Stored in the EU'
  if (region?.continent === 'us') return 'Stored in North America'
  return 'End-to-end encrypted'
}

/** Refresh on onboarding transitions, including successful sign-in. */
export function useRegionLabel(refreshKey?: string): string {
  const [state, setState] = useState<{ key: string | undefined; response: UserRegionResponse | null } | null>(null)
  useEffect(() => {
    let cancelled = false
    void fetchRegion().then(response => {
      if (!cancelled) setState({ key: refreshKey, response })
    })
    return () => { cancelled = true }
  }, [refreshKey])
  return productRegionLabel(state?.key === refreshKey ? state?.response : null)
}
