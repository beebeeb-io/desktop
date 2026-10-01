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
 *   • ?window=onboarding           → first-launch flow (Onboarding.tsx, macOS)
 *   • ?window=onboarding&platform=windows
 *                                  → Windows first-run wizard (WindowsFirstRun.tsx)
 *   • ?window=tray                 → Windows 11 tray flyout (WindowsTray.tsx)
 *   • ?window=main-app&platform=windows
 *                                  → Windows main app shell (WindowsApp.tsx) —
 *                                    sidebar + content router hosting the data views
 *   • ?window=popover&platform=macos
 *                                  → macOS menu-bar popover (macPopover/MacPopover.tsx,
 *                                    task 1683 slice 3). DEV URL ONLY until slice 6: no
 *                                    Tauri config creates this window yet.
 *
 * Single HTML entry keeps the bundle layout simple — inactive components are
 * tree-shaken. The tray and main app windows are opened by the Rust side via
 * tauri::WebviewWindowBuilder; they receive their ?window param in the URL.
 *
 * See docs/superpowers/plans/2026-05-07-desktop-sync-client.md (Task 8 + 12).
 */

import { StrictMode, type ReactElement } from 'react'
import { createRoot } from 'react-dom/client'
import App from './App'
import AccountSessionBoundary from './AccountSessionBoundary'
import ConflictWindow from './ConflictWindow'
import Onboarding from './Onboarding'
import WindowsTray from './WindowsTray'
import WindowsFirstRun from './WindowsFirstRun'
import WindowsApp from './WindowsApp'
import MacPopover from './macPopover/MacPopover'
import { DEFAULT_CONFIG } from './desktopApi'
import { initializeDesktopThemeFromConfig, setDesktopThemePreference } from './windows/theme'
import { ToastProvider } from './windows/ui'
import { CapabilityProvider, CapabilityGate, useCapabilities } from './capabilities'
import './design.css'
import './macPopover/macPopover.css'

const container = document.getElementById('root')
if (!container) {
  throw new Error('root element missing from index.html')
}

const params = new URLSearchParams(window.location.search)
const which = params.get('window')
const platform = params.get('platform')

// The macOS popover is created once and kept alive, so it must make no call until it is shown:
// it applies the system theme now (no command) and re-reads the saved preference on every show.
const isMacPopover = which === 'popover' && platform === 'macos'
if (isMacPopover) setDesktopThemePreference(DEFAULT_CONFIG.theme)
else void initializeDesktopThemeFromConfig()

function HostOnboarding() {
  const caps = useCapabilities()
  return <CapabilityGate route="onboarding">{caps?.host_os === 'windows' ? <WindowsFirstRun /> : <Onboarding />}</CapabilityGate>
}

let component: ReactElement
if (isMacPopover) {
  // The popover is a frameless 372 x 488 surface of its own; `html.mac-popover` (macPopover.css)
  // sizes the document to it. No session boundary, toast host or capability provider: each one
  // polls or fetches on mount, and the popover must stay silent while hidden.
  document.documentElement.classList.add('mac-popover')
  component = <MacPopover />
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
} else if (which === 'main-app' && platform === 'windows') {
  component = <WindowsApp />
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

createRoot(container).render(
  <StrictMode>
    {isMacPopover ? (
      component
    ) : which === 'onboarding' ? (
      <ToastProvider><CapabilityProvider>{component}</CapabilityProvider></ToastProvider>
    ) : (
      <AccountSessionBoundary><ToastProvider><CapabilityProvider>{component}</CapabilityProvider></ToastProvider></AccountSessionBoundary>
    )}
  </StrictMode>,
)
