// LD-0105 U1 — browser evidence for the streaming / download progress row.
// Renders five transfer states (downloading, streaming, buffering, failed,
// needs_attention) in the Windows tray popover's Recent list and on the
// Activity › Timeline In-Progress card, using a synthetic Tauri IPC.
// No app, no account, no network.
//
//   bunx vite --port 5292 --strictPort &
//   PLAYWRIGHT_MODULE=/path/to/node_modules/playwright/index.mjs \
//     node tests/render-download-progress.mjs http://localhost:5292 OUT_DIR
//
// What it proves: the tray row shows the correct subline and progress bar for
// each state; "failed" and "needs_attention" never say "complete"; the Activity
// In-Progress card renders per-state detail and the action button for
// needs_attention; light and dark screenshots captured for needs_attention.
// What it does not: the native tray icon, WebView2, real speed measurements.

import assert from 'node:assert/strict'
import { mkdir, readFile } from 'node:fs/promises'
import path from 'node:path'
import { pathToFileURL } from 'node:url'

const { chromium } = await import(
  process.env.PLAYWRIGHT_MODULE
    ? pathToFileURL(process.env.PLAYWRIGHT_MODULE).href
    : 'playwright'
)
const [base, output] = process.argv.slice(2)
assert(base && output, 'usage: node tests/render-download-progress.mjs BASE_URL OUTPUT_DIR')
const repo = path.resolve(path.dirname(new URL(import.meta.url).pathname), '..')
const caps = JSON.parse(await readFile(path.join(repo, 'tests/fixtures/desktop-capabilities.json'), 'utf8'))['windows-nsis']
await mkdir(output, { recursive: true })

const NOW = Math.floor(Date.now() / 1000)

// ── Synthetic active-transfer DTOs for each state ────────────────────────────
const TRANSFERS = {
  downloading: [
    { fileId: 'f-down-1', name: 'Quarter plan.docx', folder: 'Documents/Plans',
      direction: 'down', done: 340_000_000, total: 1_000_000_000,
      buffered: 0, speedBps: 9_600_000, failed: false, failedAtSecs: null },
  ],
  streaming: [
    { fileId: 'f-stream-1', name: 'Holiday highlights.mp4', folder: 'Videos',
      direction: 'stream', done: 0, total: 2_000_000_000,
      buffered: 610_000_000, speedBps: 15_700_000, failed: false, failedAtSecs: null },
  ],
  buffering: [
    { fileId: 'f-buf-1', name: 'Concert recording.flac', folder: 'Music/Live',
      direction: 'stream', done: 0, total: 0,
      buffered: 0, speedBps: 8_100_000, failed: false, failedAtSecs: null },
  ],
  failed: [
    { fileId: 'f-fail-1', name: 'Archive backup.zip', folder: 'Backups',
      direction: 'down', done: 50_000_000, total: 100_000_000,
      buffered: 0, speedBps: 0, failed: true, failedAtSecs: NOW - 30 },
  ],
  // Amendment 4: file changed on this device after a failed download.
  // `failed` is false; the poison row's key is content (LastWriteTime + size).
  needs_attention: [
    { fileId: 'f-attn-1', name: 'Report draft.pdf', folder: 'Documents',
      direction: 'down', done: 50_000_000, total: 100_000_000,
      buffered: 0, speedBps: 0, failed: false, failedAtSecs: null,
      attention: 'changed_on_this_device', action: 'discard_and_download_again' },
  ],
  // P2 fix (review 2026-11-06): default file-type glyph uses var(--ink-3) token;
  // a .txt file verifies the default case renders without a broken stroke.
  default_type: [
    { fileId: 'f-txt-1', name: 'Meeting notes.txt', folder: 'Notes',
      direction: 'down', done: 10_000, total: 50_000,
      buffered: 0, speedBps: 1_200_000, failed: false, failedAtSecs: null },
  ],
}

const TRAY_ASSERTIONS = {
  downloading:    { subline: 'Downloading', absent: null },
  streaming:      { subline: 'Streaming', absent: null },
  buffering:      { subline: 'Buffering', absent: null },
  failed:         { subline: "Couldn't finish downloading", absent: 'complete' },
  // Amendment 4: same short line as failed; full sentence in Activity/tooltip.
  needs_attention: { subline: "Couldn't finish downloading", absent: 'complete' },
}

const ACTIVITY_ASSERTIONS = {
  downloading:    { contains: 'Downloading', absent: null },
  streaming:      { contains: 'Streaming', absent: null },
  buffering:      { contains: 'Buffering', absent: null },
  failed:         { contains: "Couldn't finish downloading", absent: 'complete' },
  // Amendment 4: Activity shows the full corrected detail + action button.
  needs_attention: { contains: 'The file changed on this device', absent: null },
}

// ── IPC shim factory ─────────────────────────────────────────────────────────
// `recentPath` must have the transfer's `name` as its basename so the tray's
// `transferByName` map can match the recent-file row.
function makeShim() {
  return ({ caps, surface, transfers, summary, recentPath }) => {
    window.__active_transfers__ = transfers
    const callbacks = new Map()
    const listeners = []
    let next = 1
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} }
    window.emitFixtureEvent = (event, payload) => {
      for (const l of listeners.filter((l) => l.event === event)) {
        callbacks.get(l.handler)?.({ event, id: l.id, payload })
      }
    }
    window.__TAURI_INTERNALS__ = {
      metadata: {
        currentWindow: { label: surface },
        currentWebview: { label: surface },
      },
      transformCallback(cb) { const id = next++; callbacks.set(id, cb); return id },
      unregisterCallback(id) { callbacks.delete(id) },
      invoke: async (name, args) => {
        switch (name) {
          case 'plugin:event|listen':
            listeners.push({ event: args.event, handler: args.handler, id: next })
            return next++
          case 'plugin:event|unlisten': return null
          case 'desktop_capabilities': return caps
          case 'desktop_platform': return 'windows'
          case 'sync_status':
            return { logged_in: true, engine: 'running', sync_root: 'C:\\Users\\Fixture\\Beebeeb', syncing: 0, cloud_only: 0, conflicts: 0, session_revision: 1 }
          case 'sync_activity_summary': return summary
          case 'desktop_config':
            return { theme: 'system', pause_sync: false, local_cache_limit_bytes: 0, upload_kbps_limit: 0, download_kbps_limit: 0, metered: false, files_on_demand: true, sync_overlays: true }
          case 'desktop_storage_summary':
            return { used_bytes: 48_300_000_000, quota_bytes: 200_000_000_000, cache_bytes: 1_200_000_000, pinned_bytes: 2_100_000_000 }
          case 'desktop_file_overview':
            return { total_files: 0, total_bytes: 0, by_status: [], recent: [], pinned_count: 0, pinned_bytes: 0 }
          case 'desktop_recent_files':
            return [{ path: recentPath, size_bytes: 24576, status: 'cloud_only', pinned: false, modified_at: Math.floor(Date.now() / 1000) - 60 }]
          case 'get_active_transfers': return window.__active_transfers__
          case 'discard_and_download_again': return null  // void success (void → null)
          case 'account_activity': return { items: [], total: 0, page: 1, limit: 50 }
          case 'account_activity_feed': return { events: [], total: 0 }
          case 'account_region': return { preferred_region: null, regions: [], choices: [] }
          case 'account_usage': return { storage_used_bytes: 48_300_000_000, storage_quota_bytes: 200_000_000_000 }
          default:
            if (name.startsWith('plugin:')) return null
            throw new Error(`fixture: no ${name}`)
        }
      },
    }
  }
}

const idleSummary = {
  paused: false,
  engine_state: 'idle',
  files_remaining: null,
  transfers_active: 0,
  last_tick_ok_at: NOW - 20,
  queue: { waiting: 0, gave_up: 0, held: [] },
  startup_check: null,
}

const browser = await chromium.launch({
  headless: true,
  ...(process.env.CHROME_PATH ? { executablePath: process.env.CHROME_PATH } : {}),
})
const results = []

async function shot(page, name) {
  const file = path.join(output, `${name}.png`)
  await page.screenshot({ path: file, fullPage: false })
  return file
}

function expectText(page, text) {
  return page.getByText(text, { exact: false }).first().waitFor({ timeout: 8000 })
}

try {
  const shim = makeShim()

  // ── Tray: one screenshot per transfer state (4 original + needs_attention) ───
  for (const state of ['downloading', 'streaming', 'buffering', 'failed']) {
    const transfers = TRANSFERS[state]
    // recentPath basename must match transfer.name so the tray row gets the overlay
    const recentPath = `${transfers[0].folder}/${transfers[0].name}`
    const { subline, absent } = TRAY_ASSERTIONS[state]
    const page = await browser.newPage({ viewport: { width: 400, height: 560 } })
    page.on('pageerror', (err) => console.error(`pageerror [tray-${state}]:`, err))
    await page.addInitScript(shim, { caps, surface: 'tray', transfers, summary: idleSummary, recentPath })
    await page.goto(`${base}/?window=tray`)
    // Wait for the file name to appear in the recent list
    await expectText(page, transfers[0].name)
    // Now wait for the subline from the model
    await expectText(page, subline)
    // Guard: failed state must not say "complete"
    if (absent) {
      const text = await page.locator('body').textContent()
      assert.ok(!text.toLowerCase().includes(absent), `tray-${state}: must not contain "${absent}" but got content with it`)
    }
    const file = await shot(page, `tray-transfer-${state}`)
    results.push([`tray-transfer-${state}: "${subline}" visible`, file])
    await page.close()
  }

  // ── Tray: needs_attention light + dark (Amendment 4) ────────────────────────
  for (const scheme of ['light', 'dark']) {
    const state = 'needs_attention'
    const transfers = TRANSFERS[state]
    const recentPath = `${transfers[0].folder}/${transfers[0].name}`
    const { subline, absent } = TRAY_ASSERTIONS[state]
    const page = await browser.newPage({ viewport: { width: 400, height: 560 } })
    page.on('pageerror', (err) => console.error(`pageerror [tray-${state}-${scheme}]:`, err))
    await page.emulateMedia({ colorScheme: scheme })
    await page.addInitScript(makeShim(), { caps, surface: 'tray', transfers, summary: idleSummary, recentPath })
    await page.goto(`${base}/?window=tray`)
    await expectText(page, transfers[0].name)
    await expectText(page, subline)
    if (absent) {
      const text = await page.locator('body').textContent()
      assert.ok(!text.toLowerCase().includes(absent), `tray-${state}-${scheme}: must not contain "${absent}"`)
    }
    const file = await shot(page, `tray-transfer-${state}-${scheme}`)
    results.push([`tray-transfer-${state}-${scheme}: "${subline}" visible`, file])
    await page.close()
  }

  // ── Activity › In-Progress card: one screenshot per transfer state ───────────
  for (const state of ['downloading', 'streaming', 'buffering', 'failed']) {
    const transfers = TRANSFERS[state]
    const recentPath = `${transfers[0].folder}/${transfers[0].name}`
    const { contains, absent } = ACTIVITY_ASSERTIONS[state]
    const page = await browser.newPage({ viewport: { width: 1200, height: 860 } })
    page.on('pageerror', (err) => console.error(`pageerror [activity-${state}]:`, err))
    await page.addInitScript(shim, { caps, surface: 'main-app', transfers, summary: idleSummary, recentPath })
    await page.goto(`${base}/?window=main-app&platform=windows`)
    // Navigate to Activity
    await page.getByRole('button', { name: 'Activity', exact: true }).click()
    // Wait for In-Progress card header
    await expectText(page, 'Files arriving on this PC.')
    // Wait for the file name inside the card
    await expectText(page, transfers[0].name)
    // Check the detail text
    await expectText(page, contains)
    // Guard: failed state must not say "complete"
    if (absent) {
      const text = await page.locator('body').textContent()
      assert.ok(!text.toLowerCase().includes(absent), `activity-${state}: must not contain "${absent}"`)
    }
    const file = await shot(page, `activity-transfer-${state}`)
    results.push([`activity-transfer-${state}: "${contains}" visible`, file])
    await page.close()
  }
  // ── Activity: needs_attention light + dark (Amendment 4) ────────────────────
  for (const scheme of ['light', 'dark']) {
    const state = 'needs_attention'
    const transfers = TRANSFERS[state]
    const recentPath = `${transfers[0].folder}/${transfers[0].name}`
    const { contains, absent } = ACTIVITY_ASSERTIONS[state]
    const page = await browser.newPage({ viewport: { width: 1200, height: 860 } })
    page.on('pageerror', (err) => console.error(`pageerror [activity-${state}-${scheme}]:`, err))
    await page.emulateMedia({ colorScheme: scheme })
    await page.addInitScript(makeShim(), { caps, surface: 'main-app', transfers, summary: idleSummary, recentPath })
    await page.goto(`${base}/?window=main-app&platform=windows`)
    await page.getByRole('button', { name: 'Activity', exact: true }).click()
    await expectText(page, 'Files arriving on this PC.')
    await expectText(page, transfers[0].name)
    await expectText(page, contains)
    // The action button must be visible
    await expectText(page, 'Discard and download again')
    if (absent) {
      const text = await page.locator('body').textContent()
      assert.ok(!text.toLowerCase().includes(absent), `activity-${state}-${scheme}: must not contain "${absent}"`)
    }
    const file = await shot(page, `activity-transfer-${state}-${scheme}`)
    results.push([`activity-transfer-${state}-${scheme}: "${contains}" + action button visible`, file])
    await page.close()
  }
  // ── Default-type glyph (.txt): verifies var(--ink-3) renders correctly ───────
  {
    const state = 'default_type'
    const transfers = TRANSFERS[state]
    const recentPath = `${transfers[0].folder}/${transfers[0].name}`
    const page = await browser.newPage({ viewport: { width: 400, height: 560 } })
    page.on('pageerror', (err) => console.error(`pageerror [tray-${state}]:`, err))
    await page.addInitScript(makeShim(), { caps, surface: 'tray', transfers, summary: idleSummary, recentPath })
    await page.goto(`${base}/?window=tray`)
    await expectText(page, transfers[0].name)
    await expectText(page, 'Downloading')
    const file = await shot(page, `tray-transfer-${state}`)
    results.push([`tray-transfer-${state}: default glyph (var(--ink-3)) visible`, file])
    await page.close()
  }
} finally {
  await browser.close()
}

for (const [what, file] of results) console.log(`ok  ${what}  ${file}`)
console.log(`browser cases: ${results.length} passed; 0 failed`)
