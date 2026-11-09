// Desktop v2 routing: one window, one section at a time, with back and forward (DW-0401 toolbar).
// The route lives in the URL query so a headless browser can open any state directly:
//   ?window=main-app&platform=windows&section=files&folder=<id>&source=fixtures&scenario=offline&frame=DW-0401
// Section components live in src/v2/sections/<section>/; each batch owns its directories (BATCHES.md).

import { useCallback, useEffect, useState } from 'react'
import { hasTauri } from './source'

export const SECTIONS = [
  'files',
  'shared',
  'photos',
  'backups',
  'trash',
  'sync-history',
  'available-offline',
  'search',
  'settings',
  'open',
] as const

export type SectionId = (typeof SECTIONS)[number]

export interface Route {
  section: SectionId
  /** Files and Backups: the open folder; Open: the file. Opaque DriveSource ids. */
  id: string | null
  /** Settings: the page (DW-0701 and on), `page=` in the URL. Open: viewer, editor or office. Elsewhere `sub=`. */
  sub: string | null
}

export function isSection(value: string | null): value is SectionId {
  return value != null && (SECTIONS as readonly string[]).includes(value)
}

/** The URL parameter that holds a section's `sub`: Settings' pages are addressable as `section=settings&page=<page>`. */
const subParam = (section: SectionId) => (section === 'settings' ? 'page' : 'sub')

/**
 * Where the native openers point (M3e). Rust opens or focuses this window with `nav=<word>` in the URL, or sends the
 * same word as `windows-app:navigate` to a window that is open (`show_main_app_window_with_nav`): the tray menu's
 * Settings, the flyout's buttons, the setup window's rows and the View menu. A word is a section, `section/sub`, or one
 * of the legacy words below; anything else is no route (the window stays where it is).
 */
const NAV_WORDS: Record<string, Partial<Route> & { section: SectionId }> = {
  settings: { section: 'settings', sub: 'general' },
  activity: { section: 'sync-history' },
  problems: { section: 'sync-history', sub: 'problems' },
  // A decision waits: Files opens the "Two versions" dialog by itself (DW-0429).
  review: { section: 'files' },
}

export function routeFromNav(nav: string | null): Route | null {
  if (!nav) return null
  const word = NAV_WORDS[nav]
  if (word) return { id: null, sub: null, ...word }
  const [section, sub, ...rest] = nav.split('/')
  if (!isSection(section) || rest.length || (sub !== undefined && !/^[a-z][a-z-]{0,39}$/.test(sub))) return null
  return { section, id: null, sub: section === 'settings' ? sub ?? 'general' : sub ?? null }
}

export function routeFromSearch(search: string): Route {
  const params = new URLSearchParams(search)
  const section = params.get('section')
  if (!isSection(section)) {
    const fromNav = routeFromNav(params.get('nav'))
    if (fromNav) return fromNav
  }
  const at: SectionId = isSection(section) ? section : 'files'
  return {
    section: at,
    id: params.get('id'),
    // `sub=` still opens a settings page, so an older link keeps working.
    sub: params.get(subParam(at)) ?? (at === 'settings' ? params.get('sub') : null),
  }
}

export function searchFromRoute(route: Route, current: string): string {
  const params = new URLSearchParams(current)
  params.set('section', route.section)
  // The native opener's word has done its work once the window holds a route of its own.
  params.delete('nav')
  if (route.id) params.set('id', route.id)
  else params.delete('id')
  params.delete('sub')
  params.delete('page')
  if (route.sub) params.set(subParam(route.section), route.sub)
  return `?${params.toString()}`
}

/** The settings page for the section on screen (lead ruling M3a item 2): Backups opens Settings › Backups, a section
 *  without a page of its own opens General. */
export function settingsRouteFor(section: SectionId): Partial<Route> & { section: SectionId } {
  return { section: 'settings', sub: section === 'backups' ? 'backups' : 'general' }
}

/** Settings without a page (the sidebar gear, Ctrl+,) opens the page for the section on screen; a named page stays. */
function withSettingsPage(next: Partial<Route> & { section: SectionId }): Partial<Route> & { section: SectionId } {
  if (next.section !== 'settings' || next.sub) return next
  return { ...next, ...settingsRouteFor(routeFromSearch(window.location.search).section) }
}

export interface Navigation {
  route: Route
  go: (route: Partial<Route> & { section: SectionId }) => void
  back: () => void
  forward: () => void
}

/**
 * Navigation from outside a section (the window-wide banners, M3a): the same history entry `go` makes, announced as a
 * popstate so the window's useNavigation follows it.
 */
export function navigateTo(next: Partial<Route> & { section: SectionId }): void {
  const full: Route = { id: null, sub: null, ...withSettingsPage(next) }
  window.history.pushState(null, '', searchFromRoute(full, window.location.search))
  window.dispatchEvent(new PopStateEvent('popstate'))
}

/** The event Rust sends a window that is already open (`show_main_app_window_with_nav`), with the nav word. */
export const NATIVE_NAV_EVENT = 'windows-app:navigate'

let driveRoute: Route | null = null
/** The last route outside Settings: Back to Drive and Esc return the window as it was (DW-0701#0), else Files. */
export function lastDriveRoute(): Route {
  return driveRoute ?? { section: 'files', id: null, sub: null }
}

/** History-backed navigation: Back and Forward (Alt+Left, Alt+Right, the toolbar arrows) are the browser's own. */
export function useNavigation(): Navigation {
  const [route, setRoute] = useState<Route>(() => routeFromSearch(window.location.search))
  if (route.section !== 'settings' && route.section !== 'open') driveRoute = route

  useEffect(() => {
    const onPop = () => setRoute(routeFromSearch(window.location.search))
    window.addEventListener('popstate', onPop)
    return () => window.removeEventListener('popstate', onPop)
  }, [])

  // The native openers (the tray menu, the flyout, the setup window) reach a window that is open through this event.
  useEffect(() => {
    if (!hasTauri()) return
    let off: (() => void) | undefined
    let live = true
    void import('@tauri-apps/api/event').then(({ listen }) => listen<string>(NATIVE_NAV_EVENT, (e) => {
      const next = routeFromNav(typeof e.payload === 'string' ? e.payload : null)
      if (next) navigateTo(next)
    })).then((stop) => { if (live) off = stop; else stop() })
    return () => { live = false; off?.() }
  }, [])

  const go = useCallback((next: Partial<Route> & { section: SectionId }) => {
    const full: Route = { id: null, sub: null, ...withSettingsPage(next) }
    window.history.pushState(null, '', searchFromRoute(full, window.location.search))
    setRoute(full)
  }, [])

  const back = useCallback(() => window.history.back(), [])
  const forward = useCallback(() => window.history.forward(), [])

  return { route, go, back, forward }
}
