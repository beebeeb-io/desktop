// M3e: the wiring of B09 (the flyout), B12 (the settings shell, Drive, Backups) and B13 (settings today). Settings pages
// are addressable and every opener names its page (lead ruling); the flyout's actions use commands that answer the
// Windows "tray" window and say a refusal; the update button follows the platform (ruling M3e item 1).
import { afterEach, beforeEach, describe, expect, test } from 'bun:test'
import { routeFromNav, routeFromSearch, searchFromRoute, settingsRouteFor } from '../src/v2/routes'
import { followHref, settingsPage } from '../src/v2/sections/settings/shell/links'
import { flyoutFailure, revealRow, runFlyoutCommand } from '../src/v2/tray'
import { flyoutView } from '../src/v2/tray/model'
import { SCENARIOS, NOW } from '../src/v2/tray/fixtures'
import { kindOf } from '../src/v2/sections/settings/today/insights'
import { updateCopyFor } from '../src/v2/update'
import { MAC_DOWNLOAD_COPY, RESTART_COPY } from '../src/v2/updateCopy'
import type { DesktopCapabilities } from '../src/capabilities'

/** A stand-in for Tauri's IPC and the window's history: each command's answer, and what was called. */
let calls: { cmd: string; args?: Record<string, unknown> }[] = []
let answers: Record<string, unknown> = {}
let pushed: string[] = []
const HIDE = 'plugin:window|hide'
/** The commands an action ran, without the flyout hiding itself (asserted on its own). */
const ran = () => calls.filter((c) => c.cmd !== HIDE)
const hid = () => calls.filter((c) => c.cmd === HIDE).length
const g = globalThis as Record<string, unknown>
const saved = { window: g.window, PopStateEvent: g.PopStateEvent }
beforeEach(() => {
  g.PopStateEvent ??= class extends Event {}
  calls = []
  answers = {}
  pushed = []
  ;(globalThis as Record<string, unknown>).window = {
    location: { search: '?window=main-app&platform=windows&section=files', pathname: '/' },
    history: { pushState: (_s: unknown, _t: string, url: string) => { pushed.push(url) } },
    dispatchEvent: () => true,
    __TAURI_INTERNALS__: {
      metadata: { currentWindow: { label: 'tray' }, currentWebview: { windowLabel: 'tray', label: 'tray' } },
      invoke: async (cmd: string, args?: Record<string, unknown>) => {
        calls.push({ cmd, args })
        const a = answers[cmd]
        if (a instanceof Error) throw a
        if (cmd === HIDE) return null
        if (a === undefined) throw new Error(`${cmd}: not implemented in a browser`)
        return a
      },
    },
  }
})
afterEach(() => { g.window = saved.window; g.PopStateEvent = saved.PopStateEvent })

describe('settings pages are addressable as section=settings&page=<page> (lead ruling, M3e)', () => {
  test('the URL holds the page, and an older sub= link still opens it', () => {
    expect(routeFromSearch('?section=settings&page=backups').sub).toBe('backups')
    expect(routeFromSearch('?section=settings&sub=storage').sub).toBe('storage')
    expect(searchFromRoute({ section: 'settings', id: null, sub: 'about' }, '?window=main-app&platform=windows&sub=x')).toBe('?window=main-app&platform=windows&section=settings&page=about')
    expect(searchFromRoute({ section: 'shared', id: null, sub: 'requests' }, '?section=settings&page=general')).toBe('?section=shared&sub=requests')
  })
  test('a native opener’s nav word lands on its page, and the word is dropped once the window has a route', () => {
    expect(routeFromSearch('?window=main-app&platform=windows&nav=settings')).toEqual({ section: 'settings', id: null, sub: 'general' })
    expect(routeFromSearch('?window=main-app&platform=windows&nav=settings/backups')).toEqual({ section: 'settings', id: null, sub: 'backups' })
    expect(routeFromNav('problems')).toEqual({ section: 'sync-history', id: null, sub: 'problems' })
    expect(routeFromNav('activity')).toEqual({ section: 'sync-history', id: null, sub: null })
    expect(routeFromNav('review')).toEqual({ section: 'files', id: null, sub: null })
    expect(routeFromNav('trash')).toEqual({ section: 'trash', id: null, sub: null })
    for (const bad of ['', 'finder', 'settings/Backups', 'settings/a/b', 'settings/x&y=1']) expect(routeFromNav(bad)).toBeNull()
    // A route of the window's own wins over the opener's word.
    expect(routeFromSearch('?section=trash&nav=settings').section).toBe('trash')
    expect(searchFromRoute({ section: 'files', id: null, sub: null }, '?nav=settings')).toBe('?section=files')
  })
  test('the gear and Ctrl+, open the page of the section on screen: Backups opens Settings › Backups, others General (ruling M3a item 2)', () => {
    expect(settingsRouteFor('backups')).toEqual({ section: 'settings', sub: 'backups' })
    for (const s of ['files', 'shared', 'photos', 'trash', 'sync-history', 'available-offline', 'search', 'settings'] as const) {
      expect(settingsRouteFor(s)).toEqual({ section: 'settings', sub: 'general' })
    }
  })
  test('page words that are a dialog or a sub-page land there; an unknown word opens General', () => {
    expect(settingsPage('report')).toEqual({ pane: 'general', dialog: 'report' })
    expect(settingsPage('mail-backup')).toEqual({ pane: 'backups' })
    expect(settingsPage('signin-activity')).toEqual({ pane: 'security', props: { init: { page: 'activity' } } })
    expect(settingsPage('devices')).toEqual({ pane: 'devices' })
    expect(settingsPage('nonsense')).toEqual({ pane: 'general' })
  })
})

describe('a settings link never navigates the web view (DW-0712#0, DW-0739, DW-0751#4, DW-0791)', () => {
  test('a settings page moves inside Settings; another section moves the window; the web opens in the OS', async () => {
    const went: unknown[] = []
    expect(followHref('?section=settings&page=signin-activity', (p, o) => went.push([p, o]))).toBeNull()
    expect(went).toEqual([['security', { dialog: undefined, props: { init: { page: 'activity' } } }]])
    expect(followHref('?section=shared&sub=with-me', () => undefined)).toBeNull()
    expect(pushed).toEqual(['?window=main-app&platform=windows&section=shared&sub=with-me'])
    answers['plugin:opener|open_url'] = null
    const opened = followHref('https://beebeeb.io', () => undefined)
    expect(opened).not.toBe(false)
    expect((await opened)?.ok).toBe(true)
    expect(calls).toEqual([{ cmd: 'plugin:opener|open_url', args: { url: 'https://beebeeb.io' } }])
    expect(followHref('#top', () => undefined)).toBe(false)
  })
})

describe('the flyout’s actions (B09)', () => {
  test('a refusal reads in the app’s one wording, never the host’s text', () => {
    expect(flyoutFailure('pause', { ok: false, reason: 'This action belongs to the menu-bar popover.', unsupported: false })).toBe('Couldn’t pause syncing.')
    expect(flyoutFailure('settings', { ok: false, reason: 'x', unsupported: true })).toBe('Couldn’t open Settings. This isn’t available in this build yet.')
    expect(flyoutFailure('open', { ok: true, value: undefined })).toBeNull()
  })
  test('Sync anyway is refused, never a retry dressed up as it (DW-1222, blocked on metered detection)', async () => {
    const r = await runFlyoutCommand('sync-anyway')
    expect(r.ok).toBe(false)
    expect(calls).toEqual([])
  })
  test('a refused action leaves the flyout open so its line can be read; a done one hides it', async () => {
    const refused = await runFlyoutCommand('unlock')
    expect(flyoutFailure('unlock', refused)).toBe('Couldn’t open Beebeeb to unlock. This isn’t available in this build yet.')
    expect(hid()).toBe(0)
    answers.popover_unlock = null
    expect((await runFlyoutCommand('unlock')).ok).toBe(true)
    expect(ran().map((c) => c.cmd)).toEqual(['popover_unlock', 'popover_unlock'])
    expect(hid()).toBe(1)
    // Try again stays open either way: the flyout shows sync starting again.
    answers.popover_retry_sync = null
    expect((await runFlyoutCommand('try-again')).ok).toBe(true)
    expect(hid()).toBe(1)
  })
  test('each action calls its command: Show opens Sync history’s problems, Review the decision, Restart to update installs', async () => {
    answers.popover_open_window = null
    answers.install_update = null
    await runFlyoutCommand('show-failed')
    await runFlyoutCommand('review')
    await runFlyoutCommand('update')
    expect(ran()).toEqual([
      { cmd: 'popover_open_window', args: { to: 'problems' } },
      { cmd: 'popover_open_window', args: { to: 'review' } },
      { cmd: 'install_update', args: {} },
    ])
  })
  test('a Recent changes row shows its file in File Explorer; a file no longer in the folder opens Trash (DW-1202#0)', async () => {
    const row = flyoutView(SCENARIOS['up-to-date'].snapshot, {}, NOW).list
    if (row.kind !== 'rows') throw new Error('rows expected')
    answers.drive_reveal = null
    expect((await revealRow(row.rows[0])).ok).toBe(true)
    expect(ran()).toEqual([{ cmd: 'drive_reveal', args: { id: 'h0' } }])
    calls = []
    answers.drive_reveal = new Error('not-found: no such file')
    answers.popover_open_window = null
    await revealRow(row.rows[0])
    expect(ran()).toEqual([{ cmd: 'drive_reveal', args: { id: 'h0' } }, { cmd: 'popover_open_window', args: { to: 'trash' } }])
  })
})

describe('the update button follows the platform (lead ruling M3e item 1)', () => {
  const caps = (host_os: DesktopCapabilities['host_os'], update_format: DesktopCapabilities['update_format']) => ({ host_os, update_format }) as DesktopCapabilities
  test('Windows and Linux restart into it; a Mac downloads it; an unknown host is the Windows app', () => {
    expect(updateCopyFor(caps('windows', 'nsis'))).toEqual(RESTART_COPY)
    expect(updateCopyFor(caps('linux', 'appimage'))).toEqual(RESTART_COPY)
    expect(updateCopyFor(caps('macos', 'app_bundle'))).toEqual(MAC_DOWNLOAD_COPY)
    expect(updateCopyFor(caps('linux', 'package_manager')).mode).toBe('download')
    expect(updateCopyFor(null)).toEqual(RESTART_COPY)
  })
  test('the flyout says Restart to update on Windows (DW-1210, DW-1214) and Download update on a Mac', () => {
    const s = SCENARIOS.update
    expect(flyoutView(s.snapshot, s.extras, NOW).action?.label).toBe('Restart to update')
    expect(flyoutView(s.snapshot, s.extras, NOW).line).toBe(RESTART_COPY.line)
    expect(flyoutView(s.snapshot, { ...s.extras, update: MAC_DOWNLOAD_COPY }, NOW).action?.label).toBe('Download update')
    expect(flyoutView(SCENARIOS['up-to-date'].snapshot, { updateRequired: true }, NOW).action).toEqual({ label: 'Restart to update', command: 'update', primary: true })
  })
})

describe('Storage insights reads the account’s own largest files (B13)', () => {
  test('a file’s kind comes from its name, else from the category the breakdown gives it', () => {
    expect(kindOf('Projects/Render final.mov', 'media')).toBe('Video')
    expect(kindOf('Scans/Invoice scan.PDF', 'documents')).toBe('PDF')
    expect(kindOf('Pictures/IMG_0001.HEIC', 'media')).toBe('Photo')
    expect(kindOf('Backups/site.7z', 'other')).toBe('Archive')
    expect(kindOf('Notes/plan.txt', 'documents')).toBe('Documents')
    expect(kindOf('blob', 'other')).toBe('Other')
  })
})
