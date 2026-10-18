/**
 * Task 1919: the Data residency view renders the server's storage-location list
 * (`GET /me/region` -> `choices`): "Europe" is the default (any datacenter), the live
 * datacenters are selectable, a coming-soon datacenter is listed but never selectable.
 * The fixture is a response captured from a real local API (src-tauri/tests/fixtures), so
 * these tests read the same bytes the Rust decoder test decodes.
 */
import { describe, expect, test } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import captured from '../src-tauri/tests/fixtures/region-choices-api.json'
import { DataResidencyContent } from '../src/windows/views/SettingsView'
import {
  buildDataResidencyViewState,
  commitPreferredRegionSelection,
  loadDataResidency,
  residencySummary,
  savedRegionMessage,
} from '../src/windows/dataResidencySettingsModel'
import type { CommandResult, StorageChoice, UserRegionResponse } from '../src/desktopApi'

// The Rust DTO maps `available_regions` to the IPC key `regions`; the rest is as captured.
const ipc = (preferred: string | null, over: Partial<UserRegionResponse> = {}): UserRegionResponse => ({
  preferred_region: preferred,
  regions: [],
  choices: captured.choices as StorageChoice[],
  ...over,
})
const ok = (value: UserRegionResponse) => async (): Promise<CommandResult<UserRegionResponse>> => ({ ok: true, value })
const noop = () => {}

async function view(preferred: string | null, saving = false, over: Partial<UserRegionResponse> = {}) {
  const state = await loadDataResidency(ok(ipc(preferred, over)))
  return { state, view: buildDataResidencyViewState({ ...state, saving }) }
}

function render(v: Awaited<ReturnType<typeof view>>, onSelect: (id: string) => void = noop) {
  return renderToStaticMarkup(
    <DataResidencyContent loading={false} loadError={v.state.error} view={v.view} onRetry={noop} onSelect={onSelect} />,
  )
}

describe('the list comes from the API response', () => {
  test('one row per choice, in the server order, with the server names', async () => {
    const { view: v } = await view(null)
    expect(v.items.map(i => i.id)).toEqual(['europe', 'fsn1', 'hel1', 'nbg1', 'ede'])
    expect(v.items.map(i => i.displayName)).toEqual(['Europe', 'Falkenstein', 'Helsinki', 'Nuremberg', 'Ede'])
    // Countries are the payload's, not a client table.
    expect(v.items.map(i => i.detail)).toEqual([
      'Any of our datacenters in Europe', 'Germany', 'Finland', 'Germany', 'Netherlands',
    ])
  })

  test('changing the API data changes the rows with no client change', async () => {
    const changed = (captured.choices as StorageChoice[]).map(c =>
      c.id === 'ede' ? { ...c, status: 'live', selectable: true } : c.id === 'nbg1' ? { ...c, name: 'Nürnberg', status: 'coming_soon', selectable: false } : c)
    const { view: v } = await view(null, false, { choices: changed })
    expect(v.items.find(i => i.id === 'ede')).toMatchObject({ comingSoon: false, disabled: false })
    expect(v.items.find(i => i.id === 'nbg1')).toMatchObject({ displayName: 'Nürnberg', comingSoon: true, disabled: true })
  })

  test('an extra datacenter the client has never heard of is shown and selectable', async () => {
    const extra: StorageChoice = {
      id: 'zzz9', kind: 'datacenter', name: 'Zwolle', city: 'Zwolle', country: 'Netherlands', country_code: 'NL',
      status: 'live', selectable: true, is_default: false,
    }
    const { view: v } = await view(null, false, { choices: [...(captured.choices as StorageChoice[]), extra] })
    expect(v.items.at(-1)).toMatchObject({ id: 'zzz9', displayName: 'Zwolle', detail: 'Netherlands', disabled: false })
  })
})

describe('selection', () => {
  test('no preference selects Europe, the default', async () => {
    const { view: v } = await view(null)
    expect(v.effectiveRegion).toBe('europe')
    expect(v.items.filter(i => i.selected).map(i => i.id)).toEqual(['europe'])
    expect(v.items.find(i => i.id === 'europe')!.isDefault).toBe(true)
    expect(v.items.filter(i => i.isDefault)).toHaveLength(1)
  })

  test('a stored datacenter is the selected row', async () => {
    const { view: v } = await view('hel1')
    expect(v.items.filter(i => i.selected).map(i => i.id)).toEqual(['hel1'])
  })

  test('Ede is listed as coming soon and cannot be chosen', async () => {
    const { view: v } = await view(null)
    const ede = v.items.find(i => i.id === 'ede')!
    expect(ede).toMatchObject({ comingSoon: true, disabled: true, selected: false })
    const html = render(await view(null))
    expect(html).toContain('Coming soon')
    // The Ede button is disabled; the live ones are not.
    const edeButton = html.match(/<button[^>]*>(?:(?!<\/button>).)*Ede(?:(?!<\/button>).)*<\/button>/s)![0]
    expect(edeButton).toContain('disabled=""')
    const helsinkiButton = html.match(/<button[^>]*>(?:(?!<\/button>).)*Helsinki(?:(?!<\/button>).)*<\/button>/s)![0]
    expect(helsinkiButton).not.toContain('disabled=""')
  })

  test('saving locks every row', async () => {
    const { view: v } = await view(null, true)
    expect(v.items.every(i => i.disabled)).toBe(true)
  })

  test('only Europe selectable (no datacenter live) leaves the list disabled, with no promise of a choice', async () => {
    const onlyEurope = (captured.choices as StorageChoice[]).filter(c => c.id === 'europe')
    const v = await view(null, false, { choices: onlyEurope })
    expect(v.view.onlyOneRegion).toBe(true)
    expect(render(v)).not.toContain('Existing files stay')
  })

  test('selecting sends the id verbatim, rolls back on a refusal and keeps the server value on success', async () => {
    const seen: Array<string | null> = []
    const sent: string[] = []
    const failed = await commitPreferredRegionSelection({
      id: 'ede', preferredRegion: null, saving: false, setPreferredRegion: n => seen.push(n),
      savePreferredRegion: async id => { sent.push(id); throw new Error("400 region_unavailable: 'ede' is not available") },
    })
    expect(sent).toEqual(['ede'])
    expect(seen).toEqual(['ede', null])
    expect(failed).toMatchObject({ status: 'error', previousPreferredRegion: null })

    seen.length = 0
    const saved = await commitPreferredRegionSelection({
      id: 'hel1', preferredRegion: null, saving: false, setPreferredRegion: n => seen.push(n),
      savePreferredRegion: async id => ({ preferred_region: id }),
    })
    expect(seen).toEqual(['hel1', 'hel1'])
    expect(saved).toMatchObject({ status: 'saved', preferredRegion: 'hel1' })
  })
})

describe('copy', () => {
  test('Europe default: EU residency, any datacenter, no city claim', async () => {
    const html = render(await view(null))
    expect(html).toContain('Stored in the EU. New files go to any of our datacenters in Europe.')
    expect(html).toContain('Default')
  })

  test('a chosen datacenter names its city and country from the payload', async () => {
    const html = render(await view('hel1'))
    expect(html).toContain('Stored in the EU. New files go to Helsinki, Finland.')
  })

  test('existing files stay where they are', async () => {
    expect(render(await view(null))).toContain('Existing files stay where they are.')
  })

  test('no provider, no flag, no static-list promise', async () => {
    const html = render(await view('nbg1'))
    expect(html).not.toMatch(/hetzner|provider|More regions|Choose where/i)
    expect(html).not.toMatch(/\p{Regional_Indicator}/u)
  })

  test('saved toast names the chosen location, never a provider', async () => {
    const { view: v } = await view('hel1')
    expect(savedRegionMessage(v.items, 'hel1')).toBe('New files will be stored in Helsinki.')
    expect(savedRegionMessage(v.items, 'europe')).toBe('New files will go to any of our datacenters in Europe.')
    expect(savedRegionMessage(v.items, null)).toBe('New files will go to any of our datacenters in Europe.')
  })

  test('a stored datacenter that is no longer in the list is said plainly, not as an error', async () => {
    const without = (captured.choices as StorageChoice[]).filter(c => c.id !== 'hel1')
    const v = await view('hel1', false, { choices: without })
    const html = render(v)
    expect(html).toContain('Your chosen location isn’t available right now.')
    expect(html).not.toContain('Could not load data residency')
    expect(v.view.items.some(i => i.selected)).toBe(false)
  })

  // PR #128 review: a stored datacenter still in the list but no longer selectable is the same
  // "unavailable" state, not a selected row with a summary promising new files go there.
  test('a stored datacenter still listed but no longer selectable is unavailable, not selected', async () => {
    const retired = (captured.choices as StorageChoice[]).map(c =>
      c.id === 'hel1' ? { ...c, status: 'coming_soon', selectable: false } : c)
    const v = await view('hel1', false, { choices: retired })
    expect(v.view.preferenceUnavailable).toBe(true)
    expect(v.view.items.some(i => i.selected)).toBe(false)
    expect(residencySummary(v.view)).toBeNull()
    const html = render(v)
    expect(html).toContain('Your chosen location isn’t available right now.')
    expect(html).not.toContain('Helsinki, Finland')
  })

  // PR #128 review: with Europe the only selectable row and a stale stored datacenter, Europe must
  // stay clickable so the user can replace the stale preference with the available default.
  test('a stale preference with Europe as the only selectable row leaves Europe choosable', async () => {
    const onlyEurope = (captured.choices as StorageChoice[]).filter(c => c.id === 'europe')
    const v = await view('hel1', false, { choices: onlyEurope })
    expect(v.view.onlyOneRegion).toBe(true)
    expect(v.view.preferenceUnavailable).toBe(true)
    expect(v.view.items[0]).toMatchObject({ id: 'europe', selected: false, disabled: false })
  })
})

describe('an older server without choices', () => {
  const legacy = { preferred_region: null, regions: [{ continent: 'europe', display_name: 'Europe', city: 'Falkenstein', is_default: true }] }

  test('falls back to the legacy continent list from the same response, with no invented country', async () => {
    const state = await loadDataResidency(ok(legacy as UserRegionResponse))
    const v = buildDataResidencyViewState({ ...state, saving: false })
    expect(v.items).toHaveLength(1)
    expect(v.items[0]).toMatchObject({ id: 'europe', displayName: 'Europe', detail: 'Falkenstein', selected: true, disabled: true })
  })
})
