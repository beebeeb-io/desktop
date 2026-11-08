// Settings (B12 shell; B13 and B14 add panes). Ctrl+, lands here; Back to Drive and Esc return to Files.
import { SettingsView } from './shell/SettingsView'
import type { PaneId } from './shell/context'
import type { SectionProps } from '../types'

const KNOWN: PaneId[] = ['general', 'about', 'account', 'security', 'devices', 'notifications', 'storage', 'backups', 'sync', 'thumbnails', 'compare', 'backup-failures', 'device', 'bandwidth', 'insights', 'recent', 'location', 'diagnostics']

export default function Section({ nav }: SectionProps) {
  const sub = nav.route.sub as PaneId | null
  const pane = sub && KNOWN.includes(sub) ? sub : 'general'
  return <SettingsView key={pane} pane={pane} onBack={() => nav.go({ section: 'files', id: null, sub: null })} />
}
