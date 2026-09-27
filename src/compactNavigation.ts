import type { DesktopPlatform } from './desktopApi'

export type CompactPage =
  | 'status'
  | 'finder'
  | 'selective-sync'
  | 'versions'
  | 'account'
  | 'bandwidth'
  | 'notifications'

export interface CompactNavItem {
  id: CompactPage
  label: string
  icon: string
}

const COMPACT_PAGE_IDS: ReadonlySet<string> = new Set([
  'status',
  'finder',
  'selective-sync',
  'versions',
  'account',
  'bandwidth',
  'notifications',
])

export const COMPACT_NAV_ITEMS: ReadonlyArray<CompactNavItem> = [
  { id: 'status', label: 'Status', icon: '●' },
  { id: 'finder', label: 'Finder location', icon: '⌂' },
  { id: 'selective-sync', label: 'Selective sync', icon: '↓' },
  { id: 'versions', label: 'Versions & conflicts', icon: '⎇' },
  { id: 'account', label: 'Account & security', icon: '⌘' },
  { id: 'bandwidth', label: 'Network', icon: '⇅' },
  { id: 'notifications', label: 'Notifications', icon: '!' },
]

export function compactPageFromString(value: string | null): CompactPage | null {
  if (value === 'shared') return 'finder'
  return value != null && COMPACT_PAGE_IDS.has(value) ? (value as CompactPage) : null
}

// App.tsx is the SAME component tree on macOS and Linux (main.tsx only tags
// the URL with ?platform=macos for macOS, and that tag toggles a CSS class,
// never any rendered text — see main.tsx's doc comment). A literal "macOS
// Drive" string in that shared tree was therefore wrong on every shipped
// Linux build. Fed by the `desktop_platform` IPC command's `DesktopPlatform`
// result (task 1546 finding 5).
export function brandSubtitleForPlatform(platform: DesktopPlatform): string {
  switch (platform) {
    case 'macos':
      return 'macOS Drive'
    case 'windows':
      return 'Windows Drive'
    case 'linux':
      return 'Linux Drive'
    case 'unknown':
      return 'Drive'
  }
}
