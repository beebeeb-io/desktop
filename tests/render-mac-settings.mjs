// Real-browser layout check of the macOS Settings window (task 1683 slice 4, spec sections 5, 8, 12).
// Local evidence, not a CI gate (Playwright is not a desktop dependency); the bun tests in
// tests/macSettings*.test.ts pin the same rules in CI.
//
//   node tests/render-mac-settings.mjs REPO OUTPUT [--target new|old]
//   PLAYWRIGHT_MODULE=/opt/node22/lib/node_modules/playwright/index.mjs  (as the other render-*.mjs)
//   FONTS_CSS=/home/user/evidence/1683/fonts.css   optional: declares Inter for the PNGs
//   DESIGN_PNG_DIR=/home/user/evidence/1683/png    optional: side-by-side with the design renders
//
// It bundles the REAL src/main.tsx (the same mount path the app uses) and loads it at a URL,
// with a synthetic Tauri IPC. One predicate measures both targets:
//
//   --target old  today's window: `?platform=macos` (App.tsx, `html.macos-flyout`) at 680 x 540.
//                 This is the RED: the predicate must fail with the numbers of spec section 8,
//                 1125 vs 540 (the document scrolls, the sidebar goes with it) and 485 vs 452
//                 (`.content` is wider than its pane). The run exits 1 when the layout is red.
//   --target new  `?window=settings-v2&platform=macos` at 560 wide, every tab at its natural
//                 height, a 60-character fake path in every field that can hold user data, in
//                 light and dark. Exits 1 on the first failed assertion, else prints a count line.
//
// The predicate (`report`): the document and every element that can clip or scroll
// (overflow other than visible, except a declared ellipsis) must have scrollWidth <= clientWidth;
// the document must not scroll vertically; at natural height the content pane needs no scrolling.
import assert from 'node:assert/strict'
import { existsSync } from 'node:fs'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { spawnSync } from 'node:child_process'
import path from 'node:path'
import { pathToFileURL } from 'node:url'

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ? pathToFileURL(process.env.PLAYWRIGHT_MODULE).href : 'playwright')
const args = process.argv.slice(2)
const targetFlag = args.indexOf('--target')
const target = targetFlag >= 0 ? args[targetFlag + 1] : 'new'
const [repo, output] = args.filter((a, i) => !a.startsWith('--') && args[i - 1] !== '--target')
assert(repo && output && ['new', 'old'].includes(target), 'usage: node tests/render-mac-settings.mjs REPO OUTPUT [--target new|old]')
await mkdir(path.join(output, 'png'), { recursive: true })

// ── Build the real entry ────────────────────────────────────────────────────
const outdir = path.join(output, 'bundle')
const built = spawnSync('bun', ['build', path.join(repo, 'src/main.tsx'), '--outdir', outdir, '--target', 'browser', '--format', 'iife', '--define', 'process.env.NODE_ENV="development"'], { encoding: 'utf8', cwd: repo })
assert.equal(built.status, 0, `bun build failed: ${built.stderr}`)
const fontsCss = process.env.FONTS_CSS ?? ''
// The RED run uses no web font, like the original repro (shoot.mjs), so its numbers are comparable.
const haveFonts = target === 'new' && fontsCss !== '' && existsSync(fontsCss)
if (target === 'new') console.log(haveFonts ? `fonts: ${fontsCss}` : 'WARNING fonts: FONTS_CSS unset or missing, the PNGs use a fallback face (assertions unaffected)')
const caps = JSON.parse(await readFile(path.join(repo, 'tests/fixtures/desktop-capabilities.json'), 'utf8')).macos
const pagePath = path.join(output, 'page.html')
await writeFile(
  pagePath,
  `<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width">${haveFonts ? `<link rel="stylesheet" href="${pathToFileURL(fontsCss).href}">` : ''}<link rel="stylesheet" href="bundle/main.css"></head><body><div id="root"></div><script src="bundle/main.js"></script></body></html>`,
)

// ── Fixture data: a 60-character fake path wherever user data can render ────
const FAKE_PATH = '/Users/guuslangelaar/Library/CloudStorage/Beebeeb-Drive/Docs'.padEnd(60, 's') // with slashes
const FAKE_TOKEN = 'ProjectFolderNameWithNoBreakOpportunitiesAtAll_0123456789ab'.padEnd(60, 'x') // no slash, no space
assert.equal(FAKE_PATH.length, 60)
assert.equal(FAKE_TOKEN.length, 60)
const LONG_EMAIL = `${'x'.repeat(48)}@example.eu` // 60 characters, no break opportunity before the @

const finderMissing = { installed: false, path: FAKE_PATH, status: 'missing', last_error: null, last_attempt_at: null, reason_category: null }
const FINDERS = {
  installed: { installed: true, path: FAKE_PATH, status: 'installed', last_error: null, last_attempt_at: 3, reason_category: null },
  missing: finderMissing,
  failed: { installed: false, path: FAKE_PATH, status: 'error', last_error: `Timed out waiting for the Beebeeb File Provider domain at ${FAKE_PATH}`, last_attempt_at: 2, reason_category: 'timeout' },
  failedLongCategory: { installed: false, path: FAKE_PATH, status: 'error', last_error: FAKE_PATH, last_attempt_at: 2, reason_category: FAKE_TOKEN },
  userDisabled: { installed: false, path: FAKE_PATH, status: 'error', last_error: 'Beebeeb is turned off in System Settings. Open Login Items & Extensions, turn on Beebeeb under File Providers, then try again.', last_attempt_at: 4, reason_category: 'user_disabled' },
}
const tree = (n) => [
  { id: 'f1', name: FAKE_TOKEN, is_folder: true, excluded: false, size_bytes: 1, file_count: 1, on_disk_bytes: 0, pinned: true, children: [
    { id: 'f2', name: FAKE_PATH, is_folder: true, excluded: false, size_bytes: 1, file_count: 1, on_disk_bytes: 0, pinned: false, children: [] },
  ] },
  ...Array.from({ length: n }, (_, i) => ({ id: `g${i}`, name: `Folder ${i + 1}`, is_folder: true, excluded: false, size_bytes: 1, file_count: 1, on_disk_bytes: 0, pinned: i % 3 === 0, children: [] })),
]
const snapshot = (over = {}) => ({
  phase: 'synced', generated_at: 1000, paused: false,
  account: { email: LONG_EMAIL, logged_in: true, vault_unlocked: true, auth_expired: false, ...(over.account ?? {}) },
  engine: { state: 'idle', files_remaining: 0, bytes_total: 0, bytes_done: 0, last_tick_ok_at: 990 },
  reason: null, finder: { setup: 'ready', reason: null, reason_line: null },
  storage: over.storage === undefined ? { used_bytes: 84_300_000_000, quota_bytes: 200_000_000_000, fetched_at: 900, stale: false } : over.storage,
  storage_full: false, pending_changes: 0, conflicts: { count: 0, files: [] }, activity: [],
})
const baseFixture = {
  finder: FINDERS.installed, tree: tree(2), snapshot: snapshot(),
  config: { upload_kbps_limit: 0, download_kbps_limit: 5000, pause_sync: false, notify_conflicts: true, notify_sync_complete: false, notify_quota_warnings: true, theme: 'system' },
  subscription: { plan: 'basic', billing_cycle: 'yearly', status: 'active', seats: 1, region: 'eu', current_period_end: '2026-10-14T00:00:00Z', quota_bytes: 200_000_000_000, used_bytes: 84_300_000_000 },
  repair: { removed_file_provider_domain: true, disabled_autostart: true, removed_socket: true, removed_cache_files: 0, skipped_cache_files: 0, pending_operations_preserved: 3, warnings: [] },
  update: { status: 'up_to_date', current_version: '0.8.6', channel: 'stable' },
}
// `tab` and `act` drive the page into the state; `has`/`hasNot` are instrument checks that the
// state is the one named (a green layout check on the wrong state would prove nothing).
const SCENARIOS = [
  { name: 'general', tab: 'General', shot: 'general', has: ['Open Beebeeb at login', 'Tell me about', 'Sync finished'] },
  { name: 'account', tab: 'Account', shot: 'account', has: [LONG_EMAIL, 'Basic plan · renews 14 Oct 2026', 'Vault is unlocked', 'Sign out…'] },
  { name: 'account-locked', tab: 'Account', fx: { snapshot: snapshot({ account: { vault_unlocked: false }, storage: null }) }, has: ['Vault is locked', 'Unlock'] },
  { name: 'sync', tab: 'Sync', shot: 'sync', has: ['Beebeeb in Finder', 'Added', 'Repair…', '2 folders', 'Choose folders…', 'Speed'], hasNot: [FAKE_PATH] },
  { name: 'sync-failed', tab: 'Sync', fx: { finder: FINDERS.failed }, has: ['Couldn’t add Beebeeb to Finder', 'macOS didn’t respond in time.', 'reason: timeout', 'Try again'], hasNot: [FAKE_PATH, 'File Provider'], errorSurfaces: 1 },
  { name: 'sync-failed-long-category', tab: 'Sync', fx: { finder: FINDERS.failedLongCategory }, has: ['Couldn’t add Beebeeb to Finder', 'macOS couldn’t finish setting it up.'], hasNot: [FAKE_PATH], errorSurfaces: 1 },
  { name: 'sync-user-disabled', tab: 'Sync', fx: { finder: FINDERS.userDisabled }, has: ['Open Login Items & Extensions'], errorSurfaces: 0 },
  { name: 'sync-missing', tab: 'Sync', fx: { finder: FINDERS.missing }, has: ['Add to Finder'], errorSurfaces: 0 },
  { name: 'sync-repaired', tab: 'Sync', fx: { repair: { ...baseFixture.repair, warnings: [`Could not remove ${FAKE_PATH}`] } }, act: async (p) => { await p.getByRole('button', { name: 'Repair…' }).click(); await p.getByRole('button', { name: 'Repair', exact: true }).click(); await p.getByText('3 changes waiting to upload were kept.').waitFor() }, has: [FAKE_PATH, 'Add to Finder'] },
  { name: 'sync-chooser', tab: 'Sync', fx: { tree: tree(12) }, act: async (p) => { await p.getByRole('button', { name: 'Choose folders…' }).click(); await p.getByRole('dialog').waitFor() }, has: ['Keep on this Mac', FAKE_TOKEN] },
  { name: 'sync-repair-confirm', tab: 'Sync', act: async (p) => { await p.getByRole('button', { name: 'Repair…' }).click(); await p.getByRole('dialog').waitFor() }, has: ['Repair Beebeeb in Finder?', 'turns off Open Beebeeb at login'] },
  { name: 'about', tab: 'About', shot: 'about', has: ['Beebeeb for Mac', 'Version 0.8.6', 'Get help', 'Export a support bundle', 'Paths, file and folder names and sign-in tokens are removed'] },
  { name: 'about-update', tab: 'About', fx: { update: { status: 'update_available', current_version: '0.8.6', channel: 'stable', version: '0.8.7', body: '', release_notes_url: 'https://example.invalid' } }, act: async (p) => { await p.getByRole('button', { name: 'Check for updates' }).click(); await p.getByText('Version 0.8.7 is available.').waitFor() }, has: ['Restart to update'] },
]

// ── In the page ─────────────────────────────────────────────────────────────
function installBackend({ caps, fx }) {
  window.__calls = []
  window.__fx = fx
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} }
  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: 'settings' }, currentWebview: { label: 'settings' } },
    transformCallback: () => 1,
    unregisterCallback() {},
    invoke: async (name, a) => {
      window.__calls.push({ name, args: a })
      const f = window.__fx
      switch (name) {
        case 'desktop_capabilities': return caps
        case 'desktop_platform': return 'macos'
        case 'app_version': return '0.8.6'
        case 'desktop_config': return null
        case 'sync_status': return { session_revision: 1, logged_in: true, vault_unlocked: true, engine: 'stopped', sync_root: null, syncing: 0, cloud_only: 0, conflicts: 0, auth_expired: false }
        case 'desktop_storage_summary': return { used_bytes: 84300000000, quota_bytes: 200000000000, cache_bytes: 0, pinned_bytes: 0 }
        case 'get_desktop_config': return f.config
        case 'set_desktop_config': f.config = a.config; return null
        case 'autostart_enabled': return true
        case 'toggle_autostart': return false
        case 'finder_location_state': return f.finder
        case 'install_finder_location': return f.finder
        case 'reset_macos_integration': f.finder = { installed: false, path: null, status: 'missing', last_error: null, last_attempt_at: null, reason_category: null }; return f.repair
        case 'list_remote_tree': return f.tree
        case 'set_recursive_pin': return null
        case 'popover_snapshot': return f.snapshot
        case 'account_subscription': return f.subscription
        case 'check_for_updates_now': return f.update
        case 'lock_vault': case 'unlock_vault': case 'clear_session': case 'open_login_items_and_extensions_settings': return null
        case 'report_problem': return { path: '/tmp/x.json', email_opened: true }
        default:
          if (name.startsWith('plugin:')) return 1
          return null
      }
    },
  }
}

// The one predicate. Runs in the page.
function report() {
  const scrolling = document.scrollingElement
  const clips = (el) => {
    const s = getComputedStyle(el)
    if (s.textOverflow === 'ellipsis') return false // a declared truncation, not a layout defect
    return [s.overflowX, s.overflowY].some((v) => v !== 'visible')
  }
  const name = (el) => (el === document.documentElement ? 'html' : el === document.body ? 'body' : `${el.tagName.toLowerCase()}${el.id ? '#' + el.id : ''}${typeof el.className === 'string' && el.className ? '.' + el.className.trim().split(/\s+/).join('.') : ''}`)
  const containers = [document.documentElement, document.body, ...document.querySelectorAll('body *')].filter((el) => el === document.documentElement || el === document.body || clips(el))
  const rows = containers
    .filter((el) => el.clientWidth > 0)
    .map((el) => ({ el: name(el), scrollWidth: el === document.documentElement ? scrolling.scrollWidth : el.scrollWidth, clientWidth: el === document.documentElement ? scrolling.clientWidth : el.clientWidth, scrollHeight: el.scrollHeight, clientHeight: el.clientHeight }))
  const pane = document.querySelector('.ms-pane')
  const toolbar = document.querySelector('.ms-toolbar')
  // `scrollHeight` of a pane in a tall window is the window, not the content, so the natural
  // height is measured from the content: the bottom of the lowest in-flow child plus the pane's
  // bottom padding. A dialog (position: fixed) is not part of the content.
  function contentHeight(el) {
    const top = el.getBoundingClientRect().top
    const bottoms = [...el.children].filter((c) => getComputedStyle(c).position !== 'fixed').map((c) => c.getBoundingClientRect().bottom)
    if (bottoms.length === 0) return 0
    return Math.ceil(Math.max(...bottoms) - top + parseFloat(getComputedStyle(el).paddingBottom))
  }
  return {
    innerWidth, innerHeight,
    docScrollHeight: scrolling.scrollHeight, docScrollTop: scrolling.scrollTop, docScrollWidth: scrolling.scrollWidth,
    wide: rows.filter((r) => r.scrollWidth > r.clientWidth),
    rows,
    pane: pane ? { scrollHeight: pane.scrollHeight, clientHeight: pane.clientHeight, scrollTop: pane.scrollTop, contentHeight: contentHeight(pane) } : null,
    toolbarTop: toolbar ? toolbar.getBoundingClientRect().top : null,
    toolbarHeight: toolbar ? toolbar.getBoundingClientRect().height : null,
    errorSurfaces: document.querySelectorAll('[data-error-surface]').length,
    text: document.body.innerText,
  }
}

const browser = await chromium.launch({ headless: true, ...(process.env.CHROME_PATH ? { executablePath: process.env.CHROME_PATH } : {}) })
let passed = 0
const failures = []
function check(ok, label, detail = '') {
  if (ok) { passed += 1; return }
  failures.push(`${label}${detail ? ` (${detail})` : ''}`)
}

async function open(url, viewport, scheme, fx) {
  const context = await browser.newContext({ viewport, colorScheme: scheme, deviceScaleFactor: 1, locale: 'nl-NL' })
  const page = await context.newPage()
  const errors = []
  page.on('pageerror', (e) => errors.push(String(e)))
  page.on('console', (m) => { if (m.type() === 'error') errors.push(`console: ${m.text()}`) })
  await page.addInitScript(installBackend, { caps, fx: structuredClone(fx) })
  await page.goto(`${pathToFileURL(pagePath).href}?${url}`)
  return { context, page, errors }
}

try {
  if (target === 'old') {
    // RED: today's window, same predicate, 680 x 540.
    for (const scheme of ['light', 'dark']) {
      const { context, page, errors } = await open('platform=macos', { width: 680, height: 540 }, scheme, baseFixture)
      await page.waitForSelector('.app-shell', { timeout: 8000 }).catch(async (e) => {
        throw new Error(`old ${scheme}: .app-shell never rendered. page errors: ${JSON.stringify(errors)}; body: ${(await page.evaluate(() => document.body.innerText)).slice(0, 300)}`, { cause: e })
      })
      await page.waitForTimeout(1500)
      const status = await page.evaluate(report)
      const doc = { docScrollHeight: status.docScrollHeight, innerHeight: status.innerHeight }
      const content = status.rows.find((r) => r.el.startsWith('main.content'))
      console.log(`[old ${scheme}] Status page: document scrollHeight ${doc.docScrollHeight} vs innerHeight ${doc.innerHeight}; .content ${content?.scrollWidth} vs ${content?.clientWidth}`)
      check(status.docScrollHeight <= status.innerHeight, `old ${scheme} Status: the document must not scroll`, `${status.docScrollHeight} vs ${status.innerHeight}`)
      await page.locator('.sidebar button').filter({ hasText: 'Finder location' }).first().click()
      await page.waitForTimeout(600)
      const finder = await page.evaluate(report)
      const fc = finder.rows.find((r) => r.el.startsWith('main.content'))
      console.log(`[old ${scheme}] Finder page: .content ${fc?.scrollWidth} vs ${fc?.clientWidth}; document ${finder.docScrollHeight} vs ${finder.innerHeight}`)
      check(finder.wide.length === 0, `old ${scheme} Finder: no scroll container wider than its box`, finder.wide.map((r) => `${r.el} ${r.scrollWidth} vs ${r.clientWidth}`).join('; '))
      assert.deepEqual(errors, [], `old ${scheme}: page errors`)
      await page.screenshot({ path: path.join(output, 'png', `old-${scheme}-finder.png`) })
      await context.close()
    }
  } else {
    // ONLY=a,b limits the run to named scenarios (used for mutation runs); a full run does not set it.
    const only = process.env.ONLY ? process.env.ONLY.split(',') : null
    const chosen = SCENARIOS.filter((sc) => only === null || only.includes(sc.name))
    assert(chosen.length > 0, `ONLY matched no scenario: ${process.env.ONLY}`)
    for (const scheme of ['light', 'dark']) {
      for (const sc of chosen) {
        const fx = { ...baseFixture, ...(sc.fx ?? {}) }
        const label = `${sc.name} ${scheme}`
        // 1. Tall window: measure the natural height of this state.
        const tall = await open('window=settings-v2&platform=macos', { width: 560, height: 1600 }, scheme, fx)
        let natural
        try {
          await tall.page.waitForSelector('.ms-shell')
          await tall.page.getByRole('tab', { name: sc.tab, exact: true }).click()
          await tall.page.waitForTimeout(450)
          if (sc.act) await sc.act(tall.page)
          await tall.page.waitForTimeout(250)
          const r = await tall.page.evaluate(report)
          natural = Math.ceil(r.toolbarHeight + r.pane.contentHeight)
          check(r.docScrollHeight <= r.innerHeight, `${label}: tall window, the document does not scroll`)
          for (const text of sc.has ?? []) check(r.text.includes(text), `${label}: shows "${text}"`)
          for (const text of sc.hasNot ?? []) check(!r.text.includes(text), `${label}: does not show "${text}"`)
          if (sc.errorSurfaces !== undefined) check(r.errorSurfaces === sc.errorSurfaces, `${label}: ${sc.errorSurfaces} error surface(s)`, `found ${r.errorSurfaces}`)
          check(tall.errors.length === 0, `${label}: no page errors`, tall.errors.join(' | '))
        } finally { await tall.context.close() }
        // 2. The window at exactly that height: nothing scrolls, nothing is wider than its box.
        const fit = await open('window=settings-v2&platform=macos', { width: 560, height: natural }, scheme, fx)
        try {
          await fit.page.waitForSelector('.ms-shell')
          await fit.page.getByRole('tab', { name: sc.tab, exact: true }).click()
          await fit.page.waitForTimeout(450)
          if (sc.act) await sc.act(fit.page)
          await fit.page.waitForTimeout(250)
          const r = await fit.page.evaluate(report)
          check(r.wide.length === 0, `${label} @${natural}: scrollWidth <= clientWidth on every scroll container (${r.rows.length} checked)`, r.wide.map((w) => `${w.el} ${w.scrollWidth} vs ${w.clientWidth}`).join('; '))
          check(r.rows.length >= 3, `${label}: the predicate found its containers`, `${r.rows.length}`)
          check(r.docScrollHeight <= r.innerHeight, `${label} @${natural}: the document does not scroll`, `${r.docScrollHeight} vs ${r.innerHeight}`)
          check(r.pane.scrollHeight <= r.pane.clientHeight, `${label} @${natural}: the pane needs no scrolling at its natural height`, `${r.pane.scrollHeight} vs ${r.pane.clientHeight}`)
          check(fit.errors.length === 0, `${label}: no page errors`, fit.errors.join(' | '))
          if (sc.shot) {
            // A drawn approximation of the native traffic lights, for the PNG only.
            await fit.page.evaluate(() => {
              const tl = document.createElement('div')
              tl.setAttribute('style', 'position:absolute;left:18px;top:16px;display:flex;gap:8px')
              tl.innerHTML = ['#ff5f57', '#febc2e', '#28c840'].map((c) => `<div style="width:12px;height:12px;border-radius:50%;background:${c}"></div>`).join('')
              document.querySelector('.ms-toolbar').style.position = 'relative'
              document.querySelector('.ms-toolbar').appendChild(tl)
            })
            await fit.page.evaluate(() => document.fonts.ready)
            await fit.page.screenshot({ path: path.join(output, 'png', `${sc.shot}-${scheme}.png`) })
            await writeFile(path.join(output, 'png', `${sc.shot}-${scheme}.height.txt`), `${natural}\n`)
          } else if (scheme === 'light') {
            await fit.page.screenshot({ path: path.join(output, 'png', `state-${sc.name}-light.png`) })
          }
        } finally { await fit.context.close() }
        // 3. A short window: the pane scrolls, the toolbar and the document do not.
        const short = await open('window=settings-v2&platform=macos', { width: 560, height: 320 }, scheme, fx)
        try {
          await short.page.waitForSelector('.ms-shell')
          await short.page.getByRole('tab', { name: sc.tab, exact: true }).click()
          await short.page.waitForTimeout(450)
          if (sc.act) await sc.act(short.page)
          await short.page.evaluate(() => { const p = document.querySelector('.ms-pane'); p.scrollTop = p.scrollHeight })
          const r = await short.page.evaluate(report)
          check(r.toolbarTop === 0, `${label} @320: the toolbar stays at the top after scrolling`, `top ${r.toolbarTop}`)
          check(r.docScrollTop === 0 && r.docScrollHeight <= r.innerHeight, `${label} @320: only the pane scrolls, never the document`, `doc ${r.docScrollTop}/${r.docScrollHeight} vs ${r.innerHeight}`)
          check(r.wide.length === 0, `${label} @320: scrollWidth <= clientWidth on every scroll container`, r.wide.map((w) => `${w.el} ${w.scrollWidth} vs ${w.clientWidth}`).join('; '))
        } finally { await short.context.close() }
      }
    }
  }
} finally {
  await browser.close()
}

if (failures.length > 0) {
  console.log(`RED (${target}): ${failures.length} failed, ${passed} passed`)
  for (const f of failures) console.log(`  FAIL ${f}`)
  process.exit(1)
}
console.log(`${passed} passed, 0 failed (mac settings in Chromium, target ${target})`)
