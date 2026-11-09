// Settings (B12 shell; B13 and B14 add panes). Each page is addressable as `section=settings&page=<page>` (M3e,
// lead ruling): the gear and Ctrl+, open the page for the section on screen, other openers name their page, and
// Back to Drive or Esc return the window as it was.
import { useEffect } from 'react'
import { SettingsView } from './shell/SettingsView'
import type { PaneId } from './shell/context'
import { settingsPage } from './shell/links'
import { lastDriveRoute } from '../../routes'
import type { SectionProps } from '../types'

/** What the next page opens with, handed over across the route change (a dialog, a sub-page's props). */
let handoff: { pane: PaneId; dialog?: string; props?: Record<string, unknown> } | null = null

export default function Section({ nav }: SectionProps) {
  const page = settingsPage(nav.route.sub)
  const given = handoff?.pane === page.pane ? handoff : null
  // Read while the page renders (StrictMode renders twice), dropped once it is on screen: Back to it later opens it plain.
  useEffect(() => { handoff = null }, [nav.route.sub])
  return (
    <SettingsView key={`${nav.route.sub ?? ''}`} pane={page.pane}
      dialog={given?.dialog ?? page.dialog ?? null} props={given?.props ?? page.props ?? null}
      onGo={(pane, opts) => { handoff = { pane, ...opts }; nav.go({ section: 'settings', sub: pane }) }}
      onBack={() => nav.go(lastDriveRoute())} />
  )
}
