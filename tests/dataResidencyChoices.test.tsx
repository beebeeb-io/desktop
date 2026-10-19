/**
 * Task 1919, by COUNTRY since task 2050: the Data residency view renders the server's
 * storage-location list (`GET /me/region` -> `choices`): "Europe" is the default (any European
 * data center), the live countries are selectable, a coming-soon country is listed but never
 * selectable. No city is ever shown. The country fixture follows the server contract (server
 * PR #251, aead1d01; not a live capture, the server is not deployed); the older-server fixture is
 * a response captured from a real local API before 2050. Both are in src-tauri/tests/fixtures, so
 * these tests read the same bytes the Rust decoder tests decode.
 */
import { describe, expect, test } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import captured from '../src-tauri/tests/fixtures/region-choices-country-api.json'
import older from '../src-tauri/tests/fixtures/region-choices-older-server-api.json'
import { DataResidencyContent } from '../src/windows/views/SettingsView'
import {
  buildDataResidencyViewState,
  commitPreferredRegionSelection,
  commitStrictSelection,
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
  return { state, view: buildDataResidencyViewState({ ...state, strict: state.regionStrict, saving }) }
}

function render(v: Awaited<ReturnType<typeof view>>, onSelect: (id: string) => void = noop) {
  return renderToStaticMarkup(
    <DataResidencyContent loading={false} loadError={v.state.error} view={v.view} onRetry={noop} onSelect={onSelect} onStrictChange={noop} />,
  )
}

describe('the list comes from the API response', () => {
  test('one row per choice, in the server order, with the server names', async () => {
    const { view: v } = await view(null)
    expect(v.items.map(i => i.id)).toEqual(['europe', 'de', 'fi', 'nl'])
    expect(v.items.map(i => i.displayName)).toEqual(['Europe', 'Germany', 'Finland', 'Netherlands'])
    // Only the Europe row explains itself; a country is its own name, with no city line.
    expect(v.items.map(i => i.detail)).toEqual(['Any of our European data centers. We place your files.', '', '', ''])
  })

  test('changing the API data changes the rows with no client change', async () => {
    const changed = (captured.choices as StorageChoice[]).map(c =>
      c.id === 'nl' ? { ...c, status: 'live', selectable: true } : c.id === 'fi' ? { ...c, name: 'Suomi', status: 'coming_soon', selectable: false } : c)
    const { view: v } = await view(null, false, { choices: changed })
    expect(v.items.find(i => i.id === 'nl')).toMatchObject({ comingSoon: false, disabled: false })
    expect(v.items.find(i => i.id === 'fi')).toMatchObject({ displayName: 'Suomi', comingSoon: true, disabled: true })
  })

  test('an extra country the client has never heard of is shown and selectable', async () => {
    const extra: StorageChoice = {
      id: 'se', kind: 'country', name: 'Sweden', country: 'Sweden', country_code: 'SE',
      status: 'live', selectable: true, is_default: false,
    }
    const { view: v } = await view(null, false, { choices: [...(captured.choices as StorageChoice[]), extra] })
    expect(v.items.at(-1)).toMatchObject({ id: 'se', displayName: 'Sweden', detail: '', disabled: false })
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

  test('a stored country is the selected row', async () => {
    const { view: v } = await view('fi')
    expect(v.items.filter(i => i.selected).map(i => i.id)).toEqual(['fi'])
  })

  test('the Netherlands is listed as coming soon and cannot be chosen', async () => {
    const { view: v } = await view(null)
    const nl = v.items.find(i => i.id === 'nl')!
    expect(nl).toMatchObject({ comingSoon: true, disabled: true, selected: false })
    const html = render(await view(null))
    expect(html).toContain('Coming soon')
    // The Netherlands button is disabled; the live ones are not.
    const nlButton = html.match(/<button[^>]*>(?:(?!<\/button>).)*Netherlands(?:(?!<\/button>).)*<\/button>/s)![0]
    expect(nlButton).toContain('disabled=""')
    const finlandButton = html.match(/<button[^>]*>(?:(?!<\/button>).)*Finland(?:(?!<\/button>).)*<\/button>/s)![0]
    expect(finlandButton).not.toContain('disabled=""')
  })

  test('saving locks every row', async () => {
    const { view: v } = await view(null, true)
    expect(v.items.every(i => i.disabled)).toBe(true)
  })

  test('only Europe selectable (no country live) leaves the list disabled, with no promise of a choice', async () => {
    const onlyEurope = (captured.choices as StorageChoice[]).filter(c => c.id === 'europe')
    const v = await view(null, false, { choices: onlyEurope })
    expect(v.view.onlyOneRegion).toBe(true)
    expect(render(v)).not.toContain('Existing files stay')
  })

  test('selecting sends the id verbatim, rolls back on a refusal and keeps the server value on success', async () => {
    const seen: Array<string | null> = []
    const sent: string[] = []
    const failed = await commitPreferredRegionSelection({
      id: 'nl', preferredRegion: null, saving: false, setPreferredRegion: n => seen.push(n),
      savePreferredRegion: async id => { sent.push(id); throw new Error("400 region_unavailable: 'nl' is not available") },
    })
    expect(sent).toEqual(['nl'])
    expect(seen).toEqual(['nl', null])
    expect(failed).toMatchObject({ status: 'error', previousPreferredRegion: null })

    seen.length = 0
    const saved = await commitPreferredRegionSelection({
      id: 'fi', preferredRegion: null, saving: false, setPreferredRegion: n => seen.push(n),
      savePreferredRegion: async id => ({ preferred_region: id }),
    })
    expect(seen).toEqual(['fi', 'fi'])
    expect(saved).toMatchObject({ status: 'saved', preferredRegion: 'fi' })
  })
})

describe('copy', () => {
  test('Europe default: EU residency, any European data center, no place claim', async () => {
    const html = render(await view(null))
    expect(html).toContain('Stored in the EU. New files go to any of our European data centers.')
    expect(html).toContain('Any of our European data centers. We place your files.')
    expect(html).toContain('Default')
  })

  test('a chosen country names the country from the payload', async () => {
    const html = render(await view('fi'))
    expect(html).toContain('Stored in the EU. New files go to Finland.')
  })

  test('existing files stay where they are', async () => {
    expect(render(await view(null))).toContain('Existing files stay where they are.')
  })

  test('no provider, no flag, no static-list promise', async () => {
    const html = render(await view('de'))
    expect(html).not.toMatch(/hetzner|provider|More regions|Choose where/i)
    expect(html).not.toMatch(/\p{Regional_Indicator}/u)
  })

  test('saved toast names the chosen country, never a provider', async () => {
    const { view: v } = await view('fi')
    expect(savedRegionMessage(v.items, 'fi')).toBe('New files will be stored in Finland.')
    expect(savedRegionMessage(v.items, 'europe')).toBe('New files will go to any of our European data centers.')
    expect(savedRegionMessage(v.items, null)).toBe('New files will go to any of our European data centers.')
  })

  test('a stored country that is no longer in the list is said plainly, not as an error', async () => {
    const without = (captured.choices as StorageChoice[]).filter(c => c.id !== 'fi')
    const v = await view('fi', false, { choices: without })
    const html = render(v)
    expect(html).toContain('Your chosen location isn’t available right now.')
    expect(html).not.toContain('Could not load data residency')
    expect(v.view.items.some(i => i.selected)).toBe(false)
  })

  // PR #128 review: a stored country still in the list but no longer selectable is the same
  // "unavailable" state, not a selected row with a summary promising new files go there.
  test('a stored country still listed but no longer selectable is unavailable, not selected', async () => {
    const retired = (captured.choices as StorageChoice[]).map(c =>
      c.id === 'fi' ? { ...c, status: 'coming_soon', selectable: false } : c)
    const v = await view('fi', false, { choices: retired })
    expect(v.view.preferenceUnavailable).toBe(true)
    expect(v.view.items.some(i => i.selected)).toBe(false)
    expect(residencySummary(v.view)).toBeNull()
    const html = render(v)
    expect(html).toContain('Your chosen location isn’t available right now.')
    expect(html).not.toContain('New files go to Finland')
  })

  // PR #128 review: with Europe the only selectable row and a stale stored country, Europe must
  // stay clickable so the user can replace the stale preference with the available default.
  test('a stale preference with Europe as the only selectable row leaves Europe choosable', async () => {
    const onlyEurope = (captured.choices as StorageChoice[]).filter(c => c.id === 'europe')
    const v = await view('fi', false, { choices: onlyEurope })
    expect(v.view.onlyOneRegion).toBe(true)
    expect(v.view.preferenceUnavailable).toBe(true)
    expect(v.view.items[0]).toMatchObject({ id: 'europe', selected: false, disabled: false })
  })
})

describe('only store in <country> (task 1952, by country since task 2050)', () => {
  test('is offered only while a country is selected, named after it', async () => {
    expect(render(await view(null))).not.toContain('Only store in')
    expect((await view(null)).view.pinned).toBeNull()
    const html = render(await view('de'))
    expect(html).toContain('Only store in Germany')
    expect(html).toContain('role="switch"')
    expect(html).toContain('aria-checked="false"')
    expect(html).toContain('If Germany can’t take a new file, it goes to another EU country.')
  })

  test('on: the saved value is shown and the hint says the upload stops', async () => {
    const html = render(await view('de', false, { region_strict: true }))
    expect(html).toContain('aria-checked="true"')
    expect(html).toContain('If Germany can’t take a new file, the upload stops instead of going to another EU country.')
  })

  test('a stored strict with Europe selected is not offered and not on', async () => {
    const v = await view(null, false, { region_strict: true })
    expect(v.view.strict).toBe(false)
    expect(render(v)).not.toContain('Only store in')
  })

  test('the switch re-sends the pinned country with the new value and goes back on a refusal', async () => {
    const strict: boolean[] = []
    const sent: Array<[string, boolean]> = []
    const saved = await commitStrictSelection({
      pinnedId: 'de', next: true, strict: false, saving: false, setStrict: n => strict.push(n),
      saveRegion: async (id, next) => { sent.push([id, next]); return { preferred_region: id, region_strict: next } },
    })
    expect(sent).toEqual([['de', true]])
    expect(saved).toEqual({ status: 'saved', strict: true })

    strict.length = 0
    const failed = await commitStrictSelection({
      pinnedId: 'de', next: true, strict: false, saving: false, setStrict: n => strict.push(n),
      saveRegion: async () => { throw new Error('400 region_unavailable') },
    })
    expect(strict).toEqual([true, false])
    expect(failed).toMatchObject({ status: 'error', previousStrict: false, message: '400 region_unavailable' })
  })

  test('nothing is sent with no country, no change, or a save in flight', async () => {
    const never = async () => { throw new Error('must not be sent') }
    for (const over of [{ pinnedId: null }, { next: false }, { saving: true }]) {
      const result = await commitStrictSelection({
        pinnedId: 'de', next: true, strict: false, saving: false, setStrict: () => {}, saveRegion: never, ...over,
      })
      expect(result).toEqual({ status: 'noop' })
    }
  })
})

describe('an older server (one row per datacenter, each with a city)', () => {
  const olderIpc = (preferred: string | null, over: Partial<UserRegionResponse> = {}): UserRegionResponse => ({
    preferred_region: preferred, regions: [], choices: older.choices as StorageChoice[], ...over,
  })
  const olderView = async (preferred: string | null) => {
    const state = await loadDataResidency(ok(olderIpc(preferred)))
    return { state, view: buildDataResidencyViewState({ ...state, saving: false }) }
  }

  test('datacenters are folded into one row per country, named by the country, never a city', async () => {
    const { view: v } = await olderView(null)
    expect(v.items.map(i => i.displayName)).toEqual(['Europe', 'Germany', 'Finland', 'Netherlands'])
    // Germany has two datacenters; the row is sent by the first live one's id.
    expect(v.items.map(i => i.id)).toEqual(['europe', 'fsn1', 'hel1', 'ede'])
    expect(v.items.find(i => i.id === 'ede')).toMatchObject({ comingSoon: true, disabled: true })
    const html = render({ state: (await olderView(null)).state, view: v })
    expect(html).not.toMatch(/Falkenstein|Helsinki|Nuremberg|\bEde\b/)
  })

  test('a stored datacenter id selects its country row, whichever datacenter it was', async () => {
    for (const stored of ['fsn1', 'nbg1']) {
      const { view: v } = await olderView(stored)
      expect(v.items.filter(i => i.selected).map(i => i.displayName)).toEqual(['Germany'])
      expect(v.selectedId).toBe('fsn1')
      expect(residencySummary(v)).toBe('Stored in the EU. New files go to Germany.')
    }
  })

  test('a datacenter with no country is dropped, not shown by its city', async () => {
    const noCountry = (older.choices as StorageChoice[]).map(c => c.id === 'hel1' ? { ...c, country: null, country_code: null } : c)
    const state = await loadDataResidency(ok(olderIpc(null, { choices: noCountry })))
    const html = render({ state, view: buildDataResidencyViewState({ ...state, saving: false }) })
    expect(state.choices.map(c => c.id)).toEqual(['europe', 'fsn1', 'ede'])
    expect(html).not.toContain('Helsinki')
  })

  test('a country is live as soon as one of its datacenters is', async () => {
    const mixed = (older.choices as StorageChoice[]).map(c =>
      c.id === 'fsn1' ? { ...c, status: 'coming_soon', selectable: false } : c)
    const state = await loadDataResidency(ok(olderIpc(null, { choices: mixed })))
    expect(state.choices.find(c => c.name === 'Germany')).toMatchObject({ id: 'nbg1', selectable: true, status: 'live' })
  })
})

describe('an older server without choices', () => {
  const legacy = { preferred_region: null, regions: [{ continent: 'europe', display_name: 'Europe', country: null, is_default: true }] }

  test('falls back to the legacy continent list from the same response, with no invented place', async () => {
    const state = await loadDataResidency(ok(legacy as UserRegionResponse))
    const v = buildDataResidencyViewState({ ...state, saving: false })
    expect(v.items).toHaveLength(1)
    expect(v.items[0]).toMatchObject({ id: 'europe', displayName: 'Europe', selected: true, disabled: true })
    expect(residencySummary(v)).toBe('Stored in the EU. New files go to any of our European data centers.')
  })
})
