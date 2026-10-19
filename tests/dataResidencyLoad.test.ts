import { expect, test } from 'bun:test'
import { buildDataResidencyViewState, loadDataResidency } from '../src/windows/dataResidencySettingsModel'
import type { CommandResult, UserRegionResponse } from '../src/desktopApi'

const response: UserRegionResponse = {
  preferred_region: null,
  regions: [{ continent: 'europe', display_name: 'Europe', country: null, is_default: true }],
}

test('failed load has no invented cards and Retry loads the actual API regions', async () => {
  let calls = 0
  const fetchRegion = async (): Promise<CommandResult<UserRegionResponse>> => {
    calls++
    return calls === 1
      ? { ok: false, reason: 'HTTP 503', unsupported: false }
      : { ok: true, value: response }
  }
  const failed = await loadDataResidency(fetchRegion)
  expect(failed.error).toBe('Data residency is unavailable. Please try again.')
  expect(failed.choices).toEqual([])
  expect(failed.preferredRegion).toBeNull()
  expect(failed.regionStrict).toBe(false)
  const retried = await loadDataResidency(fetchRegion)
  expect(calls).toBe(2)
  expect(retried.error).toBeNull()
  expect(buildDataResidencyViewState({ ...retried, saving: false }).items).toEqual([{
    id: 'europe', kind: 'region', displayName: 'Europe', detail: 'Any of our European data centers. We place your files.',
    locationLabel: 'any of our European data centers', isDefault: true, selected: true, comingSoon: false, disabled: true,
  }])
})

test('empty successful response is unavailable, without a default region', async () => {
  const state = await loadDataResidency(async () => ({ ok: true, value: { regions: [], preferred_region: null } }))
  expect(state.error).toBe('No storage regions are currently available. Please try again.')
  expect(state.choices).toEqual([])
})

test('unexpected invocation rejection becomes a recoverable error', async () => {
  const state = await loadDataResidency(async () => { throw new Error('invoke failed') })
  expect(state.error).toBe('Data residency is unavailable. Please try again.')
  expect(state.choices).toEqual([])
})
