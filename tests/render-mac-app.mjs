// Real-browser check of the macOS Beebeeb window (task 1683 slices 1 and 4; spec docs/specs/2026-10-24-macos-two-surface-app.md
// §7, §8, §11; hi-fi design/hifi/macos-two-surface.html sections 3 to 5). Slice 4 adds Review (rows, badge, the conflict
// and version-history sheets, empty and locked) and Trash (rows, the delete sheet, empty and locked). Local evidence, not a CI gate (Playwright is
// not a desktop dependency); tests/macApp.test.tsx and tests/macSettingsLayout.test.ts pin the same rules in CI.
//
//   node tests/render-mac-app.mjs REPO OUTPUT
//   PLAYWRIGHT_MODULE=/path/to/playwright/index.mjs   (as the other render-*.mjs)
//   HIFI_PNG_DIR=/path/to/hifi-figures               optional: side by side with the hi-fi's figures
//
// It bundles the REAL src/main.tsx (the mount path the app uses), loads `?window=main&platform=macos` with a synthetic
// Tauri IPC, drives the window to each state through its own controls (the gear, the sidebar, the buttons), and
// checks: nothing that clips or scrolls is wider than its box; the document never scrolls; the window shows the words
// it must and none it must not; no page errors. Each state is drawn at 920 x 620 (the default size) in light and dark,
// and the Settings states again at 760 x 520 (the minimum). A drawn copy of the traffic lights sits over the sidebar in
// the PNGs only. Exits 1 on any failed check, else prints a count line.
import assert from 'node:assert/strict'
import { existsSync } from 'node:fs'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { spawnSync } from 'node:child_process'
import path from 'node:path'
import { pathToFileURL } from 'node:url'

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ? pathToFileURL(process.env.PLAYWRIGHT_MODULE).href : 'playwright')
const [repo, output] = process.argv.slice(2)
assert(repo && output, 'usage: node tests/render-mac-app.mjs REPO OUTPUT')
await mkdir(path.join(output, 'png'), { recursive: true })

const outdir = path.join(output, 'bundle')
const built = spawnSync('bun', ['build', path.join(repo, 'src/main.tsx'), '--outdir', outdir, '--target', 'browser', '--format', 'iife', '--define', 'process.env.NODE_ENV="development"'], { encoding: 'utf8', cwd: repo })
assert.equal(built.status, 0, `bun build failed: ${built.stderr}`)
const caps = JSON.parse(await readFile(path.join(repo, 'tests/fixtures/desktop-capabilities.json'), 'utf8')).macos
const pagePath = path.join(output, 'page.html')
await writeFile(pagePath, '<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width"><link rel="stylesheet" href="bundle/main.css"></head><body><div id="root"></div><script src="bundle/main.js"></script></body></html>')

const LONG_EMAIL = `${'x'.repeat(48)}@example.eu` // 60 characters, no break before the @
const KEPT_OURS = '/Users/sam/Library/Containers/io.beebeeb.app/Data/Documents/Changes not sent/2026-10-10 at 14.05.12'
const KEPT_MACOS = '/Users/sam/Library/CloudStorage/Beebeeb-Beebeeb (10-10-2026 14.05)'
const ago = (seconds) => Math.floor(Date.now() / 1000) - seconds
const activityRows = () => [
  { id: 'a1', name: 'Q3 ledger reconciliation.xlsx', folder: 'Investigations', path: 'Investigations/Q3 ledger reconciliation.xlsx', direction: 'up', state: 'done', at: ago(120), size_bytes: 48_000, done_bytes: 48_000, total_bytes: 48_000 },
  { id: 'a2', name: 'Board pack, October.pdf', folder: 'Board', path: 'Board/Board pack, October.pdf', direction: 'down', state: 'active', at: ago(10), size_bytes: 9_000_000, done_bytes: 5_580_000, total_bytes: 9_000_000 },
  { id: 'a3', name: 'Site visit, north wing.mov', folder: 'Photos', path: 'Photos/Site visit, north wing.mov', direction: 'up', state: 'failed', at: ago(3600 * 3), size_bytes: null, done_bytes: null, total_bytes: null },
  { id: 'a4', name: 'notes.md', folder: '', path: 'notes.md', direction: 'down', state: 'done', at: ago(3600 * 30), size_bytes: 3_000, done_bytes: 3_000, total_bytes: 3_000 },
]
const snapshot = (over = {}) => ({
  phase: 'synced', generated_at: 1000, paused: false,
  engine: { state: 'idle', files_remaining: 0, bytes_total: 0, bytes_done: 0, last_tick_ok_at: Math.floor(Date.now() / 1000) - 70 },
  reason: null, finder: { setup: 'ready', reason: null, reason_line: null },
  storage: { used_bytes: 84_300_000_000, quota_bytes: 200_000_000_000, fetched_at: 900, stale: false },
  storage_full: false, pending_changes: 0, conflicts: { count: 0, files: [] }, activity: activityRows(),
  ...over,
  account: { email: 'sam@example.eu', logged_in: true, vault_unlocked: true, auth_expired: false, ...(over.account ?? {}) },
})
// "Today at hh:mm" in the page's own clock (the browser and this script share the machine's time zone).
const todayAt = (h, m) => { const d = new Date(); d.setHours(h, m, 0, 0); return Math.floor(d.getTime() / 1000) }
const LONG_NAME = `${'Quarterly-investigation-summary-'.repeat(4)}final.xlsx`
const reviewEntries = [
  { id: 'conflict:f1', file_id: 'f1', file_name: 'Board notes, 29 Sept.md', folder: 'Team shared', kind: 'conflict', status: 'needs review', updated_at: todayAt(9, 12), detail: 'Changed on two devices.', action: 'open_conflict' },
  { id: 'op:2', file_id: 'f2', file_name: 'Site visit, north wing.mov', folder: 'Photos', kind: 'failed_upload', status: 'upload review', updated_at: todayAt(8, 47), detail: 'Couldn’t upload, will retry', action: 'review_upload' },
]
const reviewAllKinds = [
  ...reviewEntries,
  { id: 'op:3', file_id: 'f3', file_name: 'Archive 2025.zip', folder: null, kind: 'quota_failure', status: 'quota blocked', updated_at: todayAt(8, 30), detail: 'This change can’t upload until you free up space or add storage. It stays on this Mac.', action: 'review_upload' },
  { id: 'op:4', file_id: 'f4', file_name: LONG_NAME, folder: 'Quarterly investigations, shared with the whole finance team in Rotterdam', kind: 'stale_base', status: 'stale base kept local', updated_at: todayAt(8, 2), detail: 'This file changed somewhere else too. Your version is kept on this Mac.', action: 'review_upload' },
  { id: 'op:5', file_id: 'f5', file_name: 'Plan.pdf', kind: 'auth_failure', status: 'sign-in needed', updated_at: todayAt(7, 40), detail: 'Your session has expired. Sign in again to resume syncing.', action: 'sign_in_again' },
]
const conflictPreview = {
  is_text: true,
  local: { size_bytes: 4096, text: '## Agenda\n1. Budget\n2. Hiring\n3. Roadmap\n4. AOB (added on this Mac)\n' },
  remote: { size_bytes: 4096, text: '## Agenda\n1. Budget\n2. Roadmap (moved up)\n3. Hiring\n4. AOB\n' },
}
const searchHits = { query: 'q3', index_state: 'ready', indexed_file_count: 3, results: [
  { file_id: 'f1', name: 'Q3 ledger reconciliation.xlsx', rel_path: 'Investigations/Q3 ledger reconciliation.xlsx', status: 'synced', size_bytes: 48_000, modified_at: todayAt(9, 0), shared_with_me: false },
  { file_id: 'f2', name: 'Q3 notes.md', rel_path: 'Q3 notes.md', status: 'synced', size_bytes: 3_000, modified_at: todayAt(8, 0), shared_with_me: false },
] }
const versionList = { file_id: 'f1', current_version: 5, versions: [
  { id: 'v5', version_number: 5, is_current: true, created_at: new Date(todayAt(9, 0) * 1000).toISOString(), size_bytes: 52_000, restorable: true },
  { id: 'v4', version_number: 4, is_current: false, created_at: new Date((todayAt(9, 0) - 86400) * 1000).toISOString(), size_bytes: 48_000, restorable: true },
  { id: 'v3', version_number: 3, is_current: false, created_at: new Date((todayAt(9, 0) - 14 * 86400) * 1000).toISOString(), size_bytes: 44_000, restorable: true },
] }
const trashItems = [
  { id: 't1', name: 'Draft 3, interview questions.docx', is_folder: false, size_bytes: 48_000, updated_at: String(todayAt(9, 0) - 86400) },
  { id: 't2', name: 'Old scans', is_folder: true, size_bytes: 1_400_000_000, updated_at: String(todayAt(9, 0) - 13 * 86400) },
  { id: 't3', name: LONG_NAME, is_folder: false, size_bytes: 2_300_000, updated_at: String(todayAt(9, 0)) },
]
const base = {
  // When the fixtures were made: the page shifts every "how long ago" in a snapshot by the time since, so a scenario that
  // runs minutes into this script still reads "1 min ago" and "just now", not what the clock says by then.
  t0: Math.floor(Date.now() / 1000),
  snapshot: snapshot(),
  overview: { by_status: [{ status: 'cloud_only', count: 8311, bytes: 1 }, { status: 'local', count: 1204, bytes: 1 }], recent: [], total_files: 9515, total_bytes: 2, pinned_count: 40, pinned_bytes: 1, pinned_folders: 3 },
  review: reviewEntries, conflictPreview, search: searchHits, versions: versionList, trash: trashItems,
  finder: { setup: 'ready', reason: null, launch_location: 'applications', attempt: 1, max_attempts: 1, last_failure: null },
  config: { upload_kbps_limit: 0, download_kbps_limit: 5000, pause_sync: false, notify_conflicts: true, notify_sync_complete: false, notify_quota_warnings: true, theme: 'system' },
  subscription: { plan: 'basic', billing_cycle: 'yearly', status: 'active', seats: 1, region: 'eu', current_period_end: '2026-11-14T00:00:00Z', quota_bytes: 200_000_000_000, used_bytes: 84_300_000_000 },
  keptUnsent: null, kept: null,
  // The shape `account_region` answers with (Rust's DTO, as tests/render-location-by-country.mjs maps it).
  region: {
    preferred_region: null,
    region_strict: false,
    regions: [{ continent: 'europe', display_name: 'Europe', country: 'Germany', is_default: true }],
    choices: [
      { id: 'europe', kind: 'region', name: 'Europe', country: null, country_code: null, status: 'live', selectable: true, is_default: true },
      { id: 'de', kind: 'country', name: 'Germany', country: 'Germany', country_code: 'DE', status: 'live', selectable: true, is_default: false },
    ],
  },
}

function installBackend({ caps, fx }) {
  window.__fx = fx
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} }
  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } },
    transformCallback: () => 1,
    unregisterCallback() {},
    invoke: async (name, a) => {
      const f = window.__fx
      switch (name) {
        case 'desktop_capabilities': return caps
        case 'desktop_platform': return 'macos'
        case 'app_version': return '0.8.12'
        case 'sync_status': return { session_revision: 1, logged_in: f.snapshot.account.logged_in, vault_unlocked: f.snapshot.account.vault_unlocked, engine: 'running', sync_root: null, syncing: 0, cloud_only: 0, conflicts: 0, auth_expired: f.snapshot.account.auth_expired, engine_refusal: null }
        case 'popover_snapshot': {
          const shift = Math.floor(Date.now() / 1000) - f.t0
          const snap = f.snapshot
          return { ...snap, engine: { ...snap.engine, last_tick_ok_at: snap.engine.last_tick_ok_at === null ? null : snap.engine.last_tick_ok_at + shift }, activity: snap.activity.map((row) => ({ ...row, at: row.at + shift })) }
        }
        case 'account_subscription': return f.subscription
        case 'get_desktop_config': return f.config
        case 'desktop_config': return f.config
        case 'set_desktop_config': f.config = { ...f.config, ...a.config }; return null
        case 'autostart_enabled': return true
        case 'finder_setup_state': return f.finder
        case 'kept_unsent_folder': return f.keptUnsent
        case 'kept_unsynced_folder': return f.kept
        case 'list_remote_tree': return [{ id: 'f1', name: 'Photos', is_folder: true, excluded: false, size_bytes: 1, file_count: 1, on_disk_bytes: 0, pinned: true, children: [] }]
        case 'account_region': return f.region
        case 'desktop_file_overview': return f.overview
        case 'list_version_conflict_center': return f.review
        case 'conflict_content_preview': return f.conflictPreview
        case 'desktop_search_files': return f.search
        case 'list_file_versions': return f.versions
        case 'desktop_trash_list': return f.trash
        case 'consume_main_navigation': return null
        case 'consume_menu_update_check': return false
        case 'consume_sign_out_flow_request': return false
        case 'main_window_state': return null
        case 'sign_out_count': return { askId: 'a0', files: 0, copiedFiles: 0, others: 0, missingFiles: 0, earlierFolders: [], canSend: false, notSending: null }
        default:
          if (name.startsWith('plugin:')) return 1
          return null
      }
    },
  }
}

function report() {
  const scrolling = document.scrollingElement
  const clips = (el) => {
    const s = getComputedStyle(el)
    if (s.textOverflow === 'ellipsis') return false
    return [s.overflowX, s.overflowY].some((v) => v !== 'visible')
  }
  const name = (el) => (el === document.documentElement ? 'html' : `${el.tagName.toLowerCase()}${typeof el.className === 'string' && el.className ? '.' + el.className.trim().split(/\s+/).join('.') : ''}`)
  const containers = [document.documentElement, document.body, ...document.querySelectorAll('body *')].filter((el) => el === document.documentElement || el === document.body || clips(el))
  const rows = containers.filter((el) => el.clientWidth > 0).map((el) => ({
    el: name(el),
    scrollWidth: el === document.documentElement ? scrolling.scrollWidth : el.scrollWidth,
    clientWidth: el === document.documentElement ? scrolling.clientWidth : el.clientWidth,
  }))
  const sidebar = document.querySelector('.mw-sidebar')
  return {
    innerHeight, docScrollHeight: scrolling.scrollHeight,
    wide: rows.filter((r) => r.scrollWidth > r.clientWidth),
    rows: rows.length,
    sidebarWidth: sidebar ? Math.round(sidebar.getBoundingClientRect().width) : null,
    barHeight: (() => { const b = document.querySelector('.mw-storage .ms-bar'); return b ? Math.round(b.getBoundingClientRect().height) : null })(),
    toolbarHeight: (() => { const t = document.querySelector('.mw-toolbar'); return t ? Math.round(t.getBoundingClientRect().height) : null })(),
    dialogs: document.querySelectorAll('[role="dialog"]').length,
    sidebarButtons: sidebar ? [...sidebar.querySelectorAll('button')].map((b) => b.getAttribute('aria-label') ?? b.innerText.trim()) : [],
    text: document.body.innerText,
  }
}

const nav = (name) => async (p) => { await p.locator('.mw-sidebar').getByRole('button', { name }).first().click() }
const review = nav(/^Review/)
const decide = async (p) => { await review(p); await p.getByRole('button', { name: 'Decide…' }).click(); await p.getByRole('dialog').waitFor(); await p.waitForFunction(() => !document.body.innerText.includes('Loading both versions')) }
const history = async (p) => { await review(p); await p.getByRole('button', { name: 'Version history…' }).click(); await p.getByRole('dialog').waitFor() }
const trash = nav('Trash')
const gear = async (p) => { await p.getByRole('button', { name: 'Settings', exact: true }).click() }
const section = (label) => async (p) => { await gear(p); await p.getByRole('button', { name: label }).first().click() }

const SCENARIOS = [
  { name: 'home', bar: true, has: ['Home', 'End-to-end encrypted', 'Up to date', 'Last checked 1 min ago', 'Beebeeb in Finder', 'Your vault appears under Locations in Finder.', 'Open in Finder', 'On this Mac 1.204 files', 'Online-only 8.311', 'Kept on this Mac 3 folders', 'Needs your attention', 'CONFLICT', 'Board notes, 29 Sept.md', 'Team shared · today 09:12 · Changed on two devices.', 'Decide…', 'Storage', '84,3 GB of 200 GB', 'Recent activity', 'Q3 ledger reconciliation.xlsx', 'Uploaded · Investigations · 2 minutes ago', 'Downloading · 62 % · Board · just now', 'Couldn’t sync · Photos · 3 hours ago', 'Downloaded · yesterday', 'sam@example.eu', 'Basic plan'], hasNot: ['control center', 'daemon', 'On this PC', '0 files'], min: true, hifi: 'hifi-s3-fig0' },
  { name: 'home-many-attention', fx: { review: reviewAllKinds }, has: ['Needs your attention', 'Board notes, 29 Sept.md', 'Archive 2025.zip', '2 more in Review'], hasNot: ['Plan.pdf'] },
  { name: 'home-decide-sheet', act: async (p) => { await p.getByRole('button', { name: 'Decide…' }).first().click(); await p.getByRole('dialog').waitFor(); await p.waitForFunction(() => !document.body.innerText.includes('Loading both versions')) }, has: ['Conflict: Board notes, 29 Sept.md', 'This Mac', 'Other device', 'Keep both'], dialogs: 1, min: true },
  { name: 'home-quiet', fx: { review: [], overview: { by_status: [], recent: [], total_files: 0, total_bytes: 0, pinned_count: 0, pinned_bytes: 0, pinned_folders: 0 }, snapshot: snapshot({ activity: [] }) }, has: ['Up to date', 'Beebeeb in Finder', 'Storage'], hasNot: ['Needs your attention', 'Recent activity', 'On this Mac', '0 files', 'Nothing needs your attention'] },
  { name: 'home-no-folder-count', fx: { overview: { by_status: [{ status: 'local', count: 1, bytes: 1 }], recent: [], total_files: 1, total_bytes: 1, pinned_count: 0, pinned_bytes: 0 } }, has: ['On this Mac 1 file', 'Online-only 0'], hasNot: ['Kept on this Mac'] },
  { name: 'home-kept', fx: { keptUnsent: KEPT_OURS, kept: KEPT_MACOS }, has: ['Needs your attention', 'Board notes, 29 Sept.md', KEPT_OURS, KEPT_MACOS, 'Show in Finder', 'Dismiss'] },
  { name: 'home-locked', fx: { snapshot: snapshot({ phase: 'locked', account: { vault_unlocked: false }, storage: null }) }, has: ['Your vault is locked', 'Unlock vault', 'Vault locked'], hasNot: ['Beebeeb in Finder'], hifi: 'hifi-s3-fig3' },
  { name: 'home-long-email', fx: { snapshot: snapshot({ account: { email: LONG_EMAIL } }) }, has: [LONG_EMAIL] },
  { name: 'settings-general', act: gear, has: ['Settings', 'Home', 'General', 'Open Beebeeb at login', 'Tell me about'], sidebar: ['Back to Home', 'Account, sam@example.eu, Basic plan', 'General', 'Sync', 'About'], min: true, hifi: 'hifi-s4-fig1' },
  { name: 'settings-account', act: section('Account, sam@example.eu, Basic plan'), has: ['Account', 'sam@example.eu', 'Basic plan · renews 14 Nov 2026', 'Vault is unlocked', 'Where your files are stored', 'Europe', 'Germany', 'Sign out of this Mac', 'Sign out…'], hasNot: ['Couldn’t load storage locations'], min: true, hifi: 'hifi-s4-fig2' },
  { name: 'settings-sync', act: section('Sync'), has: ['Beebeeb in Finder', 'Added', 'Repair…', 'Keep on this Mac', 'Speed'], min: true, hifi: 'hifi-s4-fig3' },
  { name: 'settings-about', act: section('About'), has: ['Beebeeb for Mac', 'Version 0.8.12', 'Check for updates', 'Get help', 'Export a support bundle'], min: true, hifi: 'hifi-s4-fig5' },
  // ── Slice 4: Review ──  (the kind tag is drawn in capitals by CSS, as in the hi-fi, so `innerText` reads it in capitals)
  { name: 'review', act: review, has: ['Review', 'Files that need a decision from you, and changes Beebeeb couldn’t send yet.', 'CONFLICT', 'Board notes, 29 Sept.md', 'Team shared · today 09:12 · Changed on two devices.', 'Decide…', 'UPLOAD', 'Site visit, north wing.mov', 'Photos · today 08:47 · Couldn’t upload, will retry', 'Version history…'], hasNot: ['Local and remote both changed', 'STALE BASE', 'durable queue', 'Nothing needs your attention.'], sidebar: ['Home', 'Review, 2 need attention', 'Trash', 'Settings'], min: true },
  { name: 'review-all-kinds', fx: { review: reviewAllKinds }, act: review, has: ['CHANGED ELSEWHERE', 'STORAGE FULL', 'SIGNED OUT', 'Add storage', 'Sign in again', LONG_NAME, 'Decide…'], hasNot: ['STALE BASE', 'QUOTA', 'METADATA'], min: true },
  { name: 'review-conflict-sheet', act: decide, has: ['Conflict: Board notes, 29 Sept.md', 'This file was modified on two devices. Choose which version to keep.', 'This Mac', 'Other device', 'Keep this Mac’s', 'Keep the other', 'Keep both', '- 4. AOB (added on this Mac)', '+ 2. Roadmap (moved up)'], hasNot: ['Keep Mine', 'This device', '✓'], dialogs: 1, min: true, hifi: 'hifi-s3-fig1' },
  { name: 'review-conflict-binary', fx: { conflictPreview: { is_text: false, local: { size_bytes: 4096, text: null }, remote: { size_bytes: 8192, text: null } } }, act: decide, has: ['Binary file · 4,1 KB', 'Binary file · 8,2 KB', 'Keep both'], dialogs: 1 },
  { name: 'review-history-search', act: async (p) => { await history(p); await p.getByRole('textbox', { name: 'Search file names' }).fill('q3'); await p.getByRole('button', { name: /Q3 notes\.md/ }).waitFor() }, has: ['Version history', 'Q3 ledger reconciliation.xlsx', 'Investigations/Q3 ledger reconciliation.xlsx', 'Q3 notes.md'], dialogs: 1, min: true },
  { name: 'review-history-versions', act: async (p) => { await history(p); await p.getByRole('textbox', { name: 'Search file names' }).fill('q3'); await p.getByRole('button', { name: /Q3 ledger reconciliation/ }).click(); await p.getByRole('button', { name: 'Restore' }).first().waitFor() }, has: ['Version history', 'Q3 ledger reconciliation.xlsx', 'Version 5 is current · 2 earlier versions', 'Version 4', 'Version 3', 'Restore', 'Back'], hasNot: ['Version id', 'Queue restore'], dialogs: 1, min: true },
  { name: 'review-empty', fx: { review: [] }, act: review, has: ['Files that need a decision from you', 'Nothing needs your attention.', 'Version history…'], hasNot: ['Decide…'], sidebar: ['Home', 'Review', 'Trash', 'Settings'] },
  { name: 'review-locked', fx: { snapshot: snapshot({ phase: 'locked', account: { vault_unlocked: false }, storage: null }) }, act: review, has: ['Review', 'Your vault is locked', 'Unlock vault'], hasNot: ['Version history…', 'Nothing needs your attention.', 'Decide…'], sidebar: ['Home', 'Review', 'Trash', 'Settings'], hifi: 'hifi-s3-fig3' },
  // ── Slice 4: Trash ──
  { name: 'trash', act: trash, has: ['Trash', 'Deleted files in your vault. Restore puts them back through sync; permanent delete removes them from Trash.', 'Draft 3, interview questions.docx', 'File · 48 KB · deleted yesterday', 'Old scans', 'Folder · 1,4 GB · deleted ', LONG_NAME, 'Restore', 'Delete…', 'Refresh'], hasNot: ['Loading…'], min: true, hifi: 'hifi-s3-fig2' },
  { name: 'trash-delete-sheet', act: async (p) => { await trash(p); await p.getByRole('button', { name: 'Delete…' }).first().click(); await p.getByRole('dialog').waitFor() }, has: ['Delete permanently', 'Draft 3, interview questions.docx will be removed from Trash. This cannot be undone.', 'Cancel'], dialogs: 1, min: true },
  { name: 'trash-empty', fx: { trash: [] }, act: trash, has: ['Trash is empty', 'Deleted files will appear here.', 'Refresh'] },
  { name: 'trash-locked', fx: { snapshot: snapshot({ phase: 'locked', account: { vault_unlocked: false }, storage: null }) }, act: trash, has: ['Trash', 'Your vault is locked', 'Unlock vault'], hasNot: ['Refresh', 'Trash is empty'] },
  { name: 'signout-confirm', act: async (p) => { await section('Account, sam@example.eu, Basic plan')(p); await p.getByRole('button', { name: 'Sign out…' }).click(); await p.getByRole('dialog').waitFor() }, has: ['Sign out of this Mac?', 'Sync stops until you sign in again.', 'Cancel', 'Sign out'], dialogs: 1, hifi: 'hifi-s5-fig0' },
  { name: 'signed-out', fx: { snapshot: snapshot({ phase: 'signed_out', account: { email: null, logged_in: false, vault_unlocked: false }, storage: null }) }, has: ['Sign in to Beebeeb', 'Sign in'], hasNot: ['Home', 'Settings'], noSidebar: true },
  { name: 'signed-out-settings', fx: { snapshot: snapshot({ phase: 'signed_out', account: { email: null, logged_in: false, vault_unlocked: false }, storage: null }) }, open: '&tab=general', has: ['Sign in', 'Settings', 'General', 'About', 'Open Beebeeb at login'], sidebar: ['Back to Sign in', 'General', 'About'], hifi: 'hifi-s4-fig6' },
  { name: 'session-ended', fx: { snapshot: snapshot({ phase: 'session_ended', account: { auth_expired: true } }) }, has: ['Sign in again', 'Sync is paused until you sign in again. Your files stay in Finder.', 'Signed in before as sam@example.eu'], noSidebar: true },
]

const browser = await chromium.launch({ headless: true })
let passed = 0
const failures = []
const check = (ok, label, detail = '') => { if (ok) passed += 1; else failures.push(`${label}${detail ? ` (${detail})` : ''}`) }
const shots = []

try {
  for (const scheme of ['light', 'dark']) {
    for (const sc of SCENARIOS) {
      const sizes = [{ width: 920, height: 620, tag: '' }, ...(sc.min ? [{ width: 760, height: 520, tag: '-min' }] : [])]
      for (const size of sizes) {
        const label = `${sc.name}${size.tag} ${scheme}`
        const context = await browser.newContext({ viewport: { width: size.width, height: size.height }, colorScheme: scheme, deviceScaleFactor: 1, locale: 'nl-NL' })
        const page = await context.newPage()
        const errors = []
        page.on('pageerror', (e) => errors.push(String(e)))
        page.on('console', (m) => { if (m.type() === 'error') errors.push(`console: ${m.text()}`) })
        await page.addInitScript(installBackend, { caps, fx: structuredClone({ ...base, ...(sc.fx ?? {}) }) })
        try {
          await page.goto(`${pathToFileURL(pagePath).href}?window=main&platform=macos${sc.open ?? ''}`)
          await page.waitForSelector('.mw-window', { timeout: 8000 })
          await page.waitForFunction(() => !document.body.innerText.includes('Loading…'), null, { timeout: 8000 })
          await page.waitForTimeout(400)
          if (sc.act) await sc.act(page)
          await page.waitForTimeout(500)
          const r = await page.evaluate(report)
          check(r.wide.length === 0, `${label}: nothing wider than its box (${r.rows} containers)`, r.wide.map((w) => `${w.el} ${w.scrollWidth} vs ${w.clientWidth}`).join('; '))
          check(r.rows >= 3, `${label}: the predicate found its containers`, `${r.rows}`)
          check(r.docScrollHeight <= r.innerHeight, `${label}: the document does not scroll`, `${r.docScrollHeight} vs ${r.innerHeight}`)
          for (const text of sc.has ?? []) check(r.text.includes(text), `${label}: shows "${text}"`)
          for (const text of sc.hasNot ?? []) check(!r.text.includes(text), `${label}: does not show "${text}"`)
          if (sc.noSidebar) check(r.sidebarWidth === null, `${label}: no sidebar while signed out`)
          else check(r.sidebarWidth === 214, `${label}: the sidebar is 214 pt`, `${r.sidebarWidth}`)
          if (!sc.noSidebar) check(r.toolbarHeight === 52, `${label}: the toolbar is 52 pt`, `${r.toolbarHeight}`)
          if (sc.bar) check(r.barHeight === 4, `${label}: the storage bar is drawn, 4 pt tall`, `${r.barHeight}`)
          if (sc.sidebar) check(JSON.stringify(r.sidebarButtons) === JSON.stringify(sc.sidebar), `${label}: the sidebar lists ${sc.sidebar.join(', ')}`, r.sidebarButtons.join(', '))
          if (sc.dialogs !== undefined) check(r.dialogs === sc.dialogs, `${label}: ${sc.dialogs} sheet`, `${r.dialogs}`)
          check(errors.length === 0, `${label}: no page errors`, errors.join(' | '))
          // The native traffic lights, drawn for the PNG only (the overlay title bar puts them over the sidebar).
          await page.evaluate(() => {
            const tl = document.createElement('div')
            tl.setAttribute('style', 'position:fixed;left:18px;top:18px;display:flex;gap:8px;z-index:2000')
            tl.innerHTML = ['#ff5f57', '#febc2e', '#28c840'].map((c) => `<div style="width:12px;height:12px;border-radius:50%;background:${c}"></div>`).join('')
            document.body.appendChild(tl)
          })
          const file = path.join(output, 'png', `${sc.name}${size.tag}-${scheme}.png`)
          await page.screenshot({ path: file })
          shots.push({ sc, scheme, size, file })
        } catch (error) {
          check(false, `${label}: drove the window to its state`, String(error).slice(0, 300))
        } finally {
          await context.close()
        }
      }
    }
  }
  // Side by side with the hi-fi's figures (each figure holds the light and dark copies of one screen).
  if (process.env.HIFI_PNG_DIR) {
    for (const sc of SCENARIOS.filter((s) => s.hifi)) {
      const theirs = path.join(process.env.HIFI_PNG_DIR, `${sc.hifi}.png`)
      if (!existsSync(theirs)) { check(false, `${sc.name}: hi-fi figure ${sc.hifi}.png exists`); continue }
      const light = path.join(output, 'png', `${sc.name}-light.png`)
      const dark = path.join(output, 'png', `${sc.name}-dark.png`)
      const html = path.join(output, 'png', `compare-${sc.name}.html`)
      await writeFile(html, `<!doctype html><meta charset="utf-8"><body style="margin:0;background:#cfccc4;font:12px monospace;color:#222;padding:16px">
        <div style="margin-bottom:6px">hi-fi: design/hifi/macos-two-surface.html, ${sc.hifi}</div><img src="${pathToFileURL(theirs).href}" style="max-width:1180px">
        <div style="margin:16px 0 6px">this build, Chromium 1x, 920 x 620: light, then dark</div>
        <div style="display:flex;gap:16px"><img src="${pathToFileURL(light).href}"><img src="${pathToFileURL(dark).href}"></div></body>`)
      const page = await browser.newPage({ viewport: { width: 1900, height: 900 } })
      await page.goto(pathToFileURL(html).href)
      await page.waitForFunction(() => [...document.images].every((i) => i.complete && i.naturalWidth > 0))
      await page.screenshot({ path: path.join(output, 'png', `compare-${sc.name}.png`), fullPage: true })
      await page.close()
    }
  }
} finally {
  await browser.close()
}

console.log(`screenshots: ${shots.length}`)
if (failures.length > 0) {
  console.log(`RED: ${failures.length} failed, ${passed} passed`)
  for (const f of failures) console.log(`  FAIL ${f}`)
  process.exit(1)
}
console.log(`${passed} passed, 0 failed (the Beebeeb window in Chromium)`)
