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
