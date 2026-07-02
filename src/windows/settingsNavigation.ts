export type SettingsNavId =
  | 'sync'
  | 'notifications'
  | 'launch'
  | 'explorer-integration'
  | 'updates'
  | 'advanced'

export interface SettingsNavItem {
  id: SettingsNavId
  label: string
  icon: string
}

export const ALWAYS_ACCESSIBLE_SETTINGS: ReadonlySet<SettingsNavId> = new Set([
  'launch',
  'explorer-integration',
  'updates',
  'advanced',
])

export const SETTINGS_SECTIONS: Array<{ heading: string; items: SettingsNavItem[] }> = [
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
]

export function defaultSettingsPage(loggedIn: boolean): SettingsNavId {
  return loggedIn ? 'sync' : 'launch'
}

export function settingLabel(id: SettingsNavId): string {
  return SETTINGS_SECTIONS.flatMap((section) => section.items).find((item) => item.id === id)?.label ?? id
}

export function filterSettingsSections(
  loggedIn: boolean,
  searchQuery: string,
): Array<{ heading: string; items: SettingsNavItem[] }> {
  const query = searchQuery.trim().toLowerCase()
  return SETTINGS_SECTIONS.map((section) => ({
    ...section,
    items: section.items.filter(
      (item) =>
        (loggedIn || ALWAYS_ACCESSIBLE_SETTINGS.has(item.id)) &&
        (
          !query ||
          item.label.toLowerCase().includes(query) ||
          section.heading.toLowerCase().includes(query)
        ),
    ),
  })).filter((section) => section.items.length > 0)
}
