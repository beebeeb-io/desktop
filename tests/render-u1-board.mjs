// LD-0105 U1 board wiring: the Windows tray and Activity › Timeline, driven by a `get_active_transfers` payload the
// Rust side really emitted (never a hand-written row).
//
//   bunx vite --port 5293 --strictPort &
//   PLAYWRIGHT_MODULE=node_modules/playwright/index.mjs \
//     node tests/render-u1-board.mjs http://localhost:5293 OUT_DIR tests/fixtures/active-transfers.ld0105-u1.json LABEL
//
// FIXTURE: { "get_active_transfers": [...], "desktop_recent_files": [...] }, each the IPC's own JSON:
// - `active-transfers.ld0105-u1.json`: the "after", written by the Rust test
//   `ld0105_u1_the_ipc_rows_are_the_shared_fixture` (it fails when the board's JSON differs);
// - `active-transfers.ld0105-u1.before-native.json`: the "before", the release lane's native sample at e93800c.
// One field is rebased: a failed row's `failedAtSecs` (the capture's wall clock) becomes now − 30 s, so the row is
// inside its 15-minute window, and every recent row's `modified_at` likewise; nothing else is touched.
//
// Checks, after the screenshots (exit 1 on any FAIL):
//   A. no upload row is listed on the "In progress" card, and with no download row there is no card
//      (the U0 design gives an upload no state on these surfaces);
//   B. no tray row of an uploading file reads "Downloading";
//   C. every named download row is on the card under its name with its state line, and its tray row shows that line.
// What it does not prove: the native tray icon, WebView2, a real download (the release lane's RC rung).

import assert from 'node:assert/strict'
import { mkdir, readFile } from 'node:fs/promises'
import path from 'node:path'
import { pathToFileURL } from 'node:url'

const { chromium } = await import(
  process.env.PLAYWRIGHT_MODULE ? pathToFileURL(path.resolve(process.env.PLAYWRIGHT_MODULE)).href : 'playwright'
)
const [base, output, fixturePath, label] = process.argv.slice(2)
assert(base && output && fixturePath && label, 'usage: node tests/render-u1-board.mjs BASE_URL OUT_DIR FIXTURE LABEL')
const repo = path.resolve(path.dirname(new URL(import.meta.url).pathname), '..')
const caps = JSON.parse(await readFile(path.join(repo, 'tests/fixtures/desktop-capabilities.json'), 'utf8'))['windows-nsis']
const fixture = JSON.parse(await readFile(path.resolve(fixturePath), 'utf8'))
await mkdir(output, { recursive: true })

const NOW = Math.floor(Date.now() / 1000)
const transfers = fixture.get_active_transfers.map((r) => (r.failedAtSecs == null ? r : { ...r, failedAtSecs: NOW - 30 }))
const recent = fixture.desktop_recent_files.map((f) => ({ ...f, modified_at: NOW - 60 }))
const ups = transfers.filter((r) => r.direction === 'up')
const downs = transfers.filter((r) => r.direction !== 'up')
const stateLine = (r) => (r.failed ? "Couldn't finish downloading" : r.direction === 'stream' ? 'Streaming' : 'Downloading ·')

function shim({ caps, transfers, recent, surface }) {
  const callbacks = new Map()
  let next = 1
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} }
  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: surface }, currentWebview: { label: surface } },
    transformCallback(cb) { const id = next++; callbacks.set(id, cb); return id },
    unregisterCallback(id) { callbacks.delete(id) },
    invoke: async (name) => {
      switch (name) {
        case 'plugin:event|listen': return next++
        case 'plugin:event|unlisten': return null
        case 'desktop_capabilities': return caps
        case 'desktop_platform': return 'windows'
        case 'sync_status':
          return { logged_in: true, engine: 'running', sync_root: 'C:\\Users\\Fixture\\Beebeeb', syncing: 1, cloud_only: 0, conflicts: 0, session_revision: 1 }
        case 'sync_activity_summary':
          return { paused: false, engine_state: 'syncing', files_remaining: 1, transfers_active: 1, last_tick_ok_at: Math.floor(Date.now() / 1000) - 5, queue: { waiting: 0, gave_up: 0, held: [] }, startup_check: null }
        case 'desktop_config':
          return { theme: 'system', pause_sync: false, local_cache_limit_bytes: 0, upload_kbps_limit: 0, download_kbps_limit: 0, metered: false, files_on_demand: true, sync_overlays: true }
        case 'desktop_storage_summary':
          return { used_bytes: 48_300_000_000, quota_bytes: 200_000_000_000, cache_bytes: 1_200_000_000, pinned_bytes: 2_100_000_000 }
        case 'desktop_file_overview':
          return { total_files: 0, total_bytes: 0, by_status: [], recent: [], pinned_count: 0, pinned_bytes: 0 }
        case 'desktop_recent_files': return recent
        case 'get_active_transfers': return transfers
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

const browser = await chromium.launch({ headless: true, ...(process.env.CHROME_PATH ? { executablePath: process.env.CHROME_PATH } : {}) })
const checks = []
const check = (what, ok, detail = '') => checks.push([ok, what, detail])
const wait = (page, text) => page.getByText(text, { exact: false }).first().waitFor({ timeout: 8000 })
const files = []

try {
  for (const scheme of ['light', 'dark']) {
    // ── Activity › Timeline ──
    const activity = await browser.newPage({ viewport: { width: 1200, height: 860 } })
    activity.on('pageerror', (err) => console.error(`pageerror [activity-${scheme}]:`, err))
    await activity.emulateMedia({ colorScheme: scheme })
    await activity.addInitScript(shim, { caps, transfers, recent, surface: 'main-app' })
    await activity.goto(`${base}/?window=main-app&platform=windows`)
    await activity.getByRole('button', { name: 'Activity', exact: true }).click()
    await wait(activity, 'Your account')
    if (downs.length > 0) await wait(activity, 'Files arriving on this PC.')
    else await activity.waitForTimeout(1500) // two polls of the board: an upload must not bring the card
    const shotA = path.join(output, `${label}-activity-${scheme}.png`)
    await activity.screenshot({ path: shotA, fullPage: false })
    files.push(shotA)
    const cardCount = await activity.getByText('Files arriving on this PC.').count()
    // The page shows no timeline events (the shim has none), so any file name or id on it is the card's.
    const pageText = await activity.locator('body').textContent()
    if (scheme === 'light') {
      check(`A: card shown only for download rows (${downs.length} download, ${ups.length} upload)`, (cardCount > 0) === (downs.length > 0), `card=${cardCount}`)
      for (const r of ups) {
        const shown = cardCount > 0 && (pageText.includes(r.fileId) || (r.name !== '' && pageText.includes(r.name)))
        check(`A: upload ${r.name || r.fileId} not listed as arriving`, !shown)
      }
      for (const r of downs.filter((r) => r.name)) {
        await wait(activity, r.name).catch(() => undefined)
        const txt = await activity.locator('body').textContent()
        check(`C: card lists ${r.name} with "${stateLine(r)}"`, txt.includes(r.name) && txt.includes(stateLine(r)))
      }
    }
    await activity.close()

    // ── Tray popover ──
    const tray = await browser.newPage({ viewport: { width: 400, height: 560 } })
    tray.on('pageerror', (err) => console.error(`pageerror [tray-${scheme}]:`, err))
    await tray.emulateMedia({ colorScheme: scheme })
    await tray.addInitScript(shim, { caps, transfers, recent, surface: 'tray' })
    await tray.goto(`${base}/?window=tray`)
    for (const f of recent) await wait(tray, path.posix.basename(f.path))
    await tray.waitForTimeout(1500) // the board's first poll lands
    const shotT = path.join(output, `${label}-tray-${scheme}.png`)
    await tray.screenshot({ path: shotT, fullPage: false })
    files.push(shotT)
    if (scheme === 'light') {
      for (const f of recent) {
        const row = tray.locator(`button[title="Open ${path.posix.basename(f.path)}"]`)
        const txt = (await row.count()) > 0 ? await row.first().textContent() : ''
        if (f.status === 'uploading') check(`B: tray row ${f.path} (uploading) does not read "Downloading"`, !txt.includes('Downloading'), txt)
        const r = downs.find((r) => (r.folder ? `${r.folder}/` : '') + r.name === f.path.replace(/^\/+/, ''))
        if (r) check(`C: tray row ${f.path} shows "${stateLine(r).replace(' ·', '')}"`, txt.includes(stateLine(r).replace(' ·', '')), txt)
      }
    }
    await tray.close()
  }
} finally {
  await browser.close()
}

for (const f of files) console.log(`shot ${f}`)
for (const [ok, what, detail] of checks) console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}${ok || !detail ? '' : `  [${detail}]`}`)
const failed = checks.filter(([ok]) => !ok).length
console.log(`browser checks (${label}): ${checks.length - failed} passed; ${failed} failed`)
process.exit(failed === 0 ? 0 : 1)
