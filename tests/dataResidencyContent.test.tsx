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
})

test('API region renders its actual city, EU copy and no provider or region-choice promise', () => {
  const region = { continent: 'europe', display_name: 'Europe', city: 'Local', is_default: true, provider: 'Never Display Storage Provider' }
  const html = renderToStaticMarkup(<DataResidencyContent loading={false} loadError={null}
    view={buildDataResidencyViewState({ regions: [region], preferredRegion: null, saving: false })} onRetry={noop} onSelect={noop} />)
  expect(html).not.toContain('Never Display Storage Provider')
  expect(html).toContain('Local')
  expect(html).toContain('Stored in the EU.')
  expect(html).toContain('disabled=""')
  expect(html).not.toContain('Choose where')
  expect(html).not.toContain('More regions')
  expect(html).not.toContain('Retry')
})
