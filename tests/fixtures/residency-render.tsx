// Browser fixtures use shipping components with synthetic IPC, never native proof.
import { createRoot } from 'react-dom/client'
import Onboarding from '../../src/Onboarding'
import WindowsApp from '../../src/WindowsApp'
import SettingsView from '../../src/windows/views/SettingsView'
import { CapabilityProvider } from '../../src/capabilities'
import { ToastProvider } from '../../src/windows/ui'
import type { SyncStatus } from '../../src/desktopApi'

const surface = new URLSearchParams(window.location.search).get('surface')
const status = { logged_in: true, engine: 'running', syncing: 0 } as SyncStatus
createRoot(document.getElementById('root')!).render(
  <ToastProvider><CapabilityProvider>
    {surface === 'onboarding' ? <Onboarding /> : surface === 'settings'
      ? <SettingsView status={status} onOpenSignIn={() => {}} /> : <WindowsApp />}
  </CapabilityProvider></ToastProvider>,
)
