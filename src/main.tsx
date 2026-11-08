/**
 * Vite entry — picks which window component to mount based on the
 * `window` query param set by the Tauri side when it opens the webview.
 *
 *   • default (no ?window)         → compact app shell (App.tsx). On macOS
 *                                    the Rust side tags the URL
 *                                    ?platform=macos (this window also
 *                                    serves as the tray flyout there) —
 *                                    see `html.macos-flyout` in design.css,
 *                                    task 1384.
 *   • ?window=conflict             → conflict resolution (ConflictWindow.tsx)
 *   • ?window=onboarding           → first-launch flow (Onboarding.tsx, Linux; a Mac signs in in the
 *                                    Beebeeb window below and never builds this one)
 *   • ?window=onboarding&platform=windows
 *                                  → Windows first-run wizard (WindowsFirstRun.tsx)
 *   • ?window=tray                 → Windows 11 tray flyout (WindowsTray.tsx)
 *   • ?window=main-app&platform=windows
 *                                  → Windows main app shell (WindowsApp.tsx) —
 *                                    sidebar + content router hosting the data views
 *   • desktop v2 (branch v2): ?window=main-app&platform=windows mounts src/v2 (V2App) instead; the
 *                                    legacy shell stays reachable at ?window=legacy-main-app&platform=windows
 *                                    until batch B13 retires it. The tray and the Windows first run move
 *                                    to v2 when src/v2/tray/enabled.ts / src/v2/onboarding/enabled.ts flip.
 *                                    A plain browser gets the fixtures DriveSource (?frame=DW-xxxx, ?scenario=).
 *   • ?window=main&platform=macos  → the Beebeeb window on a Mac (MacApp.tsx, task 1683
 *                                    slice 1): Home and Settings in one window, label `main`,
 *                                    built on demand from tauri.macos.conf.json.
 *
 * Single HTML entry keeps the bundle layout simple — inactive components are
 * tree-shaken. The tray and main app windows are opened by the Rust side via
 * tauri::WebviewWindowBuilder; they receive their ?window param in the URL.
 *
 * See docs/superpowers/plans/2026-05-07-desktop-sync-client.md (Task 8 + 12).
 */

import { StrictMode, useEffect, useState, type ReactElement } from 'react'
import { createRoot } from 'react-dom/client'
import App from './App'
import AccountSessionBoundary from './AccountSessionBoundary'
import ConflictWindow from './ConflictWindow'
import Onboarding from './Onboarding'
import WindowsTray from './WindowsTray'
import WindowsFirstRun from './WindowsFirstRun'
import WindowsApp from './WindowsApp'
import MacApp from './MacApp'
import PopoverApp from './PopoverApp'
import { initializeDesktopThemeFromConfig } from './windows/theme'
import { ToastProvider } from './windows/ui'
import { CapabilityProvider, CapabilityGate, useCapabilities } from './capabilities'
import { TRAY_V2 } from './v2/tray/enabled'
import { ONBOARDING_V2 } from './v2/onboarding/enabled'
import './design.css'
import './macSettings.css'
import './macApp.css'
import './popover.css'

const container = document.getElementById('root')
if (!container) {
  throw new Error('root element missing from index.html')
}

const params = new URLSearchParams(window.location.search)
const which = params.get('window')
const platform = params.get('platform')

void initializeDesktopThemeFromConfig()

function HostOnboarding() {
  const caps = useCapabilities()
  // Task 1683 slice 2: a Mac signs in in the Beebeeb window and never builds this one. Were it ever loaded there, the
  // Linux-style window below would run the install-era Finder commands, which no macOS code path may reach.
  if (caps?.host_os === 'macos') return null
  return <CapabilityGate route="onboarding">{caps?.host_os === 'windows' ? <WindowsFirstRun /> : <Onboarding mode={params.get('mode') === 'reauth' ? 'reauth' : 'setup'} />}</CapabilityGate>
}

const v2Target =
  which === 'main-app' && platform === 'windows' ? 'main'
  : which === 'tray' && TRAY_V2 ? 'tray'
  : which === 'onboarding' && platform === 'windows' && ONBOARDING_V2 ? 'onboarding'
  : null

function V2Window({ target }: { target: 'main' | 'tray' | 'onboarding' }) {
  const [loaded, setLoaded] = useState<ReactElement | null>(null)
  useEffect(() => {
    let live = true
    void import('./v2/entry').then((m) => {
      m.followTheme()
      if (live) setLoaded(target === 'main' ? <m.V2App /> : target === 'tray' ? <m.TrayApp /> : <m.OnboardingApp />)
    })
    return () => { live = false }
  }, [target])
  return loaded
}

let component: ReactElement
if (v2Target) {
  component = <V2Window target={v2Target} />
} else if (which === 'conflict') {
  component = <ConflictWindow />
} else if (which === 'tray') {
  // The tray webview is frameless + opaque. Tag <html> so design.css can
  // apply the tray-specific document surface without affecting other windows.
  document.documentElement.classList.add('tray-window')
  component = <WindowsTray />
} else if (which === 'onboarding' && platform === 'windows') {
  component = <HostOnboarding />
} else if (which === 'onboarding') {
  component = <HostOnboarding />
} else if (which === 'legacy-main-app' && platform === 'windows') {
  component = <WindowsApp />
} else if (which === 'popover' && platform === 'macos') {
  document.documentElement.classList.add('popover-window')
  component = <PopoverApp />
} else if (which === 'main' && platform === 'macos') {
  component = <MacApp />
} else {
  if (platform === 'macos') {
    // This window also serves as the macOS tray flyout (task 1384). Below
    // design.css's 820px responsive breakpoint, `.app-shell` collapses its
    // sidebar+content grid to a single stacked column — correct for the
    // browser/main-app responsive case, wrong for this fixed 680px native
    // window, where it reproduced 1173's overflow (nav + footer pushed the
    // page below the fold). `html.macos-flyout` forces the two-column grid
    // back on regardless of width, for this window only.
    document.documentElement.classList.add('macos-flyout')
  }
  component = <App />
}

// A v2 window in a plain browser (headless frames, fixtures) has no IPC: the session boundary and the
// capability provider would wait forever, so it mounts bare. Inside Tauri it keeps both.
const bareV2 = v2Target != null && !('__TAURI_INTERNALS__' in window)

const root = createRoot(container)
if (bareV2) {
  root.render(<StrictMode>{component}</StrictMode>)
} else root.render(
  <StrictMode>
    {which === 'onboarding' ? (
      <ToastProvider><CapabilityProvider>{component}</CapabilityProvider></ToastProvider>
    ) : which === 'popover' ? (
      <AccountSessionBoundary><CapabilityProvider>{component}</CapabilityProvider></AccountSessionBoundary>
    ) : (
      <AccountSessionBoundary><ToastProvider><CapabilityProvider>{component}</CapabilityProvider></ToastProvider></AccountSessionBoundary>
    )}
  </StrictMode>,
)
