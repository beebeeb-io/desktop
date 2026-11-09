// Settings (B12 shell; B13 and B14 add panes). Ctrl+, lands here; Back to Drive and Esc return to Files.
import { SettingsView } from './shell/SettingsView'
import type { PaneId } from './shell/context'
import type { SectionProps } from '../types'

const KNOWN: PaneId[] = ['general', 'about', 'account', 'security', 'devices', 'notifications', 'storage', 'backups', 'sync', 'thumbnails', 'compare', 'backup-failures', 'device', 'bandwidth', 'insights', 'recent', 'location', 'diagnostics']

export default function Section({ nav }: SectionProps) {
  const sub = nav.route.sub as PaneId | null
  const pane = sub && KNOWN.includes(sub) ? sub : 'general'
  // `report`: General with the problem report open (DW-0782), for Report a problem outside Settings (DW-1518).
  const dialog = nav.route.sub === 'report' ? 'report' : null
  return <SettingsView key={`${pane}:${dialog}`} pane={pane} dialog={dialog} onBack={() => nav.go({ section: 'files', id: null, sub: null })} />
}
