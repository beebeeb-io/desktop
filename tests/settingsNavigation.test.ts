import { describe, expect, test } from 'bun:test'
import {
  ALWAYS_ACCESSIBLE_SETTINGS,
  SETTINGS_SECTIONS,
  defaultSettingsPage,
  filterSettingsSections,
  settingLabel,
} from '../src/windows/settingsNavigation'

describe('settingsNavigation', () => {
  test('keeps the consolidated settings pages grouped under one in-app section', () => {
    expect(SETTINGS_SECTIONS).toEqual([
      {
        heading: 'Beebeeb',
        items: [
          { id: 'sync', label: 'Sync', icon: 'cloud' },
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
    expect(filterSettingsSections(false, '')).toEqual([
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

  test('filters settings by label or section heading and returns stable labels', () => {
    expect(filterSettingsSections(true, 'bee')).toEqual([
      {
        heading: 'Beebeeb',
        items: [
          { id: 'sync', label: 'Sync', icon: 'cloud' },
          { id: 'notifications', label: 'Notifications', icon: 'cog' },
        ],
      },
    ])
    expect(settingLabel('explorer-integration')).toBe('Explorer integration')
  })
})
