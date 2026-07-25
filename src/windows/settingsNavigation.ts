export type SettingsNavId =
  | 'sync'
  | 'data-residency'
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

export interface SettingsNavSection {
  heading: string
  items: SettingsNavItem[]
}

export const ALWAYS_ACCESSIBLE_SETTINGS: ReadonlySet<SettingsNavId> = new Set([
  'launch',
  'explorer-integration',
  'updates',
  'advanced',
])

export const SETTINGS_SECTIONS: SettingsNavSection[] = [
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
]

export function defaultSettingsPage(loggedIn: boolean): SettingsNavId {
  return loggedIn ? 'sync' : 'launch'
}

export function settingLabel(id: SettingsNavId): string {
  return SETTINGS_SECTIONS.flatMap((section) => section.items).find((item) => item.id === id)?.label ?? id
}

export function availableSettingsSections(loggedIn: boolean): SettingsNavSection[] {
  if (loggedIn) return SETTINGS_SECTIONS

  return SETTINGS_SECTIONS.map((section) => ({
    ...section,
    items: section.items.filter((item) => ALWAYS_ACCESSIBLE_SETTINGS.has(item.id)),
  })).filter((section) => section.items.length > 0)
}
