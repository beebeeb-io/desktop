/**
 * The macOS Settings storage-location group (task 1919, by country since task 2050): the real `DataResidencyGroup`
 * component and the real `useDataResidency` hook, executed with the component harness against a scripted backend
 * that answers like `GET /me/region` (the fixture follows the server contract, server PR #251; not a live capture).
 *
 * What this proves: the rows come from the backend's list and follow it when it changes; Europe is the default;
 * a live country sends its id verbatim (with `regionStrict` stated); a coming-soon country cannot be chosen;
 * "Only store in <country>" re-sends the chosen country; one failure is one surface; no city is ever shown.
 * What it does NOT prove: layout, focus, or anything on a Mac (tests/render-mac-settings.mjs and the app run do).
 */
import { afterEach, describe, expect, test } from 'bun:test'
import { createElement, Fragment } from 'react'
import captured from '../src-tauri/tests/fixtures/region-choices-country-api.json'
import * as desktopApi from '../src/desktopApi'
import * as model from '../src/windows/dataResidencySettingsModel'
import * as parts from '../src/macSettingsParts'
import { expand, mount, textOf, visibleErrorSurfaces, type Mounted, type TreeNode } from './fixtures/componentHarness'

const React = { createElement, Fragment }
const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close() })

const hook = {
  file: 'windows/useDataResidency.ts',
  name: 'useDataResidency',
  bindings: {
    accountRegion: desktopApi.accountRegion,
    setAccountRegion: desktopApi.setAccountRegion,
    commandUnavailableLabel: desktopApi.commandUnavailableLabel,
    loadDataResidency: model.loadDataResidency,
    buildDataResidencyViewState: model.buildDataResidencyViewState,
    commitPreferredRegionSelection: model.commitPreferredRegionSelection,
    commitStrictSelection: model.commitStrictSelection,
  },
}

// The IPC shape: the Rust DTO maps `available_regions` to `regions`; everything else is as captured.
const response = (preferred: string | null, choices = captured.choices, strict = false) => ({
  preferred_region: preferred, region_strict: strict, regions: [], choices,
})

function open(backend: Record<string, (args: any) => unknown>) {
  const m = mount('MacSettings.tsx', 'DataResidencyGroup', {
    backend: backend as any,
    expand: true,
    hookModules: [hook],
    bindings: { ...parts, residencySummary: model.residencySummary },
  })
  mounted.push(m)
  return m
}
const settle = async (m: Mounted) => { await m.flush(); await m.flush(); await m.flush() }
const find = (m: Mounted, match: (el: TreeNode) => boolean) => m.elements().filter(match)
const radios = (m: Mounted) => find(m, (el) => el.props.role === 'radio')
const radio = (m: Mounted, id: string) => {
  const hit = radios(m).filter((el) => el.props['data-row'] === `region-${id}`)
  if (hit.length !== 1) throw new Error(`expected one radio ${id}, found ${hit.length}`)
  return hit[0]
}
const checkedIds = (m: Mounted) => radios(m).filter((el) => el.props['aria-checked'] === true).map((el) => el.props['data-row'])
function readable(node: any): string[] {
  if (Array.isArray(node)) return node.flatMap(readable)
  if (node == null || typeof node === 'boolean') return []
  if (typeof node !== 'object' || !node.props) return [String(node)]
  if (node.type === 'svg') return []
  return readable(node.props.children)
}
const visibleText = (m: Mounted) => readable(expand(m.tree())).join(' | ')

const switchEl = (m: Mounted) => find(m, (el) => el.props.role === 'switch')

describe('DataResidencyGroup', () => {
  test('lists the API rows in order: Europe the default, two live countries, the Netherlands coming soon and not selectable', async () => {
    const m = open({ account_region: () => response(null) })
    await settle(m)
    expect(radios(m).map((el) => el.props['data-row'])).toEqual(['region-europe', 'region-de', 'region-fi', 'region-nl'])
    expect(checkedIds(m)).toEqual(['region-europe'])
    const text = visibleText(m)
    expect(text).toContain('Stored in the EU. New files go to any of our European data centers.')
    expect(text).toContain('Germany')
    expect(text).toContain('Finland')
    expect(text).toContain('Netherlands')
    expect(text).toContain('Default')
    expect(text).toContain('Coming soon')
    for (const id of ['europe', 'de', 'fi']) expect(radio(m, id).props.disabled).toBe(false)
    expect(radio(m, 'nl').props.disabled).toBe(true)
    expect(find(m, (el) => el.props.role === 'radiogroup')).toHaveLength(1)
    expect(m.calls.map((c) => c.name)).toEqual(['account_region'])
  })

  test('the list follows the API: change the data and the rows change, with no client change', async () => {
    const changed = captured.choices
      .filter((c) => c.id !== 'de')
      .map((c) => (c.id === 'nl' ? { ...c, status: 'live', selectable: true } : c))
    const m = open({ account_region: () => response('fi', changed) })
    await settle(m)
    expect(radios(m).map((el) => el.props['data-row'])).toEqual(['region-europe', 'region-fi', 'region-nl'])
    expect(checkedIds(m)).toEqual(['region-fi'])
    expect(radio(m, 'nl').props.disabled).toBe(false)
    expect(visibleText(m)).toContain('Stored in the EU. New files go to Finland.')
    expect(visibleText(m)).not.toContain('Coming soon')
  })

  test('choosing Finland sends its id verbatim with strict stated, then reads as chosen', async () => {
    let stored: string | null = null
    const m = open({
      account_region: () => response(stored),
      account_set_region: (args) => { stored = args.preferredRegion; return { preferred_region: stored, region_strict: args.regionStrict } },
    })
    await settle(m)
    await radio(m, 'fi').props.onClick()
    await m.flush()
    expect(m.calls.filter((c) => c.name === 'account_set_region').map((c) => c.args)).toEqual([
      { preferredRegion: 'fi', regionStrict: false },
    ])
    expect(checkedIds(m)).toEqual(['region-fi'])
    expect(m.toasts).toEqual([])

    // Back to Europe: the id is "europe", not a null the server would have to interpret.
    await radio(m, 'europe').props.onClick()
    await m.flush()
    expect(m.calls.filter((c) => c.name === 'account_set_region').map((c) => c.args)).toEqual([
      { preferredRegion: 'fi', regionStrict: false }, { preferredRegion: 'europe', regionStrict: false },
    ])
    expect(checkedIds(m)).toEqual(['region-europe'])
  })

  test('a coming-soon country sends nothing even if its handler is driven', async () => {
    const m = open({ account_region: () => response(null), account_set_region: () => { throw new Error('must not be sent') } })
    await settle(m)
    await radio(m, 'nl').props.onClick()
    await m.flush()
    expect(m.calls.some((c) => c.name === 'account_set_region')).toBe(false)
    expect(checkedIds(m)).toEqual(['region-europe'])
  })

  test('choosing the row already chosen sends nothing', async () => {
    const m = open({ account_region: () => response('de'), account_set_region: () => { throw new Error('must not be sent') } })
    await settle(m)
    await radio(m, 'de').props.onClick()
    await m.flush()
    expect(m.calls.some((c) => c.name === 'account_set_region')).toBe(false)
  })

  test('a refused choice is ONE toast and the row goes back to what is saved', async () => {
    const m = open({
      account_region: () => response('de'),
      account_set_region: () => { throw new Error("400 region_unavailable: 'fi' is not available") },
    })
    await settle(m)
    await radio(m, 'fi').props.onClick()
    await m.flush()
    expect(checkedIds(m)).toEqual(['region-de'])
    expect(visibleErrorSurfaces(m)).toHaveLength(1)
    expect(m.toasts).toHaveLength(1)
    expect(m.toasts[0].title).toBe('Couldn’t change where files are stored')
    expect(m.toasts[0].message).toContain('region_unavailable')
    expect(find(m, (el) => el.props.role === 'alert')).toHaveLength(0)
  })

  test('a list that cannot load is ONE inline alert with Try again, and Try again loads it', async () => {
    let up = false
    const m = open({ account_region: () => { if (!up) throw new Error('HTTP 503'); return response(null) } })
    await settle(m)
    expect(radios(m)).toHaveLength(0)
    expect(find(m, (el) => el.props.role === 'alert')).toHaveLength(1)
    expect(visibleErrorSurfaces(m)).toHaveLength(1)
    expect(m.toasts).toEqual([])
    up = true
    await m.click('Try again')
    await settle(m)
    expect(radios(m)).toHaveLength(4)
    expect(find(m, (el) => el.props.role === 'alert')).toHaveLength(0)
    expect(m.calls.filter((c) => c.name === 'account_region')).toHaveLength(2)
  })

  test('a stored country that left the list is said plainly, with no row chosen and no error', async () => {
    const m = open({ account_region: () => response('fi', captured.choices.filter((c) => c.id !== 'fi')) })
    await settle(m)
    expect(visibleText(m)).toContain('Your chosen location isn’t available right now.')
    expect(checkedIds(m)).toEqual([])
    expect(visibleErrorSurfaces(m)).toEqual([])
    expect(radios(m)).toHaveLength(3)
  })

  test('only Europe selectable: nothing to choose, nothing promised', async () => {
    const m = open({ account_region: () => response(null, captured.choices.filter((c) => c.id === 'europe')) })
    await settle(m)
    expect(radios(m).every((el) => el.props.disabled === true)).toBe(true)
    expect(visibleText(m)).not.toContain('Existing files stay')
  })

  test('no provider, endpoint, city or flag anywhere in what a user reads', async () => {
    const m = open({ account_region: () => response('de') })
    await settle(m)
    const text = visibleText(m)
    expect(text).not.toMatch(/hetzner|provider|endpoint/i)
    expect(text).not.toMatch(/Falkenstein|Helsinki|Nuremberg|N[uü]rnberg|\bEde\b/)
    expect(text).not.toMatch(/\p{Regional_Indicator}/u)
    expect(textOf(m.tree())).toBeTruthy()
  })
})

describe('DataResidencyGroup: only store in <country>', () => {
  test('is not offered while Europe is chosen', async () => {
    const m = open({ account_region: () => response(null, captured.choices, true) })
    await settle(m)
    expect(switchEl(m)).toHaveLength(0)
    expect(visibleText(m)).not.toContain('Only store in')
  })

  test('is offered for a chosen country, named after it, and reads the saved value', async () => {
    const m = open({ account_region: () => response('de', captured.choices, true) })
    await settle(m)
    expect(visibleText(m)).toContain('Only store in Germany')
    expect(visibleText(m)).toContain('the upload stops instead of going to another EU country.')
    expect(switchEl(m)).toHaveLength(1)
    expect(switchEl(m)[0].props['aria-checked']).toBe(true)
  })

  test('turning it on re-sends the chosen country with regionStrict true', async () => {
    let strict = false
    const m = open({
      account_region: () => response('fi', captured.choices, strict),
      account_set_region: (args) => { strict = args.regionStrict; return { preferred_region: args.preferredRegion, region_strict: strict } },
    })
    await settle(m)
    expect(switchEl(m)[0].props['aria-checked']).toBe(false)
    await switchEl(m)[0].props.onClick()
    await m.flush()
    expect(m.calls.filter((c) => c.name === 'account_set_region').map((c) => c.args)).toEqual([
      { preferredRegion: 'fi', regionStrict: true },
    ])
    expect(switchEl(m)[0].props['aria-checked']).toBe(true)
    expect(visibleText(m)).toContain('Only store in Finland')
  })

  test('a refused strict change is ONE toast and the switch goes back', async () => {
    const m = open({
      account_region: () => response('fi'),
      account_set_region: () => { throw new Error('400 region_strict_requires_datacenter') },
    })
    await settle(m)
    await switchEl(m)[0].props.onClick()
    await m.flush()
    expect(switchEl(m)[0].props['aria-checked']).toBe(false)
    expect(m.toasts).toHaveLength(1)
    expect(m.toasts[0].message).toContain('region_strict_requires_datacenter')
  })

  test('moving to another country keeps strict; moving to Europe ends it', async () => {
    let strict = true
    const m = open({
      account_region: () => response('de', captured.choices, strict),
      account_set_region: (args) => { strict = args.regionStrict; return { preferred_region: args.preferredRegion, region_strict: strict } },
    })
    await settle(m)
    await radio(m, 'fi').props.onClick()
    await m.flush()
    await radio(m, 'europe').props.onClick()
    await m.flush()
    expect(m.calls.filter((c) => c.name === 'account_set_region').map((c) => c.args)).toEqual([
      { preferredRegion: 'fi', regionStrict: true },
      { preferredRegion: 'europe', regionStrict: false },
    ])
  })
})
