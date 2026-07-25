import { describe, expect, test } from 'bun:test'
import {
  buildDataResidencyViewState,
  commitPreferredRegionSelection,
} from '../src/windows/dataResidencySettingsModel'
import type { RegionInfo } from '../src/desktopApi'

const regions: RegionInfo[] = [
  {
    continent: 'europe',
    display_name: 'Europe',
    city: 'Falkenstein',
    provider: 'Hetzner',
    is_default: true,
  },
  {
    continent: 'us',
    display_name: 'North America',
    city: 'Ashburn',
    provider: 'Do Not Render',
    is_default: false,
  },
]

describe('dataResidencySettingsModel', () => {
  test('renders the only available region as selected and disabled without exposing provider names', () => {
    const view = buildDataResidencyViewState({
      preferredRegion: null,
      regions: [regions[0]],
      saving: false,
    })

    expect(view.onlyOneRegion).toBe(true)
    expect(view.items).toEqual([
      {
        continent: 'europe',
        displayName: 'Europe',
        locationLabel: 'Falkenstein, Germany',
        isDefault: true,
        selected: true,
        disabled: true,
      },
    ])
    expect(JSON.stringify(view)).not.toContain('Hetzner')
  })

  test('optimistically selects a region and rolls back when saving fails', async () => {
    const selected: Array<string | null> = []

    const result = await commitPreferredRegionSelection({
      continent: 'us',
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
      continent: 'US',
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
