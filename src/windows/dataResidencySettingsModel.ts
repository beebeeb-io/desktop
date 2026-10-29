import type {
  CommandResult,
  RegionInfo,
  SetPreferredRegionResponse,
  StorageChoice,
  UserRegionResponse,
} from '../desktopApi'

export interface DataResidencyLoadState {
  /** The rows to show, as the API sent them (see `choicesFromResponse`). */
  choices: StorageChoice[]
  preferredRegion: string | null
  /** "Only store in <country>" as saved. */
  regionStrict: boolean
  error: string | null
}

/**
 * Task 2050 (Guus, 2026-10-19): the person sees a COUNTRY, never a city. A server that speaks
 * countries sends `kind: 'country'` rows and this is the identity. An OLDER server (task 1919) sends
 * one `kind: 'datacenter'` row per datacenter, each named after its city: those are folded into one
 * row per country (by country code), named by the country. The folded row is sent by the first
 * selectable datacenter's id (the id that older server accepts on PUT) and keeps every datacenter id
 * as an alias, so a stored `fsn1` still selects the Germany row. A datacenter with no country cannot
 * be shown without naming its city, so it is dropped. Nothing here reads a city.
 */
export function foldChoices(choices: StorageChoice[]): StorageChoice[] {
  const out: StorageChoice[] = []
  const byCountry = new Map<string, StorageChoice>()
  for (const choice of choices) {
    if (choice.kind !== 'datacenter') {
      out.push(choice)
      continue
    }
    const country = choice.country?.trim()
    if (!country) continue
    const key = (choice.country_code ?? country).toLowerCase()
    const existing = byCountry.get(key)
    if (!existing) {
      const row: StorageChoice = {
        id: choice.id,
        kind: 'country',
        name: country,
        country,
        country_code: choice.country_code,
        status: choice.status,
        selectable: choice.selectable,
        is_default: false,
        aliases: [choice.id.toLowerCase()],
      }
      byCountry.set(key, row)
      out.push(row)
      continue
    }
    existing.aliases = [...(existing.aliases ?? []), choice.id.toLowerCase()]
    // A country is live as soon as one of its datacenters is, and is sent by that one's id.
    if (choice.selectable && !existing.selectable) {
      existing.id = choice.id
      existing.status = 'live'
      existing.selectable = true
    }
  }
  return out
}

/**
 * The list the view renders. The server sends `choices` (task 1919, countries since task 2050); a
 * server that predates it sends only the one-entry-per-continent list, shown as it is (the name
 * from the response, no place line). Nothing here is a client-side list.
 */
export function choicesFromResponse(response: Pick<UserRegionResponse, 'regions' | 'choices'>): StorageChoice[] {
  if (response.choices && response.choices.length > 0) return foldChoices(response.choices)
  return response.regions.map((region: RegionInfo) => ({
    id: region.continent,
    kind: 'region',
    name: region.display_name,
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
      if (choices.length > 0) {
        return {
          choices,
          preferredRegion: result.value.preferred_region,
          regionStrict: result.value.region_strict === true,
          error: null,
        }
      }
      return {
        choices: [],
        preferredRegion: null,
        regionStrict: false,
        error: 'No storage regions are currently available. Please try again.',
      }
    }
  } catch {
    // Keep unexpected invocation failures in the same recoverable error state.
  }
  return { choices: [], preferredRegion: null, regionStrict: false, error: 'Data residency is unavailable. Please try again.' }
}

export interface DataResidencyRegionItem {
  id: string
  kind: string
  displayName: string
  /** The second line of the row: what "Europe" means. Empty for a country (its name says it). */
  detail: string
  /** "Germany" / "any of our European data centers": the place, for the sentence above the list. */
  locationLabel: string
  isDefault: boolean
  selected: boolean
  comingSoon: boolean
  disabled: boolean
}

/** The country the account is pinned to, which "Only store in <country>" is a promise about. */
export interface DataResidencyPinned {
  id: string
  displayName: string
  /** "Only store in Germany". */
  strictLabel: string
  /** What the switch means in the state it is in. */
  strictHint: string
}

export interface DataResidencyViewState {
  effectiveRegion: string | null
  /** The id of the selected row (an older server's stored datacenter id selects its country's row). */
  selectedId: string | null
  onlyOneRegion: boolean
  /** A stored preference that no selectable row matches (its country left the list or can no longer be chosen): said plainly, not an error. */
  preferenceUnavailable: boolean
  /** Set only when a country is selected; Europe has no strict. */
  pinned: DataResidencyPinned | null
  /** "Only store in <country>" as saved. False whenever no country is selected. */
  strict: boolean
  /** A save is in flight: every control waits. */
  saving: boolean
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

/** What a choice means in words. Built only from the payload's own fields; a country is its own name. */
export function choiceDetail(choice: StorageChoice): string {
  return choice.kind === 'region' && choice.id === 'europe' ? 'Any of our European data centers. We place your files.' : ''
}

export function choiceLocationLabel(choice: StorageChoice): string {
  if (choice.kind === 'region' && choice.id === 'europe') return 'any of our European data centers'
  return choice.country ?? choice.name
}

/** The stored value (any case) means this row: its id, or a datacenter id an older server stored for its country. */
function matches(choice: StorageChoice, wanted: string): boolean {
  return choice.id.toLowerCase() === wanted || (choice.aliases ?? []).includes(wanted)
}

export function strictLabel(displayName: string): string {
  return `Only store in ${displayName}`
}

export function buildDataResidencyViewState({
  preferredRegion,
  choices,
  strict = false,
  saving,
}: {
  preferredRegion: string | null
  choices: StorageChoice[]
  strict?: boolean
  saving: boolean
}): DataResidencyViewState {
  const defaultId = choices.find(choice => choice.is_default && choice.selectable)?.id ?? null
  const effectiveRegion = preferredRegion ?? defaultId
  const onlyOneRegion = choices.filter(choice => choice.selectable).length <= 1
  // A stored row that is still listed but no longer selectable (a retired or disabled country)
  // is as unavailable as one that left the list: it is not selected and the summary does not
  // promise new files go there.
  const wanted = (effectiveRegion ?? '').toLowerCase()
  const matched = wanted ? choices.find(choice => choice.selectable && matches(choice, wanted)) : undefined
  const matchedId = matched?.id ?? null
  const pinnedChoice = matched?.kind === 'country' ? matched : null
  const pinned: DataResidencyPinned | null = pinnedChoice
    ? {
        id: pinnedChoice.id,
        displayName: pinnedChoice.name,
        strictLabel: strictLabel(pinnedChoice.name),
        strictHint: strict
          ? `If ${pinnedChoice.name} can’t take a new file, the upload stops instead of going to another EU country.`
          : `If ${pinnedChoice.name} can’t take a new file, it goes to another EU country.`,
      }
    : null

  return {
    effectiveRegion,
    selectedId: matchedId,
    onlyOneRegion,
    preferenceUnavailable: preferredRegion !== null && matchedId === null && defaultId !== null,
    pinned,
    strict: pinned !== null && strict,
    saving,
    items: choices.map(choice => {
      const selected = matchedId === choice.id
      return {
        id: choice.id,
        kind: choice.kind,
        displayName: choice.name,
        detail: choiceDetail(choice),
        locationLabel: choiceLocationLabel(choice),
        isDefault: choice.is_default,
        selected,
        comingSoon: !choice.selectable && choice.status === 'coming_soon',
        // The sole selectable row is locked only when it is already the selection; with a stale
        // preference it must stay clickable so the user can move back to it.
        disabled: saving || !choice.selectable || (onlyOneRegion && selected),
      }
    }),
  }
}

/** The sentence under the title. The "EU" claim follows the row the account is on. */
export function residencySummary(view: DataResidencyViewState): string | null {
  const current = view.items.find(item => item.selected)
  if (!current?.displayName.trim()) return null
  // A country or the Europe row is in the EU; a legacy continent row is said as its name.
  if (current.kind === 'country' || current.id === 'europe') {
    return `Stored in the EU. New files go to ${current.locationLabel}.`
  }
  return `Stored in ${current.displayName}.`
}

/** The toast after a saved choice, from the rows the view is showing. */
export function savedRegionMessage(items: DataResidencyRegionItem[], preferredRegion: string | null): string {
  const chosen = items.find(item => item.id === (preferredRegion ?? items.find(i => i.isDefault)?.id))
  if (chosen?.kind === 'country') return `New files will be stored in ${chosen.displayName}.`
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

export type CommitStrictResult =
  | { status: 'noop' }
  | { status: 'saved'; strict: boolean }
  | { status: 'error'; previousStrict: boolean; message: string }

/**
 * "Only store in <country>": the switch moves at once and goes back if the server refuses. It
 * re-sends the country the account is on (the PUT replaces the whole choice), never another one.
 */
export async function commitStrictSelection({
  pinnedId,
  next,
  strict,
  saving,
  setStrict,
  saveRegion,
}: {
  pinnedId: string | null
  next: boolean
  strict: boolean
  saving: boolean
  setStrict: (strict: boolean) => void
  saveRegion: (id: string, strict: boolean) => Promise<SetPreferredRegionResponse>
}): Promise<CommitStrictResult> {
  if (saving || pinnedId === null || next === strict) return { status: 'noop' }
  setStrict(next)
  try {
    const result = await saveRegion(pinnedId, next)
    const saved = result.region_strict ?? next
    setStrict(saved)
    return { status: 'saved', strict: saved }
  } catch (error) {
    setStrict(strict)
    return { status: 'error', previousStrict: strict, message: error instanceof Error ? error.message : String(error) }
  }
}
