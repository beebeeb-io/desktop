import { expect, test } from 'bun:test'
import { buildDataResidencyViewState, loadDataResidency } from '../src/windows/dataResidencySettingsModel'
import type { CommandResult, UserRegionResponse } from '../src/desktopApi'

const response: UserRegionResponse = {
  preferred_region: null,
  regions: [{ continent: 'europe', display_name: 'Europe', city: 'Local', is_default: true }],
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
  expect(failed.regions).toEqual([])
  expect(failed.preferredRegion).toBeNull()
  const retried = await loadDataResidency(fetchRegion)
  expect(calls).toBe(2)
  expect(retried.error).toBeNull()
  expect(buildDataResidencyViewState({ ...retried, saving: false }).items).toEqual([{
    continent: 'europe', displayName: 'Europe', locationLabel: 'Local',
    isDefault: true, selected: true, disabled: true,
  }])
})

test('empty successful response is unavailable, without a default region', async () => {
  const state = await loadDataResidency(async () => ({ ok: true, value: { regions: [], preferred_region: null } }))
  expect(state.error).toBe('No storage regions are currently available. Please try again.')
  expect(state.regions).toEqual([])
})

test('unexpected invocation rejection becomes a recoverable error', async () => {
  const state = await loadDataResidency(async () => { throw new Error('invoke failed') })
  expect(state.error).toBe('Data residency is unavailable. Please try again.')
  expect(state.regions).toEqual([])
})
