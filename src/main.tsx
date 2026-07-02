/**
 * Vite entry — picks which window component to mount based on the
 * `window` query param set by the Tauri side when it opens the webview.
 *
 *   • default (no ?window)         → compact app shell (App.tsx)
 *   • ?window=conflict             → conflict resolution (ConflictWindow.tsx)
 *   • ?window=onboarding           → first-launch flow (Onboarding.tsx, macOS)
 *   • ?window=onboarding&platform=windows
 *                                  → Windows first-run wizard (WindowsFirstRun.tsx)
 *   • ?window=tray                 → Windows 11 tray flyout (WindowsTray.tsx)
 *   • ?window=main-app&platform=windows
 *                                  → Windows main app shell (WindowsApp.tsx) —
 *                                    sidebar + content router hosting the data views
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
import ConflictWindow from './ConflictWindow'
import Onboarding from './Onboarding'
import WindowsTray from './WindowsTray'
import WindowsFirstRun from './WindowsFirstRun'
import WindowsApp from './WindowsApp'
import './design.css'

const container = document.getElementById('root')
if (!container) {
  throw new Error('root element missing from index.html')
}

const params = new URLSearchParams(window.location.search)
const which = params.get('window')
const platform = params.get('platform')

let component: ReactElement
if (which === 'conflict') {
  component = <ConflictWindow />
} else if (which === 'tray') {
  // The tray webview is frameless + opaque. Tag <html> so design.css can
  // apply the tray-specific document surface without affecting other windows.
  document.documentElement.classList.add('tray-window')
  component = <WindowsTray />
} else if (which === 'onboarding' && platform === 'windows') {
  component = <WindowsFirstRun />
} else if (which === 'onboarding') {
  component = <Onboarding />
} else if (which === 'main-app' && platform === 'windows') {
  component = <WindowsApp />
} else {
  component = <App />
}

createRoot(container).render(<StrictMode>{component}</StrictMode>)
