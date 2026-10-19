import { expect, test } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { DataResidencyContent } from '../src/windows/views/SettingsView'
import { buildDataResidencyViewState, choicesFromResponse, loadDataResidency } from '../src/windows/dataResidencySettingsModel'

const noop = () => {}
test('failed request renders an honest error and Retry without a fabricated region', async () => {
  const state = await loadDataResidency(async () => ({ ok: false, reason: 'HTTP 500', unsupported: false }))
  const html = renderToStaticMarkup(<DataResidencyContent loading={false} loadError={state.error}
    view={buildDataResidencyViewState({ ...state, saving: false })} onRetry={noop} onSelect={noop} onStrictChange={noop} />)
  expect(html).toContain('Could not load data residency')
  expect(html).toContain('Retry')
  expect(html).not.toContain('Falkenstein')
  expect(html).not.toContain('Default')
  expect(html).not.toContain('Existing files stay')
  expect(html).not.toContain('Stored in')
})

const regions = [
  { continent: 'europe', display_name: 'Europe', country: null, is_default: true },
  { continent: 'us', display_name: 'North America', country: null, is_default: false, provider: 'Never Display Storage Provider' },
]

function renderRegion(preferredRegion: string | null, loading = false, loadError: string | null = null, metadata = regions) {
  return renderToStaticMarkup(<DataResidencyContent loading={loading} loadError={loadError}
    view={buildDataResidencyViewState({ choices: choicesFromResponse({ regions: metadata }), preferredRegion, saving: false })} onRetry={noop} onSelect={noop} onStrictChange={noop} />)
}

test('US preference uses its loaded display name instead of the EU default', () => {
  const html = renderRegion('us')
  expect(html).toContain('Stored in North America.')
  expect(html).not.toContain('Stored in the EU')
  expect(html).not.toContain('Never Display Storage Provider')
})

test('explicit EU preference and implicit EU default both claim EU residency', () => {
  for (const preferredRegion of ['europe', null]) {
    const html = renderRegion(preferredRegion)
    expect(html).toContain('Stored in the EU. New files go to any of our European data centers.')
    expect(html).not.toContain('Stored in North America')
  }
})

test('initial load and refresh show neutral text even with previously loaded metadata', () => {
  for (const metadata of [[], regions]) {
    const html = renderRegion(null, true, null, metadata)
    expect(html).toContain('Loading data residency')
    expect(html).not.toContain('Stored in')
    expect(html).not.toContain('Falkenstein')
    expect(html).not.toContain('Retry')
  }
})

test('failed refresh hides stale location claims and cards', () => {
  const html = renderRegion('europe', false, 'Data residency is unavailable. Please try again.')
  expect(html).toContain('Data residency is unavailable. Please try again.')
  expect(html).toContain('Retry')
  expect(html).not.toContain('Stored in')
  expect(html).not.toContain('Falkenstein')
  expect(html).not.toContain('Existing files stay')
})

test('unknown effective region never falls back to another region and offers Retry', () => {
  const cases = [
    { preferredRegion: null, metadata: [] },
    { preferredRegion: null, metadata: regions.map(region => ({ ...region, is_default: false })) },
    { preferredRegion: 'us', metadata: regions.map(region => ({ ...region, display_name: '' })) },
  ]
  for (const { preferredRegion, metadata } of cases) {
    const html = renderRegion(preferredRegion, false, null, metadata)
    expect(html).toContain('Could not load data residency')
    expect(html).toContain('Retry')
    expect(html).not.toContain('Stored in')
    expect(html).not.toContain('Falkenstein')
    expect(html).not.toContain('Existing files stay')
  }
})

// A stored preference that the list no longer has is said plainly (task 1919): the rows are fine and none is selected.
test('a stored region the list does not have is a plain note with no selected row, not a load error', () => {
  const html = renderRegion('unknown')
  expect(html).toContain('Your chosen location isn’t available right now.')
  expect(html).not.toContain('Could not load data residency')
  expect(html).not.toContain('Stored in')
  expect(html).not.toContain('aria-pressed="true"')
})

test('API region renders EU copy with no city, no provider and no region-choice promise', () => {
  const region = { continent: 'europe', display_name: 'Europe', country: null, city: 'Local', is_default: true, provider: 'Never Display Storage Provider' }
  const html = renderToStaticMarkup(<DataResidencyContent loading={false} loadError={null}
    view={buildDataResidencyViewState({ choices: choicesFromResponse({ regions: [region] }), preferredRegion: null, saving: false })} onRetry={noop} onSelect={noop} onStrictChange={noop} />)
  expect(html).not.toContain('Never Display Storage Provider')
  expect(html).not.toContain('Local')
  expect(html).toContain('Stored in the EU. New files go to any of our European data centers.')
  expect(html).toContain('disabled=""')
  expect(html).not.toContain('Choose where')
  expect(html).not.toContain('More regions')
  expect(html).not.toContain('Retry')
})

// A legacy list that still carries the deprecated `city` (an IPC from before 2050) shows no city.
test('a city in the legacy list is never rendered, whatever the pool is', () => {
  const html = renderRegion(null, false, null, [
    { continent: 'europe', display_name: 'Europe', country: null, city: 'Helsinki', is_default: true } as never,
  ])
  expect(html).toContain('Stored in the EU. New files go to any of our European data centers.')
  expect(html).not.toMatch(/Helsinki|Falkenstein/)
})
