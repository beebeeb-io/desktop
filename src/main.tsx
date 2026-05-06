/**
 * Vite entry — mounts the settings App into the #root div from
 * index.html. StrictMode is intentionally on; the desktop window is
 * tiny and rare enough that the double-render trade-off is fine.
 *
 * See docs/superpowers/plans/2026-05-07-desktop-sync-client.md (Task 8).
 */

import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import App from './App'

const container = document.getElementById('root')
if (!container) {
  throw new Error('root element missing from index.html')
}

createRoot(container).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
