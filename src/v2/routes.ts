// Desktop v2 routing: one window, one section at a time, with back and forward (DW-0401 toolbar).
// The route lives in the URL query so a headless browser can open any state directly:
//   ?window=main-app&platform=windows&section=files&folder=<id>&source=fixtures&scenario=offline&frame=DW-0401
// Section components live in src/v2/sections/<section>/; each batch owns its directories (BATCHES.md).

import { useCallback, useEffect, useState } from 'react'

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
  /** Settings: the pane (DW-0701 and on). Open: viewer, editor or office. */
  sub: string | null
}

export function isSection(value: string | null): value is SectionId {
  return value != null && (SECTIONS as readonly string[]).includes(value)
}

export function routeFromSearch(search: string): Route {
  const params = new URLSearchParams(search)
  const section = params.get('section')
  return {
    section: isSection(section) ? section : 'files',
    id: params.get('id'),
    sub: params.get('sub'),
  }
}

export function searchFromRoute(route: Route, current: string): string {
  const params = new URLSearchParams(current)
  params.set('section', route.section)
  if (route.id) params.set('id', route.id)
  else params.delete('id')
  if (route.sub) params.set('sub', route.sub)
  else params.delete('sub')
  return `?${params.toString()}`
}

export interface Navigation {
  route: Route
  go: (route: Partial<Route> & { section: SectionId }) => void
  back: () => void
  forward: () => void
}

/** History-backed navigation: Back and Forward (Alt+Left, Alt+Right, the toolbar arrows) are the browser's own. */
export function useNavigation(): Navigation {
  const [route, setRoute] = useState<Route>(() => routeFromSearch(window.location.search))

  useEffect(() => {
    const onPop = () => setRoute(routeFromSearch(window.location.search))
    window.addEventListener('popstate', onPop)
    return () => window.removeEventListener('popstate', onPop)
  }, [])

  const go = useCallback((next: Partial<Route> & { section: SectionId }) => {
    const full: Route = { id: null, sub: null, ...next }
    window.history.pushState(null, '', searchFromRoute(full, window.location.search))
    setRoute(full)
  }, [])

  const back = useCallback(() => window.history.back(), [])
  const forward = useCallback(() => window.history.forward(), [])

  return { route, go, back, forward }
}
