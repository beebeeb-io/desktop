import { describe, expect, test } from 'bun:test'
import {
  ALWAYS_ACCESSIBLE_SETTINGS,
  SETTINGS_SECTIONS,
  availableSettingsSections,
  defaultSettingsPage,
  settingLabel,
} from '../src/windows/settingsNavigation'

describe('settingsNavigation', () => {
  test('keeps the consolidated settings pages grouped under one in-app section', () => {
    expect(SETTINGS_SECTIONS).toEqual([
      {
        heading: 'Beebeeb',
        items: [
          { id: 'sync', label: 'Sync', icon: 'cloud' },
          { id: 'data-residency', label: 'Data residency', icon: 'shield' },
          { id: 'notifications', label: 'Notifications', icon: 'cog' },
        ],
      },
      {
        heading: 'System',
        items: [
          { id: 'launch', label: 'Launch', icon: 'play' },
          { id: 'explorer-integration', label: 'Explorer integration', icon: 'folder' },
          { id: 'updates', label: 'Updates', icon: 'download' },
          { id: 'advanced', label: 'Advanced', icon: 'cog' },
        ],
      },
    ])
  })

  test('hides signed-in-only settings while preserving system pages', () => {
    expect([...ALWAYS_ACCESSIBLE_SETTINGS]).toEqual(['launch', 'explorer-integration', 'updates', 'advanced'])
    expect(defaultSettingsPage(false)).toBe('launch')
    expect(availableSettingsSections(false)).toEqual([
      {
        heading: 'System',
        items: [
          { id: 'launch', label: 'Launch', icon: 'play' },
          { id: 'explorer-integration', label: 'Explorer integration', icon: 'folder' },
          { id: 'updates', label: 'Updates', icon: 'download' },
          { id: 'advanced', label: 'Advanced', icon: 'cog' },
        ],
      },
    ])
  })

  test('returns all settings for signed-in users and stable labels', () => {
    expect(availableSettingsSections(true)).toEqual([
      {
        heading: 'Beebeeb',
        items: [
          { id: 'sync', label: 'Sync', icon: 'cloud' },
          { id: 'data-residency', label: 'Data residency', icon: 'shield' },
          { id: 'notifications', label: 'Notifications', icon: 'cog' },
        ],
      },
      {
        heading: 'System',
        items: [
          { id: 'launch', label: 'Launch', icon: 'play' },
          { id: 'explorer-integration', label: 'Explorer integration', icon: 'folder' },
          { id: 'updates', label: 'Updates', icon: 'download' },
          { id: 'advanced', label: 'Advanced', icon: 'cog' },
        ],
      },
    ])
    expect(settingLabel('explorer-integration')).toBe('Explorer integration')
  })
})
