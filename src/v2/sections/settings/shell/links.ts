// Where a settings link goes (M3e). A page word (`section=settings&page=<page>`) is a pane, or a dialog over a page or a
// sub-page of one; another section's link moves the window there; the web, mail and Windows' own settings open in the
// OS (../../../external.ts). No link in Settings navigates the web view itself: that would leave the app.
import { isExternal, openExternal } from '../../../external'
import { navigateTo, routeFromSearch } from '../../../routes'
import type { CommandResult } from '../../../../desktopApi'
import { PANE_IDS, type GoOptions, type PaneId } from './context'

/** Where a page word lands. */
export interface SettingsPage { pane: PaneId; dialog?: string; props?: Record<string, unknown> }

const ALIASES: Record<string, SettingsPage> = {
  // Help and report a problem: the dialog over General (DW-0782; Cancel returns to General, DW-0782#1).
  report: { pane: 'general', dialog: 'report' },
  // Free up space… from a file too large to preview (DW-1517 → DW-0780, M3d): the dialog over Storage.
  'free-up': { pane: 'storage', dialog: 'free-up' },
  // Mail Backup's own page is a proposal the board leaves out (DW-0740); its row and its switch are in Backups.
  'mail-backup': { pane: 'backups' },
  // Sign-in activity is Security's page (DW-A77), reached from Recent notifications (DW-0791#3).
  'signin-activity': { pane: 'security', props: { init: { page: 'activity' } } },
}

/** Pages that need a device (Compare, a device's own page): reached from their list, never named by an opener. */
const NEEDS_A_DEVICE: readonly PaneId[] = ['compare', 'device']

/** Every page word an opener may name: the panes, and the words for a dialog or a sub-page. Rust's `SETTINGS_PAGES`
 *  (src-tauri/src/tray_window.rs) is the same list. */
export const PAGE_WORDS: readonly string[] = [...PANE_IDS.filter((p) => !NEEDS_A_DEVICE.includes(p)), ...Object.keys(ALIASES)]

export function settingsPage(word: string | null | undefined): SettingsPage {
  if (word && (PANE_IDS as readonly string[]).includes(word)) return { pane: word as PaneId }
  // Own keys only: an inherited one ("constructor") is not a page (#274 review).
  return (word && Object.prototype.hasOwnProperty.call(ALIASES, word) && ALIASES[word]) || { pane: 'general' }
}

/**
 * Follow `href` from a settings page. Resolves with the OS's answer for an outside address (so a refusal can be said),
 * or null when the window moved itself. False: not a link this handles (the browser keeps it).
 */
export function followHref(href: string, go: (pane: PaneId, opts?: GoOptions) => void): Promise<CommandResult<void>> | null | false {
  if (isExternal(href)) return openExternal(href)
  if (!href.startsWith('?')) return false
  const params = new URLSearchParams(href)
  if (params.get('section') === 'settings') {
    // Notifications' "Recent notifications" row was written as `recent=1` before the page was addressable.
    const page = settingsPage(params.get('recent') ? 'recent' : (params.get('page') ?? params.get('sub')))
    go(page.pane, { dialog: page.dialog, props: page.props })
    return null
  }
  navigateTo(routeFromSearch(href))
  return null
}
