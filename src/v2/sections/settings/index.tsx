// Settings (B12 shell; B13 and B14 add panes). Ctrl+, lands here; Back to Drive and Esc return to Files.
import { SettingsView } from './shell/SettingsView'
import type { PaneId } from './shell/context'
import type { SectionProps } from '../types'

const KNOWN: PaneId[] = ['general', 'about', 'account', 'security', 'devices', 'notifications', 'storage', 'backups', 'sync', 'thumbnails', 'compare', 'backup-failures', 'device', 'bandwidth', 'insights', 'recent', 'location', 'diagnostics']

/** Openers outside Settings that land on a dialog: Report a problem (DW-1518 → DW-0782) and Free up space… (DW-1517 →
 *  DW-0780). Each names its pane and the dialog open on it. */
const DIALOGS: Record<string, [PaneId, string]> = { report: ['general', 'report'], 'free-up': ['storage', 'free-up'] }

export default function Section({ nav }: SectionProps) {
  const sub = nav.route.sub
  const [pane, dialog] = sub && Object.hasOwn(DIALOGS, sub) ? DIALOGS[sub] : [sub && KNOWN.includes(sub as PaneId) ? (sub as PaneId) : 'general', null]
  return <SettingsView key={`${pane}:${dialog}`} pane={pane} dialog={dialog} onBack={() => nav.go({ section: 'files', id: null, sub: null })} />
}
