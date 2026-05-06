/**
 * Vite entry — picks which window component to mount based on the
 * `window` query param set by the Tauri side when it opens the
 * webview.
 *
 *   • default          → settings shell (App.tsx)
 *   • ?window=conflict → conflict resolution UI (ConflictWindow.tsx),
 *                        with fileId / fileName / isText also read
 *                        from the query string by the component itself
 *
 * Single HTML entry (index.html) keeps the bundle layout simple — both
 * components are tree-shaken individually so the inactive one barely
 * costs anything in the cold start. If the conflict UI grows enough
 * to warrant its own bundle later, split it off via vite multi-page
 * input then.
 *
 * See docs/superpowers/plans/2026-05-07-desktop-sync-client.md (Task 8 + 12).
 */

import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import App from './App'
import ConflictWindow from './ConflictWindow'

const container = document.getElementById('root')
if (!container) {
  throw new Error('root element missing from index.html')
}

const params = new URLSearchParams(window.location.search)
const which = params.get('window')

createRoot(container).render(
  <StrictMode>{which === 'conflict' ? <ConflictWindow /> : <App />}</StrictMode>,
)
