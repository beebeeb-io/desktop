import type { CommandResult, RegionInfo, SetPreferredRegionResponse, UserRegionResponse } from '../desktopApi'

export interface DataResidencyLoadState {
  regions: RegionInfo[]
  preferredRegion: string | null
  error: string | null
}

/** No invented regions on errors or empty responses; Retry calls the API again. */
export async function loadDataResidency(
  fetchRegion: () => Promise<CommandResult<UserRegionResponse>>,
): Promise<DataResidencyLoadState> {
  try {
    const result = await fetchRegion()
    if (result.ok && result.value.regions.length > 0) {
      return { regions: result.value.regions, preferredRegion: result.value.preferred_region, error: null }
    }
    if (result.ok) {
      return { regions: [], preferredRegion: null, error: 'No storage regions are currently available. Please try again.' }
    }
  } catch {
    // Keep unexpected invocation failures in the same recoverable error state.
  }
  return { regions: [], preferredRegion: null, error: 'Data residency is unavailable. Please try again.' }
}

export interface DataResidencyRegionItem {
  continent: string
  displayName: string
  locationLabel: string
  isDefault: boolean
  selected: boolean
  disabled: boolean
}

export interface DataResidencyViewState {
  effectiveRegion: string | null
  onlyOneRegion: boolean
  items: DataResidencyRegionItem[]
}

export type CommitPreferredRegionResult =
  | {
      status: 'noop'
      preferredRegion: string | null
    }
  | {
      status: 'saved'
      previousPreferredRegion: string | null
      preferredRegion: string | null
    }
  | {
      status: 'error'
      previousPreferredRegion: string | null
      message: string
    }

export function countryFromCity(city: string): string {
  const map: Record<string, string> = {
    falkenstein: 'Germany',
    helsinki: 'Finland',
    ede: 'Netherlands',
    ashburn: 'United States',
  }
  return map[city.toLowerCase()] ?? ''
}

export function regionLocationLabel(region: RegionInfo): string {
  const country = countryFromCity(region.city)
  return country ? `${region.city}, ${country}` : region.city
}

export function buildDataResidencyViewState({
  preferredRegion,
  regions,
  saving,
}: {
  preferredRegion: string | null
  regions: RegionInfo[]
  saving: boolean
}): DataResidencyViewState {
  const effectiveRegion = preferredRegion ?? regions.find(region => region.is_default)?.continent ?? null
  const onlyOneRegion = regions.length <= 1

  return {
    effectiveRegion,
    onlyOneRegion,
    items: regions.map(region => ({
      continent: region.continent,
      displayName: region.display_name,
      locationLabel: regionLocationLabel(region),
      isDefault: region.is_default,
      selected: effectiveRegion === region.continent,
      disabled: saving || onlyOneRegion,
    })),
  }
}

export async function commitPreferredRegionSelection({
  continent,
  preferredRegion,
  saving,
  setPreferredRegion,
  savePreferredRegion,
}: {
  continent: string
  preferredRegion: string | null
  saving: boolean
  setPreferredRegion: (preferredRegion: string | null) => void
  savePreferredRegion: (preferredRegion: string) => Promise<SetPreferredRegionResponse>
}): Promise<CommitPreferredRegionResult> {
  if (saving || continent === preferredRegion) {
    return { status: 'noop', preferredRegion }
  }

  const previousPreferredRegion = preferredRegion
  setPreferredRegion(continent)

  try {
    const result = await savePreferredRegion(continent)
    setPreferredRegion(result.preferred_region)
    return {
      status: 'saved',
      previousPreferredRegion,
      preferredRegion: result.preferred_region,
    }
  } catch (error) {
    setPreferredRegion(previousPreferredRegion)
    return {
      status: 'error',
      previousPreferredRegion,
      message: error instanceof Error ? error.message : String(error),
    }
  }
}
