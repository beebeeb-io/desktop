import type {
  CommandResult,
  RegionInfo,
  SetPreferredRegionResponse,
  StorageChoice,
  UserRegionResponse,
} from '../desktopApi'

export interface DataResidencyLoadState {
  /** The rows to show, exactly as the API sent them (see `choicesFromResponse`). */
  choices: StorageChoice[]
  preferredRegion: string | null
  error: string | null
}

/**
 * The list the view renders. The server sends `choices` (task 1919); a server that predates it
 * sends only the one-entry-per-continent list, which is shown as it is (name and city from the
 * response, no country because the response has none). Nothing here is a client-side list.
 */
export function choicesFromResponse(response: Pick<UserRegionResponse, 'regions' | 'choices'>): StorageChoice[] {
  if (response.choices && response.choices.length > 0) return response.choices
  return response.regions.map((region: RegionInfo) => ({
    id: region.continent,
    kind: 'region',
    name: region.display_name,
    city: region.city || null,
    country: null,
    country_code: null,
    status: 'live',
    selectable: true,
    is_default: region.is_default,
  }))
}

/** No invented regions on errors or empty responses; Retry calls the API again. */
export async function loadDataResidency(
  fetchRegion: () => Promise<CommandResult<UserRegionResponse>>,
): Promise<DataResidencyLoadState> {
  try {
    const result = await fetchRegion()
    if (result.ok) {
      const choices = choicesFromResponse(result.value)
      if (choices.length > 0) return { choices, preferredRegion: result.value.preferred_region, error: null }
      return { choices: [], preferredRegion: null, error: 'No storage regions are currently available. Please try again.' }
    }
  } catch {
    // Keep unexpected invocation failures in the same recoverable error state.
  }
  return { choices: [], preferredRegion: null, error: 'Data residency is unavailable. Please try again.' }
}

export interface DataResidencyRegionItem {
  id: string
  kind: string
  displayName: string
  /** The second line of the row: the country, or what "Europe" means. From the payload. */
  detail: string
  /** "Helsinki, Finland" / "any of our datacenters in Europe": the place, for the sentence above the list. */
  locationLabel: string
  isDefault: boolean
  /** A region with no city of its own: the server places the data in any of its datacenters. */
  anyDatacenter: boolean
  selected: boolean
  comingSoon: boolean
  disabled: boolean
}

export interface DataResidencyViewState {
  effectiveRegion: string | null
  onlyOneRegion: boolean
  /** A stored preference that no row matches (its datacenter left the list): said plainly, not an error. */
  preferenceUnavailable: boolean
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

/** What a choice means in words. Built only from the payload's own fields. */
export function choiceDetail(choice: StorageChoice): string {
  if (choice.kind === 'datacenter') return choice.country ?? ''
  // A region: the city a legacy response names, or the plain meaning of the default row.
  return choice.city ?? `Any of our datacenters in ${choice.name}`
}

export function choiceLocationLabel(choice: StorageChoice): string {
  if (choice.kind === 'datacenter') {
    return [choice.city ?? choice.name, choice.country].filter(Boolean).join(', ')
  }
  return choice.city ?? `any of our datacenters in ${choice.name}`
}

export function buildDataResidencyViewState({
  preferredRegion,
  choices,
  saving,
}: {
  preferredRegion: string | null
  choices: StorageChoice[]
  saving: boolean
}): DataResidencyViewState {
  const defaultId = choices.find(choice => choice.is_default && choice.selectable)?.id ?? null
  const effectiveRegion = preferredRegion ?? defaultId
  const onlyOneRegion = choices.filter(choice => choice.selectable).length <= 1
  const matched = choices.some(choice => choice.id === effectiveRegion)

  return {
    effectiveRegion,
    onlyOneRegion,
    preferenceUnavailable: preferredRegion !== null && !matched && defaultId !== null,
    items: choices.map(choice => ({
      id: choice.id,
      kind: choice.kind,
      displayName: choice.name,
      detail: choiceDetail(choice),
      locationLabel: choiceLocationLabel(choice),
      isDefault: choice.is_default,
      anyDatacenter: choice.kind !== 'datacenter' && !choice.city,
      selected: effectiveRegion === choice.id,
      comingSoon: !choice.selectable && choice.status === 'coming_soon',
      disabled: saving || onlyOneRegion || !choice.selectable,
    })),
  }
}

/** The sentence under the title. The "EU" claim follows the row the account is on. */
export function residencySummary(view: DataResidencyViewState): string | null {
  const current = view.items.find(item => item.id === view.effectiveRegion)
  if (!current?.displayName.trim()) return null
  const where = current.id === 'europe' ? 'the EU' : current.displayName
  if (current.kind === 'datacenter' || current.anyDatacenter) {
    return `Stored in ${current.kind === 'datacenter' ? 'the EU' : where}. New files go to ${current.locationLabel}.`
  }
  return `Stored in ${where} (currently ${current.locationLabel}).`
}

/** The toast after a saved choice, from the rows the view is showing. */
export function savedRegionMessage(items: DataResidencyRegionItem[], preferredRegion: string | null): string {
  const chosen = items.find(item => item.id === (preferredRegion ?? items.find(i => i.isDefault)?.id))
  if (chosen?.kind === 'datacenter') return `New files will be stored in ${chosen.displayName}.`
  if (chosen) return `New files will go to ${chosen.locationLabel}.`
  return 'Your choice is saved.'
}

export async function commitPreferredRegionSelection({
  id,
  preferredRegion,
  saving,
  setPreferredRegion,
  savePreferredRegion,
}: {
  id: string
  preferredRegion: string | null
  saving: boolean
  setPreferredRegion: (preferredRegion: string | null) => void
  savePreferredRegion: (preferredRegion: string) => Promise<SetPreferredRegionResponse>
}): Promise<CommitPreferredRegionResult> {
  if (saving || id === preferredRegion) {
    return { status: 'noop', preferredRegion }
  }

  const previousPreferredRegion = preferredRegion
  setPreferredRegion(id)

  try {
    const result = await savePreferredRegion(id)
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
