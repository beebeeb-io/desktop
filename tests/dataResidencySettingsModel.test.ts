import { describe, expect, test } from 'bun:test'
import {
  buildDataResidencyViewState,
  choicesFromResponse,
  commitPreferredRegionSelection,
} from '../src/windows/dataResidencySettingsModel'
import type { RegionInfo } from '../src/desktopApi'

const regions: RegionInfo[] = [
  {
    continent: 'europe',
    display_name: 'Europe',
    country: null,
    is_default: true,
  },
  {
    continent: 'us',
    display_name: 'North America',
    country: null,
    is_default: false,
  },
]

describe('dataResidencySettingsModel', () => {
  test('renders the only available region as selected and disabled without exposing provider names', () => {
    const view = buildDataResidencyViewState({
      preferredRegion: null,
      choices: choicesFromResponse({ regions: [regions[0]] }),
      saving: false,
    })

    expect(view.onlyOneRegion).toBe(true)
    expect(view.items).toEqual([
      {
        id: 'europe',
        kind: 'region',
        displayName: 'Europe',
        // A response without `choices` names no country, and the client has no table to guess one from.
        detail: 'Any of our European data centers. We place your files.',
        locationLabel: 'any of our European data centers',
        isDefault: true,
        selected: true,
        comingSoon: false,
        disabled: true,
      },
    ])
    expect(JSON.stringify(view)).not.toContain('provider')
  })

  test('optimistically selects a region and rolls back when saving fails', async () => {
    const selected: Array<string | null> = []

    const result = await commitPreferredRegionSelection({
      id: 'us',
      preferredRegion: 'europe',
      saving: false,
      setPreferredRegion: next => selected.push(next),
      savePreferredRegion: async () => {
        throw new Error('network down')
      },
    })

    expect(selected).toEqual(['us', 'europe'])
    expect(result).toEqual({
      status: 'error',
      previousPreferredRegion: 'europe',
      message: 'network down',
    })
  })

  test('keeps server-normalized preferred region after a successful save', async () => {
    const selected: Array<string | null> = []

    const result = await commitPreferredRegionSelection({
      id: 'US',
      preferredRegion: 'europe',
      saving: false,
      setPreferredRegion: next => selected.push(next),
      savePreferredRegion: async () => ({ preferred_region: 'us' }),
    })

    expect(selected).toEqual(['US', 'us'])
    expect(result).toEqual({
      status: 'saved',
      previousPreferredRegion: 'europe',
      preferredRegion: 'us',
    })
  })
})
