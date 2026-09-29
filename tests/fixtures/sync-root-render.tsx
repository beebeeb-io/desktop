// Browser fixture only; not imported by the shipping entrypoint.
import { createRoot } from 'react-dom/client'
import WindowsApp from '../../src/WindowsApp'
import WindowsTray from '../../src/WindowsTray'
import { CapabilityProvider } from '../../src/capabilities'
import { ToastProvider } from '../../src/windows/ui'

createRoot(document.getElementById('root')!).render(
  <ToastProvider><CapabilityProvider>
    {new URLSearchParams(window.location.search).get('surface') === 'tray' ? <WindowsTray /> : <WindowsApp />}
  </CapabilityProvider></ToastProvider>,
)
