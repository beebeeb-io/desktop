/**
 * useRegion — resolve the user's storage CITY from the live `/me/region`
 * setting, the same source of truth the webapp uses.
 *
 * Brand rule: we surface the CITY only ("Falkenstein"), never the storage
 * provider. Both hooks start with the live default ("Falkenstein") so a label
 * is never blank, then update once `fetchRegion()` (a module-cached single
 * round-trip) resolves. On error the response is null and the fallback stands.
 */

import { useEffect, useState } from 'react'
import { fetchRegion, regionCity, regionLabel } from '../desktopApi'

/** The effective region's city, e.g. "Falkenstein". */
export function useRegionCity(): string {
  const [city, setCity] = useState<string>(() => regionCity(null))
  useEffect(() => {
    let cancelled = false
    void fetchRegion().then(resp => {
      if (!cancelled) setCity(regionCity(resp))
    })
    return () => {
      cancelled = true
    }
  }, [])
  return city
}

/** "City, Country" for the effective region, e.g. "Falkenstein, Germany". */
export function useRegionLabel(): string {
  const [label, setLabel] = useState<string>(() => regionLabel(null))
  useEffect(() => {
    let cancelled = false
    void fetchRegion().then(resp => {
      if (!cancelled) setLabel(regionLabel(resp))
    })
    return () => {
      cancelled = true
    }
  }, [])
  return label
}
