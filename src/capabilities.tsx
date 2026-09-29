import { createContext, useContext, useEffect, useState, type ReactNode } from 'react'
import { command, openUrl } from './desktopApi'

/** Wire shape serialized by desktop_capabilities.rs; fixtures are checked by both suites. */
export interface DesktopCapabilities {
  host_os: 'macos' | 'windows' | 'linux' | 'unknown'
  filesystem_integration: 'file_provider' | 'cloud_files' | 'none'
  on_demand: boolean
  native_credentials: boolean
  tray: 'available' | 'unavailable' | 'unknown'
  sharing: 'recipient_read_only' | 'web_only'
  install_format: 'app' | 'nsis' | 'msi' | 'appimage' | 'deb' | 'rpm' | 'unknown'
  update_format: 'app_bundle' | 'nsis' | 'msi' | 'appimage' | 'package_manager' | 'manual'
  sync_mode_selection: boolean
  metered_sync_control: boolean
  sync_overlay_control: boolean
  hydrate_all_control: boolean
  cache_limit_control: boolean
}

export const CapabilityContext = createContext<DesktopCapabilities | null>(null)
export const useCapabilities = () => useContext(CapabilityContext)

export function supportsRoute(caps: DesktopCapabilities | null, route: string): boolean {
  if (!caps) return false
  if (route === 'finder') return caps.filesystem_integration === 'file_provider'
  if (['explorer-integration', 'files'].includes(route)) return caps.filesystem_integration !== 'none'
  if (['selective-sync', 'sync'].includes(route)) return caps.on_demand
  if (route === 'onboarding') return caps.native_credentials
  return true
}

export function canInstallUpdate(caps: DesktopCapabilities | null): boolean {
  return caps !== null && ['app_bundle', 'nsis', 'msi', 'appimage'].includes(caps.update_format)
}

export function CapabilityAlternative({ children }: { children?: ReactNode }) {
  return <div role="status" style={{ padding: 24 }}>
    <p>{children ?? 'This feature is not available on this device. You can manage your files in the web app.'}</p>
    <button type="button" onClick={() => void openUrl('https://app.beebeeb.io')}>Open web app</button>
  </div>
}

export function CapabilityGate({ route, children }: { route: string; children: ReactNode }) {
  const caps = useCapabilities()
  return supportsRoute(caps, route) ? children : <CapabilityAlternative />
}

export function CapabilityProvider({ children }: { children: ReactNode }) {
  const [caps, setCaps] = useState<DesktopCapabilities | null>(null)
  const [failed, setFailed] = useState(false)
  const [attempt, setAttempt] = useState(0)
  useEffect(() => {
    let cancelled = false
    // Bound an unresponsive IPC too; no host-specific child mounts before success.
    const timeout = setTimeout(() => { if (!cancelled) setFailed(true) }, 10000)
    void command<DesktopCapabilities>('desktop_capabilities').then((result) => {
      if (cancelled) return
      clearTimeout(timeout)
      if (result.ok) setCaps(result.value)
      else setFailed(true)
    })
    return () => { cancelled = true; clearTimeout(timeout) }
  }, [attempt])
  if (caps) return <CapabilityContext.Provider value={caps}>{children}</CapabilityContext.Provider>
  if (!failed) return <p role="status">Checking device support…</p>
  return <CapabilityAlternative>
    Device support could not be checked.{' '}
    <button type="button" onClick={() => { setFailed(false); setAttempt((value) => value + 1) }}>Retry</button>
  </CapabilityAlternative>
}
