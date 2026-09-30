import { expect, test } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { DataResidencyContent } from '../src/windows/views/SettingsView'
import { buildDataResidencyViewState, loadDataResidency } from '../src/windows/dataResidencySettingsModel'

const noop = () => {}
test('failed request renders an honest error and Retry without a fabricated region', async () => {
  const state = await loadDataResidency(async () => ({ ok: false, reason: 'HTTP 500', unsupported: false }))
  const html = renderToStaticMarkup(<DataResidencyContent loading={false} loadError={state.error}
    view={buildDataResidencyViewState({ ...state, saving: false })} onRetry={noop} onSelect={noop} />)
  expect(html).toContain('Could not load data residency')
  expect(html).toContain('Retry')
  expect(html).not.toContain('Falkenstein')
  expect(html).not.toContain('Default')
  expect(html).not.toContain('Existing files stay')
  expect(html).not.toContain('Stored in')
})

const regions = [
  { continent: 'europe', display_name: 'Europe', city: 'Falkenstein', is_default: true },
  { continent: 'us', display_name: 'North America', city: 'Ashburn', is_default: false, provider: 'Never Display Storage Provider' },
]

function renderRegion(preferredRegion: string | null, loading = false, loadError: string | null = null, metadata = regions) {
  return renderToStaticMarkup(<DataResidencyContent loading={loading} loadError={loadError}
    view={buildDataResidencyViewState({ regions: metadata, preferredRegion, saving: false })} onRetry={noop} onSelect={noop} />)
}

test('US preference uses its loaded display name instead of the EU default', () => {
  const html = renderRegion('us')
  expect(html).toContain('Stored in North America (currently Ashburn, United States).')
  expect(html).not.toContain('Stored in the EU')
  expect(html).not.toContain('Never Display Storage Provider')
})

test('explicit EU preference and implicit EU default both claim EU residency', () => {
  for (const preferredRegion of ['europe', null]) {
    const html = renderRegion(preferredRegion)
    expect(html).toContain('Stored in the EU (currently Falkenstein, Germany).')
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
    { preferredRegion: 'unknown', metadata: regions },
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

test('API region renders its actual city, EU copy and no provider or region-choice promise', () => {
  const region = { continent: 'europe', display_name: 'Europe', city: 'Local', is_default: true, provider: 'Never Display Storage Provider' }
  const html = renderToStaticMarkup(<DataResidencyContent loading={false} loadError={null}
    view={buildDataResidencyViewState({ regions: [region], preferredRegion: null, saving: false })} onRetry={noop} onSelect={noop} />)
  expect(html).not.toContain('Never Display Storage Provider')
  expect(html).toContain('Local')
  expect(html).toContain('Stored in the EU (currently Local).')
  expect(html).toContain('disabled=""')
  expect(html).not.toContain('Choose where')
  expect(html).not.toContain('More regions')
  expect(html).not.toContain('Retry')
})

// A new EU pool must be displayed from metadata, never a hardcoded default.
test('transparency follows the current pool city', () => {
  const html = renderRegion(null, false, null, [
    { continent: 'europe', display_name: 'Europe', city: 'Helsinki', is_default: true },
  ])
  expect(html).toContain('Stored in the EU (currently Helsinki, Finland).')
  expect(html).not.toContain('Falkenstein')
})
