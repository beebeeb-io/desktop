/**
 * Task 1683 slice 5, the copy sweep: every row of the two-surface spec's §10.2 ("Every copy change, old → new") that
 * lands in this slice, pinned by the words that must be gone from the page and the words that replace them. The guard
 * in tests/macCopyGuard.test.ts answers "does any developer word survive on a Mac"; this answers "is each row done as the
 * table says", including rows whose old words were not on the guard's list ("Visible without local bytes").
 *
 * The pages are large hook components, so these pins read their source: the same technique as
 * tests/finderSetupSourceContract.test.ts. A row is pinned in the file the table names; `gone` is the old text,
 * `present` the new. Rows that belong to another slice of task 1683 are listed at the end, with their owner, so the
 * absence of a pin is a decision on the page and not an omission.
 */
import { afterEach, describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { command, commandUnavailableLabel } from '../src/desktopApi'

const ROOT = process.env.GUARD_ROOT ?? fileURLToPath(new URL('..', import.meta.url))
const source = (file: string) => readFileSync(join(ROOT, 'src', file), 'utf8')
/** The same text with every run of whitespace as one space, so a sentence wrapped across JSX lines still matches. */
const flat = (file: string) => source(file).replace(/\s+/g, ' ')

interface Row {
  row: number
  what: string
  file: string
  gone: string[]
  present?: string[]
}

const ROWS: Row[] = [
  { row: 1, what: 'Drive status intro', file: 'pages/Status.tsx', gone: ['is the control center', 'the file surface', 'can hydrate, upload, and keep versions safe'] },
  { row: 2, what: 'Drive status loading', file: 'pages/Status.tsx', gone: ['Loading daemon status…', 'Waiting for daemon status.'], present: ['Loading…'] },
  { row: 3, what: 'Drive status metrics and rows', file: 'pages/Status.tsx', gone: ['Hydrates and uploads waiting now', 'Visible without local bytes', 'Version reviews needing action', 'Session present', 'No session token', 'engine {'], present: ['Transfer queue', 'Downloads and uploads waiting now', 'Download when you open them', 'Files that need a decision from you', 'Vault access', 'Signed in'] },
  { row: 4, what: 'Account page sign-out paragraph', file: 'pages/Account.tsx', gone: ['Future native wiring', 'Signing out stops sync and removes the session'] },
  { row: 5, what: 'Account page intro', file: 'pages/Account.tsx', gone: ['Session token, unlock state, diagnostics', 'leaving the Finder location visible'] },
  { row: 6, what: 'Account page rows', file: 'pages/Account.tsx', gone: ['Session token', 'Stored by native session layer', 'Master key lifetime', 'Unwrapped in memory', 'Unavailable to file operations'], present: ['Vault is unlocked', 'Vault is locked'] },
  { row: 7, what: 'Selective sync intro', file: 'pages/SelectiveSync.tsx', gone: ['The drive is online-only by default', 'remote-first behavior', 'hydrated'], present: ['Folders you choose stay on this Mac. Everything else downloads when you open it.'] },
  { row: 8, what: 'Selective sync panels and empty state', file: 'pages/SelectiveSync.tsx', gone: ['Persisted by', 'Local daemon state', 'Recursive folder toggles', 'The UI is ready for', 'this build has no folder data'], present: ['Couldn’t load your folders.', 'No folders yet.', 'No folders kept on this computer', 'A folder and everything inside it'] },
  { row: 9, what: 'Finder location "Drive model"', file: 'pages/SyncFolder.tsx', gone: ['Remote-first vault namespace.', 'Shared roots appear next to owned files.', 'Virtual view of pinned content.', 'Items needing version review.'], present: ['Drive model', 'Your vault, stored in the cloud.', 'Shared folders appear next to your own files.', 'Folders you choose to keep on this computer.'] },
  { row: 10, what: 'Finder repair', file: 'pages/SyncFolder.tsx', gone: ['clears the local socket', 'remove the stale local socket', 'Reset Finder integration?', 'REPAIR_TITLE', 'REPAIR_BODY'], present: ['RESET_FINDER_TITLE', 'RESET_FINDER_BODY'] },
  { row: 11, what: 'Versions intro', file: 'pages/VersionCenter.tsx', gone: ['Desktop writes create server-side versions', 'until the daemon can finish them'], present: ['Files that need a decision from you, and changes Beebeeb couldn’t send yet.'] },
  { row: 12, what: 'Versions metrics', file: 'pages/VersionCenter.tsx', gone: ['Rows marked conflict by the daemon', 'Failed or queued Finder writes', 'Local bytes preserved for review', 'Stale-base edits'], present: ['Active conflicts', 'Files changed in two places', 'Changes Beebeeb couldn’t send yet', 'Changed elsewhere'] },
  { row: 13, what: 'Versions empty', file: 'pages/VersionCenter.tsx', gone: ['recorded in the local state DB'], present: ['Nothing needs your attention.'] },
  { row: 14, what: 'Versions history unavailable', file: 'pages/VersionCenter.tsx', gone: ['until the server list-versions route responds'], present: ['Couldn’t load this file’s versions. Try again.'] },
  { row: 15, what: 'Restore toast (Versions)', file: 'pages/VersionCenter.tsx', gone: ['will be restored by the sync engine'], present: ['goes back to version', 'on the next sync.'] },
  { row: 15, what: 'Restore toast (version history)', file: 'DesktopVersionHistory.tsx', gone: ['will be restored by the sync engine'], present: ['goes back to version', 'on the next sync.'] },
  { row: 18, what: 'Notifications page toast', file: 'pages/Notifications.tsx', gone: ['are not wired up yet', 'will not persist'], present: ['Couldn’t load these settings'] },
  { row: 19, what: 'Bandwidth page toast', file: 'pages/Bandwidth.tsx', gone: ['are not wired up yet', 'will not persist'], present: ['Couldn’t load these settings'] },
  { row: 20, what: 'Shared roots intro', file: 'pages/Shared.tsx', gone: ['Shared content is part of the Finder namespace', 'enforced by the daemon'] },
]

describe('§10.2: each row of this slice is done as the table says', () => {
  for (const { row, what, file, gone, present } of ROWS) {
    test(`#${row} ${what} (${file})`, () => {
      const text = flat(file)
      for (const old of gone) expect(text, `old text still in ${file}: "${old}"`).not.toContain(old)
      for (const fresh of present ?? []) expect(text, `new text missing from ${file}: "${fresh}"`).toContain(fresh)
    })
  }

  test('the table names real rows: each file exists and was read, so a rename cannot make a pin pass by reading nothing', () => {
    expect(new Set(ROWS.map((r) => r.file)).size).toBeGreaterThanOrEqual(8)
    for (const { file } of ROWS) expect(source(file).length, file).toBeGreaterThan(500)
    // Counts, not "no failures": 19 pins this slice owns.
    expect(ROWS).toHaveLength(19)
  })
})

describe('§10.2 #24: a command that is missing is said plainly, on every platform', () => {
  const warnings: unknown[][] = []
  const original = console.warn
  afterEach(() => {
    console.warn = original
    warnings.length = 0
  })

  test('the sentence is "This isn’t available in this version of Beebeeb." and carries no command name', () => {
    console.warn = (...args: unknown[]) => { warnings.push(args) }
    const label = commandUnavailableLabel('set_recursive_pin')
    expect(label).toBe('This isn’t available in this version of Beebeeb.')
    expect(label).not.toContain('set_recursive_pin')
    expect(label).not.toMatch(/wired|build/i)
  })

  // PR #140 review, P3: the label is read while a component renders, so it must not log: every render of a notice would
  // write the same line again. The line is written once, where the host's answer is read and the command is found missing.
  test('reading the sentence logs nothing, however often a component renders it', () => {
    console.warn = (...args: unknown[]) => { warnings.push(args) }
    for (let render = 0; render < 3; render++) commandUnavailableLabel('set_recursive_pin')
    expect(warnings).toEqual([])
  })

  test('the command name goes to the log once, where the missing command is found, so it can still be found', async () => {
    console.warn = (...args: unknown[]) => { warnings.push(args) }
    const previous = (globalThis as any).window
    ;(globalThis as any).window = { __TAURI_INTERNALS__: { invoke: async () => { throw new Error('unknown command set_recursive_pin') } } }
    try {
      const result = await command('set_recursive_pin')
      expect(result).toMatchObject({ ok: false, unsupported: true })
    } finally {
      ;(globalThis as any).window = previous
    }
    expect(warnings).toEqual([['set_recursive_pin', 'unavailable']])
  })

  test('an ordinary failure is not a missing command and logs nothing here', async () => {
    console.warn = (...args: unknown[]) => { warnings.push(args) }
    const previous = (globalThis as any).window
    ;(globalThis as any).window = { __TAURI_INTERNALS__: { invoke: async () => { throw new Error('vault is locked') } } }
    try {
      expect(await command('set_recursive_pin')).toMatchObject({ ok: false, unsupported: false })
    } finally {
      ;(globalThis as any).window = previous
    }
    expect(warnings).toEqual([])
  })
})

describe('§10.2 rows that belong to another slice of task 1683 (not pinned here, on purpose)', () => {
  // The owners are the spec's own §12 rows. Each row is listed once, so a reader can see every number of the table accounted for.
  const OTHERS: Array<[number[], string]> = [
    [[16, 17, 35], 'slice 4, Review and Trash (kind labels, the review actions, the conflict sheet\'s words)'],
    [[21, 22, 36, 38, 44], 'slice 2, signed-out window (the ready and pinning steps go with spec C §11; window titles; the sign-in screens)'],
    [[23, 39, 40, 41, 42, 43, 47], 'slice 3, the popover (the compact window and its sidebar are not built on a Mac)'],
    [[37, 48], 'slice 1, done: the macOS menu and the Settings sidebar words'],
    [[45], 'Home\'s new copy: no slice names it; reported to the lead in the PR'],
  ]

  test('every row number 1 to 48 is accounted for exactly once: pinned here, built new in the Mac components, or owned by another slice', () => {
    const mine = new Set([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 18, 19, 20, 24])
    // 25 to 34: the old text lives only in Windows-shell views a Mac never mounts, and the Mac's components are new, so
    // they never had it. 46 is the new Settings copy, pinned by the component tests (tests/macSecurity*, macSettings*).
    const byConstruction = new Set([25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 46])
    const others = OTHERS.flatMap(([rows]) => rows)
    const all = [...mine, ...byConstruction, ...others].sort((a, b) => a - b)
    expect(all).toEqual(Array.from({ length: 48 }, (_, i) => i + 1))
  })
})
