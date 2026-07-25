import { describe, expect, test } from 'bun:test'
import {
  buildDowngradeConfirmationViewModel,
  buildUpdateCheckViewModel,
  stateFromManualUpdateResult,
  type ManualUpdateCheckState,
} from '../src/windows/updateCheckViewModel'
import type { ManualUpdateCheckResult } from '../src/desktopApi'

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
      version: '0.1.2',
    }

    expect(buildUpdateCheckViewModel(state, '0.1.1', 'stable')).toEqual({
      title: 'Version 0.1.2 is available',
      detail: 'Use the update banner to restart and apply the Beta update.',
      chip: 'Update available',
      tone: 'amber',
    })
  })

  test('maps and shows downgrade-available result without reusing update copy', () => {
    const result: ManualUpdateCheckResult = {
      status: 'downgrade_available',
      current_version: '0.2.0',
      current_channel: 'beta',
      channel: 'stable',
      version: '0.1.9',
      body: 'Stable notes',
      release_notes_url: 'https://github.com/beebeeb-io/desktop/releases/tag/desktop-v0.1.9',
    }

    const state = stateFromManualUpdateResult(result)

    expect(state).toEqual({
      kind: 'downgrade_available',
      currentVersion: '0.2.0',
      currentChannel: 'beta',
      channel: 'stable',
      version: '0.1.9',
      body: 'Stable notes',
      releaseNotesUrl: 'https://github.com/beebeeb-io/desktop/releases/tag/desktop-v0.1.9',
    })
    expect(buildUpdateCheckViewModel(state, '0.2.0', 'stable')).toEqual({
      title: 'Stable is at version 0.1.9',
      detail: 'You are running 0.2.0 from the Beta channel. Confirm before downgrading.',
      chip: 'Downgrade available',
      tone: 'amber',
    })
  })

  test('keeps downgrade confirmation copy explicit about version channel and restart', () => {
    const state: ManualUpdateCheckState = {
      kind: 'downgrade_available',
      currentVersion: '0.2.0-beta.1',
      currentChannel: 'beta',
      channel: 'stable',
      version: '0.1.9',
      body: 'Stable notes',
      releaseNotesUrl: 'https://github.com/beebeeb-io/desktop/releases/tag/desktop-v0.1.9',
    }

    expect(buildDowngradeConfirmationViewModel(state)).toEqual({
      title: 'Confirm downgrade',
      message:
        'Downgrade Beebeeb from 0.2.0-beta.1 to 0.1.9 on the Stable channel? The signed installer will run and Beebeeb will restart if it succeeds.',
      confirmLabel: 'Downgrade to 0.1.9',
      cancelLabel: 'Cancel',
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
