// The settings view inside the main window (DW-0701): the settings sidebar with Back to Drive (Esc), the pane at
// the right. Never a separate window. Panes by B13 (panes-today.ts) and B14 (panes-account.ts) register beside ours.
import { useCallback, useEffect, useState, type ComponentType, type ReactElement } from 'react'
import { Icon, type IconName } from '@beebeeb/shared/design'
import { Ctx, type GoOptions, type PaneId } from './context'
import { BASE_PANES, DIALOGS } from './panes'
import { PANES_TODAY, DIALOGS_TODAY } from '../panes-today'
import { PANES_ACCOUNT } from '../panes-account'
import { seed } from '../../../seed'

const SETTINGS: { id: PaneId; label: string; icon: IconName }[] = [
  { id: 'general', label: 'General', icon: 'settings' },
  { id: 'account', label: 'Account', icon: 'users' },
  { id: 'security', label: 'Security', icon: 'shield' },
  { id: 'devices', label: 'Devices and sessions', icon: 'columns' },
  { id: 'notifications', label: 'Notifications', icon: 'bell' },
]
const DRIVE: { id: PaneId; label: string; icon: IconName }[] = [
  { id: 'storage', label: 'Storage', icon: 'cloud' },
  { id: 'backups', label: 'Backups', icon: 'upload' },
  { id: 'sync', label: 'Sync and File Explorer', icon: 'folder' },
  { id: 'thumbnails', label: 'Thumbnails and previews', icon: 'image' },
  { id: 'bandwidth', label: 'Bandwidth', icon: 'activity' },
]
/** Panes that live under another sidebar entry. */
const PARENT: Partial<Record<PaneId, PaneId>> = { about: 'general', compare: 'thumbnails', 'backup-failures': 'backups', device: 'backups', insights: 'storage', recent: 'notifications', location: 'general', diagnostics: 'general' }

function Missing({ pane }: { pane: string }) {
  return <div className="grid flex-1 place-items-center text-[13px] text-ink-3">The {pane} pane is not available in this build.</div>
}

export function SettingsView({ pane: initialPane = 'general', dialog: initialDialog = null, props: initialProps = null, routeKey = null, onGo, onBack, tall, override }: {
  pane?: PaneId
  dialog?: string | null
  /** The first pane's props: a sub-page an opener named (Sign-in activity is Security's `activity` page). */
  props?: Record<string, unknown> | null
  /** The window's route takes the page (`section=settings&page=<page>`, M3e), so a page is addressable and Back
   *  steps through the pages. Without it (a frame) the view keeps the page itself. */
  onGo?: (pane: PaneId, opts?: GoOptions) => void
  /** The route's page word: when it changes, the view opens the page, dialog and props it is given (the sidebar stays
   *  mounted, so its focus stays where the person clicked). */
  routeKey?: string | null
  onBack?: () => void
  tall?: boolean
  /** A frame's variant of the first pane (a pane state the user reaches by acting). */
  override?: ReactElement
}) {
  const [pane, setPane] = useState<PaneId>(initialPane)
  const [dialog, setDialog] = useState<string | null>(() => seed('settings.dialog', initialDialog))
  // The live state route (../../../seed.ts): the first pane's props, read once, so the pane keeps them until it changes.
  // Leaving that pane drops them: coming back opens the pane plain.
  const [seeded, setSeeded] = useState(() => ({ pane: initialPane, props: seed<Record<string, unknown> | null>('settings.paneProps', initialProps) }))
  useEffect(() => { if (seeded.props && pane !== seeded.pane) setSeeded({ pane: seeded.pane, props: null }) }, [pane, seeded])
  // A new page from the route (another page clicked, Back, an opener): taken while rendering, so no frame shows the old.
  const [route, setRoute] = useState(routeKey)
  if (route !== routeKey) {
    setRoute(routeKey)
    setPane(initialPane)
    setDialog(initialDialog)
    setSeeded({ pane: initialPane, props: initialProps })
  }
  const close = useCallback(() => setDialog(null), [])
  const open = useCallback((d: string) => setDialog(d), [])
  const go = useCallback((to: PaneId, opts: GoOptions = {}) => {
    // The page on screen opens the dialog or sub-page itself; another page goes through the route when there is one.
    if (to === pane || !onGo) {
      setPane(to)
      setDialog(opts.dialog ?? null)
      if (opts.props) setSeeded({ pane: to, props: opts.props })
      return
    }
    onGo(to, opts)
  }, [pane, onGo])

  // Esc is Back to Drive; an open dialog or pop-up takes the key first (they stop it in the capture phase).
  useEffect(() => {
    const key = (e: KeyboardEvent) => { if (e.key === 'Escape' && !e.defaultPrevented) onBack?.() }
    document.addEventListener('keydown', key)
    return () => document.removeEventListener('keydown', key)
  }, [onBack])

  const panes: Partial<Record<PaneId, ComponentType>> = { ...BASE_PANES, ...PANES_TODAY, ...PANES_ACCOUNT }
  const Pane = panes[pane] as ComponentType<Record<string, unknown>> | undefined
  const paneProps = seeded.props && pane === seeded.pane ? seeded.props : {}
  const Dlg = dialog ? { ...DIALOGS, ...DIALOGS_TODAY }[dialog.split(':')[0]] : undefined
  const active = PARENT[pane] ?? pane

  const item = (i: { id: PaneId; label: string; icon: IconName }) => (
    <li key={i.id}>
      <a href={`?section=settings&page=${i.id}`} aria-current={active === i.id ? 'page' : undefined}
        onClick={(e) => { e.preventDefault(); go(i.id) }}
        className={`flex h-8 items-center gap-2.5 rounded-md px-2.5 text-[13px] ${active === i.id ? 'bg-paper-3 font-medium text-ink' : 'text-ink-2 hover:bg-paper-2'}`}>
        <Icon name={i.icon} size={15} />
        {i.label}
      </a>
    </li>
  )

  return (
    <Ctx.Provider value={{ pane, go, dialog, open, close }}>
      <div className={`flex ${tall ? 'min-h-screen' : 'h-full min-h-0 flex-1'} bg-paper`} data-testid="settings-view">
        <nav aria-label="Settings" className="flex w-[232px] shrink-0 flex-col gap-1 border-r border-line bg-paper-2 p-2">
          <a href="?section=files" onClick={(e) => { e.preventDefault(); onBack?.() }}
            className="mb-2 flex h-8 items-center gap-2 rounded-md px-2.5 text-[13px] text-ink-2 hover:bg-paper-3">
            <Icon name="chevron-right" size={14} className="rotate-180" />
            <span className="flex-1">Back to Drive</span>
            <kbd className="font-mono text-[11px] text-ink-3">esc</kbd>
          </a>
          <div className="px-2.5 pb-0.5 pt-1 text-[11px] font-medium uppercase tracking-wide text-ink-3">Settings</div>
          <ul className="flex flex-col gap-0.5" data-testid="settings-nav">{SETTINGS.map(item)}</ul>
          <div className="px-2.5 pb-0.5 pt-3 text-[11px] font-medium uppercase tracking-wide text-ink-3">Drive</div>
          <ul className="flex flex-col gap-0.5" data-testid="drive-nav">{DRIVE.map(item)}</ul>
        </nav>
        <div className="flex min-w-0 flex-1 flex-col overflow-y-auto">{override && pane === initialPane ? override : Pane ? <Pane key={`${route ?? ''}:${pane}`} {...paneProps} /> : <Missing pane={pane} />}</div>
        {Dlg ? <Dlg arg={dialog!.split(':')[1]} /> : null}
      </div>
    </Ctx.Provider>
  )
}
