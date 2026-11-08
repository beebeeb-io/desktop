// B13 registers the panes for what the app does today here (one file, theirs).
// The first block replaces B12's pane of the same id (a richer version); the rest are new panes.
import { createElement, useEffect, type ComponentType } from 'react'
import type { PaneId } from './shell/context'
import { useSettings } from './shell/context'
import { Bandwidth } from './today/bandwidth'
import { Insights } from './today/insights'
import { Recent } from './today/recent'
import { Location } from './today/location'
import { Diagnostics } from './today/diagnostics'
import { AboutToday, ReleaseNotesDialog } from './today/about'
import { SyncToday, RepairAlert } from './today/sync'
import { KeepDialog, KeepConfirm } from './today/keep'
import { InterceptLinks } from './today/intercept'

export const PANES_TODAY: Partial<Record<PaneId, ComponentType>> = {
  bandwidth: Bandwidth,
  insights: Insights,
  recent: Recent,
  location: Location,
  diagnostics: Diagnostics,
  about: AboutToday,
  sync: SyncToday,
  // B12's own panes, with their links to today's pages wired (Recent notifications, Diagnostics).
  general: InterceptLinks.general,
  notifications: InterceptLinks.notifications,
}

/** A B12 dialog that is really a page now (Storage insights, Storage location): open the page instead. */
function Redirect({ to }: { to: PaneId }) {
  const { go, close } = useSettings()
  useEffect(() => { close(); go(to) }, [close, go, to])
  return null
}
const InsightsRedirect = () => createElement(Redirect, { to: 'insights' })
const LocationRedirect = () => createElement(Redirect, { to: 'location' })

export const DIALOGS_TODAY: Record<string, ComponentType<{ arg?: string }>> = {
  insights: InsightsRedirect,
  region: LocationRedirect,
  repair: RepairAlert,
  keep: KeepDialog,
  'keep-confirm': KeepConfirm,
  'release-notes': ReleaseNotesDialog,
}
