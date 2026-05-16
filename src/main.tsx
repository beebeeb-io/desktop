/**
 * Vite entry — picks which window component to mount based on the
 * `window` query param set by the Tauri side when it opens the
 * webview.
 *
 *   • default             → settings shell (App.tsx)
 *   • ?window=conflict    → conflict resolution UI (ConflictWindow.tsx),
 *                           with fileId / fileName / isText also read
 *                           from the query string by the component itself
 *   • ?window=onboarding  → first-launch onboarding (Onboarding.tsx),
 *                           3-step flow: login → folder picker → sync status.
 *                           Opened automatically by lib.rs on first launch
 *                           when no sync_root is configured.
 *
 * Single HTML entry (index.html) keeps the bundle layout simple — all
 * components are tree-shaken individually so inactive ones barely cost
 * anything in the cold start.
 *
 * See docs/superpowers/plans/2026-05-07-desktop-sync-client.md (Task 8 + 12).
 */

import { StrictMode, type ReactElement } from 'react'
import { createRoot } from 'react-dom/client'
import App from './App'
import ConflictWindow from './ConflictWindow'
import Onboarding from './Onboarding'
import './design.css'

const container = document.getElementById('root')
if (!container) {
  throw new Error('root element missing from index.html')
}

const params = new URLSearchParams(window.location.search)
const which = params.get('window')

let component: ReactElement
if (which === 'conflict') {
  component = <ConflictWindow />
} else if (which === 'onboarding') {
  component = <Onboarding />
} else {
  component = <App />
}

createRoot(container).render(<StrictMode>{component}</StrictMode>)
