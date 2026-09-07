import type { PlatformName } from '../platform'

export type SettingsNavId =
  | 'sync'
  | 'data-residency'
  | 'notifications'
  | 'launch'
  | 'explorer-integration'
  | 'updates'
  | 'advanced'

/**
 * Label for the OS shell-integration nav item/page — "Finder" on macOS,
 * "Explorer" on Windows. `platform === null` means the real host platform
 * hasn't resolved yet (see `usePlatform()` in `platform.ts`); callers in that
 * state must pass `null` rather than a guessed fallback, so the sidebar shows
 * a neutral "Shell integration" instead of flipping from "Explorer
 * integration" to "Finder integration" once the real OS resolves (PR #34
 * review).
 */
export function shellIntegrationLabel(platform: PlatformName | null): string {
  if (platform === 'macos') return 'Finder integration'
  if (platform === 'linux') return 'File manager integration'
  if (platform === 'windows') return 'Explorer integration'
  return 'Shell integration'
}

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

export function settingLabel(id: SettingsNavId, platform: PlatformName | null = 'windows'): string {
  if (id === 'explorer-integration') return shellIntegrationLabel(platform)
  return SETTINGS_SECTIONS.flatMap((section) => section.items).find((item) => item.id === id)?.label ?? id
}

function withPlatformLabels(sections: SettingsNavSection[], platform: PlatformName | null): SettingsNavSection[] {
  return sections.map((section) => ({
    ...section,
    items: section.items.map((item) =>
      item.id === 'explorer-integration' ? { ...item, label: shellIntegrationLabel(platform) } : item,
    ),
  }))
}

export function availableSettingsSections(loggedIn: boolean, platform: PlatformName | null = 'windows'): SettingsNavSection[] {
  if (loggedIn) return withPlatformLabels(SETTINGS_SECTIONS, platform)

  return withPlatformLabels(
    SETTINGS_SECTIONS.map((section) => ({
      ...section,
      items: section.items.filter((item) => ALWAYS_ACCESSIBLE_SETTINGS.has(item.id)),
    })).filter((section) => section.items.length > 0),
    platform,
  )
}
