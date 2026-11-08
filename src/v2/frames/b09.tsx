// Batch B09's frames (design/desktop-v2/BATCHES.md). Owned by B09 only.
import { createElement } from 'react'
import type { FrameRegistry } from './types'
import { Flyout, type FlyoutHandlers } from '../tray/Flyout'
import { DesktopWithFlyout } from '../tray/Desktop'
import { Explorer } from '../tray/Explorer'
import { SCENARIOS, NOW } from '../tray/fixtures'
import { flyoutView } from '../tray/model'

const none: FlyoutHandlers = {
  onAction: () => undefined, onRow: () => undefined, onOpenBeebeeb: () => undefined,
  onOpenExplorer: () => undefined, onSettings: () => undefined, onViewHistory: () => undefined,
}

function flyout(name: string) {
  const s = SCENARIOS[name]
  return () => (
    <div className="grid h-screen place-items-center bg-paper-3">
      <Flyout view={flyoutView(s.snapshot, s.extras, NOW)} handlers={none} scrollToEnd={s.scrollToEnd} />
    </div>
  )
}

const onDesktop = () => {
  const s = SCENARIOS['up-to-date']
  return <DesktopWithFlyout flyout={<Flyout view={flyoutView(s.snapshot, s.extras, NOW)} handlers={none} />} />
}

export const FRAMES: FrameRegistry = {
  'DW-1201': onDesktop,
  'DW-1202': flyout('up-to-date'),
  'DW-1203': flyout('syncing'),
  'DW-1204': flyout('paused'),
  'DW-1205': flyout('needs-decision'),
  'DW-1206': flyout('offline'),
  'DW-1207': flyout('stopped'),
  'DW-1208': flyout('almost-full'),
  'DW-1209': flyout('storage-full'),
  'DW-1210': flyout('update'),
  'DW-1211': flyout('locked'),
  'DW-1212': flyout('checking'),
  'DW-1213': flyout('no-plan'),
  'DW-1214': flyout('update-required'),
  'DW-1215': flyout('finder-adding'),
  'DW-1216': flyout('finder-failed'),
  'DW-1219': flyout('signed-out'),
  'DW-1220': flyout('last-50'),
  'DW-1221': flyout('longest'),
  'DW-1222': flyout('metered'),
  'DW-1180': () => createElement(Explorer, { selected: 'Contracts' }),
  'DW-1181': () => createElement(Explorer, { selected: 'Contract draft.pdf', menu: 'modern' }),
  'DW-1183': () => createElement(Explorer, { selected: 'Contract draft.pdf', menu: 'classic' }),
  'DW-1182': () => createElement(Explorer, { selected: 'Backups', toast: true }),
}
