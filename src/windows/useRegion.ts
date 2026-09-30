/**
 * Task 1663 review ruling (Guus, 2026-09-30): product copy follows the
 * account's effective continent; unknown/loading/error makes no location claim.
 * City metadata belongs only in the separate Data residency transparency view.
 */
import { useEffect, useState } from 'react'
import { fetchRegion, type UserRegionResponse } from '../desktopApi'

export function productRegionLabel(response: UserRegionResponse | null | undefined): string {
  const effective = response?.preferred_region
    ?? response?.regions.find(region => region.is_default)?.continent
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
