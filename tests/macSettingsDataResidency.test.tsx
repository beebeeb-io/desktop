/**
 * The macOS Settings storage-location group (task 1919): the real `DataResidencyGroup` component and the real
 * `useDataResidency` hook, executed with the component harness against a scripted backend that answers like
 * `GET /me/region` (the fixture is a response captured from a real local API).
 *
 * What this proves: the rows come from the backend's list and follow it when it changes; Europe is the default;
 * a live datacenter sends its id verbatim; a coming-soon datacenter cannot be chosen; one failure is one surface.
 * What it does NOT prove: layout, focus, or anything on a Mac (tests/render-mac-settings.mjs and the app run do).
 */
import { afterEach, describe, expect, test } from 'bun:test'
import { createElement, Fragment } from 'react'
import captured from '../src-tauri/tests/fixtures/region-choices-api.json'
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
  },
}

// The IPC shape: the Rust DTO maps `available_regions` to `regions`; everything else is as captured.
const response = (preferred: string | null, choices = captured.choices) => ({ preferred_region: preferred, regions: [], choices })

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

describe('DataResidencyGroup', () => {
  test('lists the API rows in order: Europe the default, three live datacenters, Ede coming soon and not selectable', async () => {
    const m = open({ account_region: () => response(null) })
    await settle(m)
    expect(radios(m).map((el) => el.props['data-row'])).toEqual(['region-europe', 'region-fsn1', 'region-hel1', 'region-nbg1', 'region-ede'])
    expect(checkedIds(m)).toEqual(['region-europe'])
    const text = visibleText(m)
    expect(text).toContain('Stored in the EU. New files go to any of our datacenters in Europe.')
    expect(text).toContain('Default')
    expect(text).toContain('Coming soon')
    for (const id of ['europe', 'fsn1', 'hel1', 'nbg1']) expect(radio(m, id).props.disabled).toBe(false)
    expect(radio(m, 'ede').props.disabled).toBe(true)
    expect(find(m, (el) => el.props.role === 'radiogroup')).toHaveLength(1)
    expect(m.calls.map((c) => c.name)).toEqual(['account_region'])
  })

  test('the list follows the API: change the data and the rows change, with no client change', async () => {
    const changed = captured.choices
      .filter((c) => c.id !== 'nbg1')
      .map((c) => (c.id === 'ede' ? { ...c, status: 'live', selectable: true } : c))
    const m = open({ account_region: () => response('hel1', changed) })
    await settle(m)
    expect(radios(m).map((el) => el.props['data-row'])).toEqual(['region-europe', 'region-fsn1', 'region-hel1', 'region-ede'])
    expect(checkedIds(m)).toEqual(['region-hel1'])
    expect(radio(m, 'ede').props.disabled).toBe(false)
    expect(visibleText(m)).toContain('Stored in the EU. New files go to Helsinki, Finland.')
    expect(visibleText(m)).not.toContain('Coming soon')
  })

  test('choosing Helsinki sends its id verbatim, then reads as chosen', async () => {
    let stored: string | null = null
    const m = open({
      account_region: () => response(stored),
      account_set_region: (args) => { stored = args.preferredRegion; return { preferred_region: stored } },
    })
    await settle(m)
    await radio(m, 'hel1').props.onClick()
    await m.flush()
    expect(m.calls.filter((c) => c.name === 'account_set_region').map((c) => c.args)).toEqual([{ preferredRegion: 'hel1' }])
    expect(checkedIds(m)).toEqual(['region-hel1'])
    expect(m.toasts).toEqual([])

    // Back to Europe: the id is "europe", not a null the server would have to interpret.
    await radio(m, 'europe').props.onClick()
    await m.flush()
    expect(m.calls.filter((c) => c.name === 'account_set_region').map((c) => c.args)).toEqual([
      { preferredRegion: 'hel1' }, { preferredRegion: 'europe' },
    ])
    expect(checkedIds(m)).toEqual(['region-europe'])
  })

  test('a coming-soon datacenter sends nothing even if its handler is driven', async () => {
    const m = open({ account_region: () => response(null), account_set_region: () => { throw new Error('must not be sent') } })
    await settle(m)
    await radio(m, 'ede').props.onClick()
    await m.flush()
    expect(m.calls.some((c) => c.name === 'account_set_region')).toBe(false)
    expect(checkedIds(m)).toEqual(['region-europe'])
  })

  test('choosing the row already chosen sends nothing', async () => {
    const m = open({ account_region: () => response('nbg1'), account_set_region: () => { throw new Error('must not be sent') } })
    await settle(m)
    await radio(m, 'nbg1').props.onClick()
    await m.flush()
    expect(m.calls.some((c) => c.name === 'account_set_region')).toBe(false)
  })

  test('a refused choice is ONE toast and the row goes back to what is saved', async () => {
    const m = open({
      account_region: () => response('fsn1'),
      account_set_region: () => { throw new Error("400 region_unavailable: 'hel1' is not available") },
    })
    await settle(m)
    await radio(m, 'hel1').props.onClick()
    await m.flush()
    expect(checkedIds(m)).toEqual(['region-fsn1'])
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
    expect(radios(m)).toHaveLength(5)
    expect(find(m, (el) => el.props.role === 'alert')).toHaveLength(0)
    expect(m.calls.filter((c) => c.name === 'account_region')).toHaveLength(2)
  })

  test('a stored datacenter that left the list is said plainly, with no row chosen and no error', async () => {
    const m = open({ account_region: () => response('hel1', captured.choices.filter((c) => c.id !== 'hel1')) })
    await settle(m)
    expect(visibleText(m)).toContain('Your chosen location isn’t available right now.')
    expect(checkedIds(m)).toEqual([])
    expect(visibleErrorSurfaces(m)).toEqual([])
    expect(radios(m)).toHaveLength(4)
  })

  test('only Europe selectable: nothing to choose, nothing promised', async () => {
    const m = open({ account_region: () => response(null, captured.choices.filter((c) => c.id === 'europe')) })
    await settle(m)
    expect(radios(m).every((el) => el.props.disabled === true)).toBe(true)
    expect(visibleText(m)).not.toContain('Existing files stay')
  })

  test('no provider, endpoint or flag anywhere in what a user reads', async () => {
    const m = open({ account_region: () => response('nbg1') })
    await settle(m)
    const text = visibleText(m)
    expect(text).not.toMatch(/hetzner|provider|endpoint/i)
    expect(text).not.toMatch(/\p{Regional_Indicator}/u)
    expect(textOf(m.tree())).toBeTruthy()
  })
})
