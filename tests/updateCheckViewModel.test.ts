import { describe, expect, test } from 'bun:test'
import {
  buildUpdateCheckViewModel,
  type ManualUpdateCheckState,
} from '../src/windows/updateCheckViewModel'

describe('updateCheckViewModel', () => {
  test('shows an honest idle state with the selected channel and current version', () => {
    expect(buildUpdateCheckViewModel({ kind: 'idle' }, '0.1.1-beta.1', 'beta')).toEqual({
      title: 'Version 0.1.1-beta.1',
      detail: 'Check the Beta channel manifest when you want an immediate answer.',
      chip: 'Not checked',
      tone: 'neutral',
    })
  })

  test('shows progress while checking the selected channel', () => {
    expect(buildUpdateCheckViewModel({ kind: 'checking' }, '0.1.1', 'alpha')).toEqual({
      title: 'Checking Alpha...',
      detail: 'Contacting the Alpha channel manifest.',
      chip: 'Checking',
      tone: 'amber',
    })
  })

  test('shows up-to-date result with current version and channel', () => {
    const state: ManualUpdateCheckState = {
      kind: 'up_to_date',
      currentVersion: '0.1.1',
      channel: 'stable',
    }

    expect(buildUpdateCheckViewModel(state, '0.1.0', 'beta')).toEqual({
      title: 'Version 0.1.1 is current',
      detail: 'Stable has no newer desktop update for this install.',
      chip: 'Up to date',
      tone: 'green',
    })
  })

  test('shows update-available handoff copy for the banner flow', () => {
    const state: ManualUpdateCheckState = {
      kind: 'update_available',
      currentVersion: '0.1.1',
      channel: 'beta',
      version: '0.1.2-beta.1',
    }

    expect(buildUpdateCheckViewModel(state, '0.1.1', 'stable')).toEqual({
      title: 'Version 0.1.2-beta.1 is available',
      detail: 'Use the update banner to restart and apply the Beta update.',
      chip: 'Update available',
      tone: 'amber',
    })
  })

  test('shows reachable error text without hiding the reason', () => {
    const state: ManualUpdateCheckState = {
      kind: 'error',
      reason: 'failed to fetch manifest',
    }

    expect(buildUpdateCheckViewModel(state, '0.1.1', 'stable')).toEqual({
      title: 'Could not check for updates',
      detail: 'failed to fetch manifest',
      chip: 'Check failed',
      tone: 'neutral',
    })
  })
})
