// Browser-only fixture entry. Never imported by src/main.tsx or shipped.
import { createRoot } from 'react-dom/client'
import { CapabilityProvider, CapabilityGate, useCapabilities } from '../../src/capabilities'
import SettingsView, { AdvancedPanel } from '../../src/windows/views/SettingsView'
import { SyncModeStep } from '../../src/WindowsFirstRun'
import { ToastProvider } from '../../src/windows/ui'
import type { DesktopConfig, SyncStatus } from '../../src/desktopApi'

declare global {
  interface Window {
    capabilityFixture: { surface: 'settings' | 'onboarding' | 'advanced' | 'route'; route?: string }
  }
}
const status = { logged_in: true, engine: 'running', syncing: 0 } as SyncStatus
function Surface() {
  const caps = useCapabilities()
  const fixture = window.capabilityFixture
  if (fixture.surface === 'onboarding') return <SyncModeStep onDone={() => { document.body.dataset.continued = 'true' }} />
  if (fixture.surface === 'advanced') return <AdvancedPanel storage={null} config={{ theme: 'system', local_cache_limit_bytes: 0 } as DesktopConfig} onConfigChange={() => {}} />
  if (fixture.surface === 'route') return <CapabilityGate route={fixture.route ?? 'files'}><button type="button">Native action on {caps?.host_os}</button></CapabilityGate>
  return <SettingsView status={status} onOpenSignIn={() => {}} />
}
createRoot(document.getElementById('root')!).render(<ToastProvider><CapabilityProvider><Surface /></CapabilityProvider></ToastProvider>)
