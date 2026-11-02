/**
 * The window navigation: the eleven routes, their sections and icons, and the three ways a
 * route can arrive from outside (the tray event, another window over localStorage, and the
 * stored preference at mount).
 */

// ── Nav structure ───────────────────────────────────────────────────────────

export type NavId =
  | 'home'
  | 'files'
  | 'trash'
  | 'account'
  | 'insights'
  | 'bandwidth'
  | 'selective-sync'
  | 'devices'
  | 'security'
  | 'activity'
  | 'settings'

export const WINDOWS_APP_NAV_EVENT = 'windows-app:navigate'
export const WINDOWS_APP_NAV_STORAGE_KEY = 'bb.windowsApp.nav'
export const NAV_IDS: ReadonlySet<string> = new Set([
  'home',
  'files',
  'trash',
  'account',
  'insights',
  'bandwidth',
  'selective-sync',
  'devices',
  'security',
  'activity',
  'settings',
])

export function navIdFromString(value: string | null): NavId | null {
  return value != null && NAV_IDS.has(value) ? (value as NavId) : null
}

export function consumeStoredNav(): NavId | null {
  try {
    const nav = navIdFromString(window.localStorage.getItem(WINDOWS_APP_NAV_STORAGE_KEY))
    if (nav != null) window.localStorage.removeItem(WINDOWS_APP_NAV_STORAGE_KEY)
    return nav
  } catch {
    return null
  }
}

// `nav` is the query param `show_main_app_window_with_nav` (src-tauri/src/lib.rs)
// sets when opening a fresh window; the localStorage fallback covers callers
// (e.g. the tray's "View all in Beebeeb") that emit the Tauri navigate event
// directly rather than going through that Rust helper, in case the event fires
// before this window's listener has mounted.
export function initialNav(): NavId {
  const params = new URLSearchParams(window.location.search)
  return navIdFromString(params.get('nav')) ?? consumeStoredNav() ?? 'home'
}

// Nav items reachable whether or not the user is signed in. Everything else is
// auth-gated: hidden from the sidebar and replaced by the sign-in prompt.
export const ALWAYS_ACCESSIBLE: ReadonlySet<NavId> = new Set(['home', 'settings'])

export interface NavItem {
  id: NavId
  label: string
  icon: string
}

export const NAV_SECTIONS: Array<{ heading: string; items: NavItem[] }> = [
  {
    heading: 'Beebeeb',
    items: [
      { id: 'home', label: 'Home', icon: 'home' },
      { id: 'files', label: 'Files', icon: 'folder' },
      { id: 'trash', label: 'Trash', icon: 'trash' },
      { id: 'account', label: 'Account', icon: 'user' },
    ],
  },
  {
    heading: 'Usage',
    items: [
      { id: 'insights', label: 'Insights', icon: 'chart' },
      { id: 'bandwidth', label: 'Bandwidth', icon: 'bolt' },
      { id: 'selective-sync', label: 'Selective sync', icon: 'folder' },
    ],
  },
  {
    heading: 'Security',
    items: [
      { id: 'devices', label: 'Devices', icon: 'device' },
      { id: 'security', label: 'Security', icon: 'shield' },
      { id: 'activity', label: 'Activity', icon: 'clock' },
    ],
  },
  {
    heading: 'System',
    items: [{ id: 'settings', label: 'Settings', icon: 'cog' }],
  },
]
