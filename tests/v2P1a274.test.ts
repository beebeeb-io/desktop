// P1a: desktop #274's review (M3e), the findings P1a takes (the lead's item list, 2026-11-09 05:48). Each test names
// its finding; each was seen red first (the PR body has both runs).
import { describe, expect, test } from 'bun:test'
import { existsSync, readFileSync } from 'node:fs'
import { cutPath, sectionOfPath } from '../src/v2/sections/settings/today/insights'
import { pickThenCheck } from '../src/v2/sections/settings/today/about'
import { settingsPage } from '../src/v2/sections/settings/shell/links'
import { lineOpens } from '../src/v2/chrome/StatusLine'
import { planReset } from '../src/v2/tray/model'
import { newestOnly } from '../src/v2/throttle'
import type { TransferRow } from '../src/v2/source/extras'

const src = (path: string) => readFileSync(new URL(`../src/${path}`, import.meta.url), 'utf8')

describe('#274 about.tsx:43 (P2): a new release channel is saved before the check reads it', () => {
  test('the check waits for the write; a write that failed checks nothing and says so', async () => {
    const order: string[] = []
    let land: (ok: boolean) => void = () => {}
    const save = () => new Promise<boolean>((r) => { land = (ok) => { order.push('saved'); r(ok) } })
    const done = pickThenCheck(save, async () => { order.push('checked') }, () => { order.push('refused') })
    await Promise.resolve()
    expect(order).toEqual([])
    land(true)
    await done
    expect(order).toEqual(['saved', 'checked'])
    order.length = 0
    const failed = pickThenCheck(save, async () => { order.push('checked') }, () => { order.push('refused') })
    land(false)
    await failed
    expect(order).toEqual(['saved', 'refused'])
  })
  test('the page picks through it, the write hands back its promise, and a later answer clears an earlier offer', () => {
    const about = src('v2/sections/settings/today/about.tsx')
    expect(about).toContain('pickThenCheck(() => update({ release_channel: c }), doCheck,')
    expect(about).not.toContain('update({ release_channel: c }); void doCheck()')
    expect(about).toMatch(/noteUpdate\(null\)\n\s+setDown\(/)
    expect(src('v2/sections/settings/shell/data.ts')).toMatch(/const update = useCallback\(\(patch: Partial<DesktopConfig>\): Promise<boolean> =>/)
  })
})

describe('#274 insights.tsx:35 (P2): a stored path with a leading slash is still in Backups', () => {
  test('"/Backups/…" and "\\\\Backups\\\\…" open Backups; the slash is not shown', () => {
    expect(sectionOfPath('/Backups/Anna-PC/Videos/Render final.mov')).toBe('backups')
    expect(sectionOfPath('\\Backups\\Anna-PC\\clip.mov')).toBe('backups')
    expect(sectionOfPath('Backups/Anna-PC/a.txt')).toBe('backups')
    expect(sectionOfPath('/docs/report.pdf')).toBe('files')
    expect(sectionOfPath('/BackupsOld/a.txt')).toBe('files')
    expect(cutPath('/docs/report.pdf')).toBe('docs/report.pdf')
  })
})

describe('#274 about.tsx:90: the release-notes link opens in the browser, never inside the window', () => {
  test('the banner link prevents the navigation and asks the OS', () => {
    const about = src('v2/sections/settings/today/about.tsx')
    expect(about).toMatch(/<a href=\{RELEASES_URL\} onClick=\{\(e\) => \{ e\.preventDefault\(\); void openExternal\(RELEASES_URL\)/)
    expect(about).not.toContain('<a href="https://beebeeb.io/releases" className="underline">')
  })
})

describe('#274 tray/index.tsx:113: the flyout’s refusal line goes when the flyout hides', () => {
  test('losing focus clears the line before it hides; so does Esc', () => {
    const tray = src('v2/tray/index.tsx')
    expect(tray).toContain('if (!focused) { setNotice(null); void getCurrentWindow().hide() }')
    expect(tray).toContain("if (e.key === 'Escape') { setNotice(null); void hideFlyout() }")
  })
})

describe('#274 tray/index.tsx:126: no plan is the signed-in account’s, never the last one’s', () => {
  test('signing out or another account starts the plan read again, with no plan cleared', () => {
    expect(planReset(null, { logged_in: true, email: 'a@example.com' })).toEqual({ reset: true, for: 'a@example.com' })
    expect(planReset('a@example.com', { logged_in: true, email: 'a@example.com' })).toEqual({ reset: false, for: 'a@example.com' })
    expect(planReset('a@example.com', { logged_in: false, email: null })).toEqual({ reset: true, for: null })
    expect(planReset('a@example.com', { logged_in: true, email: 'b@example.com' })).toEqual({ reset: true, for: 'b@example.com' })
    const tray = src('v2/tray/index.tsx')
    expect(tray).toMatch(/const next = planReset\(planFor, result\.value\.account\)\n\s+if \(next\.reset\) \{ planAt = 0; if \(!cancelled\) setNoPlan\(false\) \}/)
  })
})

describe('#274 tray/index.tsx:14: the flyout does not load the main window’s chrome for one predicate', () => {
  test('hasNoPlan lives in a module of its own; the tray and the gate import it from there', () => {
    const tray = src('v2/tray/index.tsx')
    expect(tray).not.toContain("from '../auth/Gate'")
    expect(tray).toContain("import { hasNoPlan } from '../auth/plan'")
    expect(src('v2/auth/Gate.tsx')).toContain("import { hasNoPlan } from './plan'")
    expect(src('v2/auth/plan.ts')).not.toMatch(/^import /m)
  })
})

describe('#274 StatusLine.tsx:95: the line reaches the Uploads panel whenever it has something to show', () => {
  const row = (state: TransferRow['state'], direction: TransferRow['direction'] = 'up'): TransferRow => ({ id: state, name: 'a', folder: '', direction, state, bytes: 0, retryable: true })
  test('a moving or stopped upload opens the panel; nothing there opens the flyout', () => {
    expect(lineOpens([row('moving')])).toBe('uploads')
    expect(lineOpens([row('stopped')])).toBe('uploads')
    expect(lineOpens([row('waiting')])).toBe('flyout')
    expect(lineOpens([row('stopped', 'down')])).toBe('flyout')
    expect(lineOpens([])).toBe('flyout')
  })
  test('a host without the flyout opens the Uploads panel, not a refusal', () => {
    const line = src('v2/chrome/StatusLine.tsx')
    expect(line).toContain("if (lineOpens(rows) === 'uploads') { openUploads(); return }")
    expect(line).toContain('if (r.unsupported) { setRefused(null); openUploads(); return }')
  })
})

describe('#274 backups/index.tsx:94: Couldn’t back up reads the list at most once a second, and an older answer never wins', () => {
  test('newestOnly: a read that started before another is not the newest', () => {
    const start = newestOnly()
    const first = start()
    const second = start()
    expect(first()).toBe(false)
    expect(second()).toBe(true)
  })
  test('the page throttles its re-reads and drops a stale answer', () => {
    const page = src('v2/sections/settings/backups/index.tsx')
    expect(page).toContain('const newest = newestRead.current()')
    expect(page).toContain('if (newest()) setList(all.filter((p) => p.backup))')
    expect(page).toMatch(/const soon = throttled\(\(\) => void load\(\)\)\n\s+const stop = source\.subscribe\(\(e\) => \{ if \(e\.kind === 'sync'\) soon\(\) \}\)/)
  })
})

describe('#274 intercept.tsx:22: one way to follow a Settings link', () => {
  test('General and Notifications are the base panes; their rows follow through the kit Row', () => {
    expect(existsSync(new URL('../src/v2/sections/settings/today/intercept.tsx', import.meta.url))).toBe(false)
    const today = src('v2/sections/settings/panes-today.ts')
    expect(today).not.toContain('InterceptLinks')
    expect(settingsPage('diagnostics')).toEqual({ pane: 'diagnostics' })
    expect(settingsPage('recent')).toEqual({ pane: 'recent' })
  })
})

describe('#274 links.ts:32: a page word is an own key, never an inherited one', () => {
  test('"constructor" and "toString" land on General', () => {
    expect(settingsPage('constructor')).toEqual({ pane: 'general' })
    expect(settingsPage('toString')).toEqual({ pane: 'general' })
    expect(settingsPage('report')).toEqual({ pane: 'general', dialog: 'report' })
  })
})
