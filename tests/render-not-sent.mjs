// Task 2409 browser evidence: Home's "Not sent" figure and the tray's waiting, held and given-up lines are links to
// Activity › Not sent, which lists each change the count counts, with its reason and its one action; Try again on a
// failed change. Against the Vite dev server with a synthetic Tauri IPC (no app, no account, no network). The three
// rows are `tests/fixtures/unsent-changes.page.json`, the JSON `unsent_changes_commands.rs` serialises: a new file
// waiting (being sent), an edit on hold (storage full), a delete that stopped after repeated errors.
//
//   bunx vite --port 5292 --strictPort &
//   PLAYWRIGHT_MODULE=/path/to/node_modules/playwright/index.mjs node tests/render-not-sent.mjs http://localhost:5292 OUT
//
// What it does not prove: WebView2, the real engine, the tray window's show/hide (NATIVE rung, the lead's).
import assert from 'node:assert/strict'
import { mkdir, readFile } from 'node:fs/promises'
import path from 'node:path'
import { pathToFileURL } from 'node:url'

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ? pathToFileURL(process.env.PLAYWRIGHT_MODULE).href : 'playwright')
const [base, output] = process.argv.slice(2)
assert(base && output, 'usage: node tests/render-not-sent.mjs BASE_URL OUTPUT_DIR')
const repo = path.resolve(path.dirname(new URL(import.meta.url).pathname), '..')
const caps = JSON.parse(await readFile(path.join(repo, 'tests/fixtures/desktop-capabilities.json'), 'utf8'))['windows-nsis']
const page3 = JSON.parse(await readFile(path.join(repo, 'tests/fixtures/unsent-changes.page.json'), 'utf8'))
await mkdir(output, { recursive: true })

const now = Math.floor(Date.now() / 1000)
// The rows' dates, relative to now, so "queued N ago" reads like a real afternoon.
const unsent = {
  ...page3,
  engine_state: 'idle',
  items: page3.items.map((item, i) => ({ ...item, queued_at: now - [26 * 3600, 3 * 3600, 4 * 60][i] })),
}
// The polled summary for the same three changes: 1 waiting, 1 held (storage full), 1 given up. Not sent = 3.
const summary = {
  paused: false,
  engine_state: 'idle',
  files_remaining: null,
  transfers_active: 1,
  last_tick_ok_at: now - 20,
  queue: { waiting: 1, gave_up: 1, held: [{ reason: 'quota', count: 1, oldest_at: now - 3 * 3600, newest_at: now - 3 * 3600 }], not_downloaded: 0 },
  startup_check: null,
}
const ROW_IDS = ['op-delete', 'op-edit', 'op-new']

const browser = await chromium.launch({ headless: true, ...(process.env.CHROME_PATH ? { executablePath: process.env.CHROME_PATH } : {}) })
const results = []

async function open({ surface, viewport, colorScheme = 'light', locked = false, allSent = false }) {
  const page = await browser.newPage({ viewport, colorScheme })
  const errors = []
  page.on('pageerror', (error) => errors.push(String(error)))
  await page.addInitScript(({ caps, summary, unsent, surface, locked }) => {
    window.calls = []
    window.fixtureUnsent = unsent
    const callbacks = new Map()
    let next = 1
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} }
    window.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: surface }, currentWebview: { label: surface } },
      transformCallback(cb) { const id = next++; callbacks.set(id, cb); return id },
      unregisterCallback(id) { callbacks.delete(id) },
      invoke: async (name, args) => {
        window.calls.push({ name, args })
        switch (name) {
          case 'plugin:event|listen': return next++
          case 'desktop_capabilities': return caps
          case 'desktop_platform': return 'windows'
          case 'sync_status': return { logged_in: true, engine: 'running', sync_root: 'C:\\Users\\Fixture\\Beebeeb', syncing: 1, cloud_only: 0, conflicts: 0, session_revision: 1 }
          case 'sync_activity_summary': return summary
          case 'unsent_changes': {
            if (locked) throw 'vault is locked'
            const { offset = 0, limit = 100 } = args ?? {}
            const u = window.fixtureUnsent
            return { ...u, items: u.items.slice(offset, offset + limit) }
          }
          case 'retry_unsent_change': {
            const u = window.fixtureUnsent
            const row = u.items.find((i) => i.id === args.id)
            if (!row) throw 'unknown'
            // What `retry_unsent_change` does to the row: due now, no longer given up; the count does not move.
            window.fixtureUnsent = { ...u, items: u.items.map((i) => (i.id === args.id ? { ...i, group: 'waiting', retryable: false, retry_at: null } : i)) }
            return 'queued'
          }
          case 'unlock_vault': return null
          case 'show_main_app_window': return null
          case 'desktop_config': return { theme: 'system', pause_sync: false, local_cache_limit_bytes: 0, upload_kbps_limit: 0, download_kbps_limit: 0, metered: false, files_on_demand: true, sync_overlays: true }
          case 'desktop_storage_summary': return { used_bytes: 48_300_000_000, quota_bytes: 200_000_000_000, cache_bytes: 1_200_000_000, pinned_bytes: 2_100_000_000 }
          case 'desktop_file_overview': return { total_files: 0, total_bytes: 0, by_status: [], recent: [], pinned_count: 0, pinned_bytes: 0 }
          case 'desktop_recent_files': return []
          case 'get_active_transfers': return []
          case 'account_activity_feed': return { events: [], total: 0, page: 1 }
          case 'account_notifications': return { notifications: [], unread_count: 0 }
          case 'get_known_folder_onboarding_seen': return true
          case 'get_known_folder_backup': return []
          case 'account_usage': return { storage_used_bytes: 200_000_000_000, storage_quota_bytes: 200_000_000_000 }
          case 'account_region': return { preferred_region: null, regions: [], choices: [] }
          default:
            if (name.startsWith('plugin:')) return null
            throw new Error(`fixture: no ${name}`)
        }
      },
    }
  }, {
    caps,
    summary: allSent ? { ...summary, transfers_active: 0, queue: { waiting: 0, gave_up: 0, held: [], not_downloaded: 0 } } : summary,
    unsent: allSent ? { ...unsent, total: 0, items: [] } : unsent,
    surface,
    locked,
  })
  return { page, errors }
}

async function shot(page, name, { fullPage = false } = {}) {
  const file = path.join(output, `${name}.png`)
  await page.screenshot({ path: file, fullPage })
  return file
}

const rowText = (page, id) => page.locator(`[data-unsent-id="${id}"]`).innerText()
const listedIds = (page) => page.locator('[data-unsent-id]').evaluateAll((els) => els.map((el) => el.getAttribute('data-unsent-id')))

/** Activity › Not sent with its three rows read. */
async function expectSection(page) {
  await page.locator('#not-sent').waitFor({ timeout: 5000 })
  await page.locator('[data-unsent-id="op-new"]').waitFor({ timeout: 5000 })
  assert.deepEqual(await listedIds(page), ROW_IDS, 'one row per change the count counts, held first')
  const del = await rowText(page, 'op-delete')
  for (const t of ['old plan.docx', 'Delete', 'Projects/2026', 'Stopped after repeated errors', 'Try again']) assert.ok(del.includes(t), `delete row: ${t} in ${JSON.stringify(del)}`)
  const edit = await rowText(page, 'op-edit')
  for (const t of ['budget.xlsx', 'Edit', 'Finance', 'On hold: your storage is full', 'Add storage']) assert.ok(edit.includes(t), `edit row: ${t} in ${JSON.stringify(edit)}`)
  const add = page.locator('[data-unsent-id="op-edit"] a', { hasText: 'Add storage' })
  assert.equal(await add.count(), 1, 'Add storage is a link')
  const fresh = await rowText(page, 'op-new')
  for (const t of ['IMG_0412.jpg', 'New file', 'Camera Roll', 'Sending · 34%']) assert.ok(fresh.includes(t), `new row: ${t} in ${JSON.stringify(fresh)}`)
  assert.ok(!fresh.includes('Try again'), 'a change being sent has no Try again')
}

/** No Amber in the section: the brand's one accent is for encryption state and primary actions only. */
async function amberIn(page) {
  return page.locator('#not-sent').evaluate((root) => {
    const hits = []
    for (const el of [root, ...root.querySelectorAll('*')]) {
      const s = getComputedStyle(el)
      for (const prop of ['color', 'backgroundColor', 'borderTopColor', 'borderBottomColor', 'borderLeftColor', 'borderRightColor']) {
        const v = s[prop]
        // oklch(0.82 0.17 84) renders near rgb(245, 183, 0); match the hue band, not one value.
        const m = v.match(/rgba?\((\d+), (\d+), (\d+)/)
        if (!m) continue
        const [r, g, b] = m.slice(1).map(Number)
        if (r > 200 && g > 140 && g < 210 && b < 90) hits.push(`${el.tagName} ${prop} ${v}`)
      }
    }
    return hits
  })
}

try {
  // ── Home: "Not sent 3" is a link to Activity › Not sent ─────────────────────
  {
    const { page, errors } = await open({ surface: 'main-app', viewport: { width: 1200, height: 860 } })
    await page.goto(`${base}/?window=main-app&platform=windows`)
    await page.getByText('Everything on this PC, at a glance.', { exact: false }).waitFor()
    const link = page.locator('a[href="#not-sent"]')
    await link.waitFor({ timeout: 5000 })
    assert.equal(await link.count(), 1)
    assert.equal((await link.textContent())?.trim(), '3', 'the link is the count')
    assert.equal(await link.getAttribute('title'), 'See which changes')
    results.push(['home-not-sent-link (an <a> reading 3)', await shot(page, 'home-not-sent-link')])
    await link.click()
    await expectSection(page)
    assert.equal(await page.getByRole('button', { name: 'Try again', exact: true }).count(), 1)
    results.push(['home-link-opens-activity-not-sent (3 rows)', await shot(page, 'home-link-opens-activity-not-sent')])
    const reads = await page.evaluate(() => window.calls.filter((c) => c.name === 'unsent_changes').map((c) => c.args))
    assert.ok(reads.length >= 1 && reads.every((a) => a.offset === 0 && a.limit === 100), `paged by 100: ${JSON.stringify(reads)}`)
    const homeReads = await page.evaluate(() => window.calls.findIndex((c) => c.name === 'unsent_changes'))
    assert.ok(homeReads > 0, 'the list is read once the section is shown')
    assert.deepEqual(await amberIn(page), [], 'no Amber in Not sent')
    assert.deepEqual(errors, [])
    await page.close()
  }
  // Home alone never reads the list (counts-only poll; the list is read while the section is on screen).
  {
    const { page, errors } = await open({ surface: 'main-app', viewport: { width: 1200, height: 860 } })
    await page.goto(`${base}/?window=main-app&platform=windows`)
    await page.locator('a[href="#not-sent"]').waitFor({ timeout: 5000 })
    await page.waitForTimeout(1500)
    const listReads = await page.evaluate(() => window.calls.filter((c) => c.name === 'unsent_changes').length)
    assert.equal(listReads, 0, 'Home does not read the list')
    results.push([`home-reads-no-list (unsent_changes x${listReads})`, await shot(page, 'home-reads-no-list')])
    assert.deepEqual(errors, [])
    await page.close()
  }

  // ── Activity › Not sent, light and dark; Try again → Queued ─────────────────
  for (const colorScheme of ['light', 'dark']) {
    const { page, errors } = await open({ surface: 'main-app', viewport: { width: 1200, height: 900 }, colorScheme })
    await page.goto(`${base}/?window=main-app&platform=windows`)
    await page.getByRole('button', { name: 'Activity', exact: true }).first().click()
    await expectSection(page)
    const name = `activity-not-sent${colorScheme === 'dark' ? '-dark' : ''}`
    results.push([name, await shot(page, name)])
    await page.locator('#not-sent').screenshot({ path: path.join(output, `${name}-section.png`) })
    results.push([`${name}-section`, path.join(output, `${name}-section.png`)])
    assert.deepEqual(await amberIn(page), [], 'no Amber in Not sent')

    if (colorScheme === 'light') {
      await page.locator('[data-unsent-id="op-delete"]').getByRole('button', { name: 'Try again', exact: true }).click()
      await page.locator('[data-unsent-id="op-delete"]', { hasText: 'Queued' }).waitFor({ timeout: 5000 })
      const retried = await page.evaluate(() => window.calls.filter((c) => c.name === 'retry_unsent_change').map((c) => c.args))
      assert.deepEqual(retried, [{ id: 'op-delete' }])
      assert.equal(await page.getByRole('button', { name: 'Try again', exact: true }).count(), 0)
      assert.equal((await listedIds(page)).length, 3, 'the retry does not change the count')
      results.push(['activity-not-sent-after-try-again (delete reads Queued; still 3 rows)', await shot(page, 'activity-not-sent-after-try-again')])
      await page.locator('#not-sent').screenshot({ path: path.join(output, 'activity-not-sent-after-try-again-section.png') })
    }
    assert.deepEqual(errors, [])
    await page.close()
  }

  // ── Locked: no names, one Unlock ───────────────────────────────────────────
  {
    const { page, errors } = await open({ surface: 'main-app', viewport: { width: 1200, height: 860 }, locked: true })
    await page.goto(`${base}/?window=main-app&platform=windows`)
    await page.getByRole('button', { name: 'Activity', exact: true }).first().click()
    await page.getByText('Beebeeb is locked. Unlock it to see which changes these are.', { exact: true }).waitFor({ timeout: 5000 })
    assert.equal(await page.locator('[data-unsent-id]').count(), 0, 'no rows while locked')
    await page.locator('#not-sent').getByRole('button', { name: 'Unlock', exact: true }).waitFor()
    results.push(['activity-not-sent-locked (0 rows, Unlock)', await shot(page, 'activity-not-sent-locked')])
    assert.deepEqual(errors, [])
    await page.close()
  }

  // ── Tray: the waiting, held and given-up lines are links ───────────────────
  for (const colorScheme of ['light', 'dark']) {
    const { page, errors } = await open({ surface: 'tray', viewport: { width: 400, height: 560 }, colorScheme })
    await page.goto(`${base}/?window=tray`)
    const links = page.locator('a[href="#not-sent"]')
    await links.first().waitFor({ timeout: 5000 })
    const texts = (await links.allTextContents()).map((t) => t.trim())
    assert.ok(texts.length >= 2, `held and given-up lines are links: ${JSON.stringify(texts)}`)
    assert.ok(texts.some((t) => /storage is full/i.test(t)), `the held line is a link: ${JSON.stringify(texts)}`)
    assert.ok(texts.some((t) => /stopped after repeated errors/i.test(t)), `the given-up line is a link: ${JSON.stringify(texts)}`)
    const name = `tray-not-sent-links${colorScheme === 'dark' ? '-dark' : ''}`
    results.push([`${name} ${JSON.stringify(texts)}`, await shot(page, name)])
    if (colorScheme === 'light') {
      await links.first().click()
      await page.waitForFunction(() => window.calls.some((c) => c.name === 'plugin:event|emit'), null, { timeout: 5000 })
      const calls = await page.evaluate(() => window.calls)
      assert.ok(calls.some((c) => c.name === 'show_main_app_window'), 'the link shows the main window')
      const emitted = calls.filter((c) => c.name === 'plugin:event|emit').map((c) => [c.args.event, c.args.payload])
      assert.deepEqual(emitted, [['windows-app:navigate', 'activity#not-sent']])
      const stored = await page.evaluate(() => Object.entries(window.localStorage).find(([, v]) => v === 'activity#not-sent'))
      assert.ok(stored, 'the target is stored for a main window that is not open yet')
      results.push([`tray-link-click (show_main_app_window, emit ${JSON.stringify(emitted[0])})`, await shot(page, 'tray-link-click')])
    }
    assert.deepEqual(errors, [])
    await page.close()
  }

  // ── A main window opened from the tray lands on Activity › Not sent ─────────
  {
    const { page, errors } = await open({ surface: 'main-app', viewport: { width: 1200, height: 860 } })
    await page.addInitScript(() => {
      try { window.localStorage.setItem('bb.windowsApp.nav', 'activity#not-sent') } catch {}
    })
    await page.goto(`${base}/?window=main-app&platform=windows`)
    await expectSection(page)
    results.push(['main-window-opened-from-tray (lands on Not sent)', await shot(page, 'main-window-opened-from-tray')])
    assert.deepEqual(errors, [])
    await page.close()
  }

  // ── After a link, Activity from the sidebar is Activity as before ──────────
  // Opened on Not sent when everything has been sent meanwhile: it says so. Leave Activity and come back from the
  // sidebar: no Not sent section (nothing is unsent), and its list is no longer read.
  {
    const { page, errors } = await open({ surface: 'main-app', viewport: { width: 1200, height: 860 }, allSent: true })
    await page.addInitScript(() => {
      try { window.localStorage.setItem('bb.windowsApp.nav', 'activity#not-sent') } catch {}
    })
    await page.goto(`${base}/?window=main-app&platform=windows`)
    await page.getByText('Every change on this PC has been sent.', { exact: true }).waitFor({ timeout: 5000 })
    await page.getByRole('button', { name: 'Home', exact: true }).first().click()
    await page.getByText('Everything on this PC, at a glance.', { exact: false }).waitFor()
    await page.evaluate(() => { try { window.localStorage.removeItem('bb.windowsApp.nav') } catch {} })
    await page.getByRole('button', { name: 'Activity', exact: true }).first().click()
    await page.getByText('Changes not sent yet, your account', { exact: false }).first().waitFor()
    await page.waitForTimeout(500)
    const before = await page.evaluate(() => window.calls.filter((c) => c.name === 'unsent_changes').length)
    assert.equal(await page.locator('#not-sent').count(), 0, 'no Not sent section on a later visit with nothing unsent')
    await page.waitForTimeout(5600)
    const after = await page.evaluate(() => window.calls.filter((c) => c.name === 'unsent_changes').length)
    assert.equal(after, before, 'the list is not polled once the section is gone')
    results.push([`activity-later-visit-all-sent (no section; unsent_changes reads stay ${after})`, await shot(page, 'activity-later-visit-all-sent')])
    assert.deepEqual(errors, [])
    await page.close()
  }
} finally {
  await browser.close()
}
for (const [what, file] of results) console.log(`ok  ${what}  ${file}`)
console.log(`browser cases: ${results.length} passed; 0 failed`)
