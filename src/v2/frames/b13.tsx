// Batch B13's frames (design/desktop-v2/BATCHES.md): settings, what the desktop app does today.
import type { ReactElement } from 'react'
import { CaptionBar } from '../chrome/CaptionBar'
import { SettingsView } from '../sections/settings/shell/SettingsView'
import type { PaneId } from '../sections/settings/shell/context'
import { Bandwidth, TEST_STATES } from '../sections/settings/today/bandwidth'
import { Recent } from '../sections/settings/today/recent'
import { Insights } from '../sections/settings/today/insights'
import { Location } from '../sections/settings/today/location'
import { Diagnostics } from '../sections/settings/today/diagnostics'
import { AboutToday } from '../sections/settings/today/about'
import { SyncToday } from '../sections/settings/today/sync'
import { AutoClick } from '../sections/settings/today/ui'
import { presetKeep } from '../sections/settings/today/keep'
import type { FrameRegistry } from './types'

/** The window with the settings view in it. `tall`: drawn taller than the window so the whole section shows. */
function win(pane: PaneId, opts: { dialog?: string; tall?: boolean; override?: ReactElement; click?: string } = {}) {
  return () => {
    const view = <SettingsView pane={pane} dialog={opts.dialog ?? null} tall={opts.tall} override={opts.override} />
    return (
      <div className={`flex ${opts.tall ? 'min-h-screen' : 'h-screen'} flex-col bg-paper font-sans text-ink`}>
        <CaptionBar title="Settings - Beebeeb" />
        <div className="flex min-h-0 flex-1 flex-col">{opts.click ? <AutoClick label={opts.click}>{view}</AutoClick> : view}</div>
      </div>
    )
  }
}

export const FRAMES: FrameRegistry = {
  // Bandwidth and the speed test
  'DW-0785': win('bandwidth', { tall: true, override: <Bandwidth hover={44} /> }),
  'DW-0785b': win('bandwidth', { override: <Bandwidth variant="idle" /> }),
  'DW-0785c': win('bandwidth', { tall: true, override: <Bandwidth variant="limited" /> }),
  'DW-0785d': win('bandwidth', { override: <Bandwidth scrollTo="limits" limitPopup="upload" /> }),
  'DW-0785e': win('bandwidth', { override: <Bandwidth variant="empty" /> }),
  'DW-0785f': win('bandwidth', { tall: true, override: <Bandwidth variant="metered" /> }),
  'DW-0786': win('bandwidth', { override: <Bandwidth scrollTo="test" test={TEST_STATES.running} /> }),
  'DW-0786b': win('bandwidth', { override: <Bandwidth scrollTo="test" test={TEST_STATES.downloads} /> }),
  'DW-0786c': win('bandwidth', { override: <Bandwidth scrollTo="test" test={TEST_STATES.done} /> }),
  'DW-0786d': win('bandwidth', { override: <Bandwidth scrollTo="test" test={TEST_STATES.error} /> }),
  // Recent notifications
  'DW-0791': win('recent', { tall: true }),
  'DW-0791b': win('recent', { override: <Recent empty /> }),
  // Storage insights
  'DW-0783': win('insights', { tall: true }),
  'DW-0783b': win('insights', { override: <Insights state="loading" /> }),
  'DW-0783c': win('insights', { override: <Insights state="error" /> }),
  'DW-0783d': win('insights', { override: <Insights state="empty" /> }),
  // Keep on this PC
  'DW-0784': () => { presetKeep(); return win('sync', { dialog: 'keep:changed' })() },
  'DW-0784b': () => { presetKeep(); return win('sync', { dialog: 'keep-confirm:changed' })() },
  'DW-0784c': win('sync', { dialog: 'keep:error' }),
  'DW-0784d': win('sync', { dialog: 'keep:empty' }),
  // General: theme, storage location, diagnostics
  'DW-0787': win('general', { click: 'Theme' }),
  'DW-0788': win('location', { override: <Location /> }),
  'DW-0788b': win('location', { override: <Location chosen="de" toast /> }),
  'DW-0788c': win('location', { override: <Location chosen="fi" unavailable /> }),
  'DW-0788d': win('location', { override: <Location error /> }),
  // About: updates
  'DW-0789': win('about', { tall: true, override: <AutoClick label="Release channel"><AboutToday /></AutoClick> }),
  'DW-0789b': win('about', { dialog: 'release-notes' }),
  // Diagnostics
  'DW-0790': win('diagnostics', { tall: true, override: <Diagnostics /> }),
  'DW-0790b': win('diagnostics', { tall: true, override: <Diagnostics saved /> }),
  // Sync and File Explorer: repair, metered wait
  'DW-0746c': win('sync', { dialog: 'repair' }),
  'DW-0746d': win('sync', { override: <SyncToday repair="failed" /> }),
  'DW-0746e': win('sync', { override: <SyncToday metered /> }),
}
