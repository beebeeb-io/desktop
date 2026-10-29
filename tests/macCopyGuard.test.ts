import { describe, expect, setDefaultTimeout, test } from 'bun:test'
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import {
  DEVELOPER_WORDS,
  WINDOWS_ONLY,
  commandNames,
  describeViolation,
  mainImports,
  macRoots,
  pardon,
  type Pardon,
  personFacingStrings,
  reachableModules,
  rustSentencesIn,
  scan,
  scanRust,
  violationsInSource,
} from './fixtures/copyGuard'

/**
 * Task 1683 slice 5 (two-surface spec `docs/specs/2026-10-24-macos-two-surface-app.md` §10.1, §12 row 5): no developer
 * word in any string a person can read on a Mac. See tests/fixtures/copyGuard.ts for what is scanned and how a
 * person-facing string is told from a machine value. `GUARD_ROOT` points the scan at another checkout, which is how it
 * was proven red on the tree before this slice.
 */
// Every test here reads, scans or copies the real source tree, which takes seconds when the machine is loaded (a gate at
// load 60 timed three of them out at bun's default 5 s; the same tests take under 4 s together at load 14). Their results do
// not depend on the time, so a slow run must not be a red one (PR #147, lead's gate).
setDefaultTimeout(30_000)

const ROOT = process.env.GUARD_ROOT ?? fileURLToPath(new URL('..', import.meta.url))

/**
 * Strings that are still on a Mac because ANOTHER slice of task 1683 removes the surface they live on. Each entry names
 * its owner and the exact number of occurrences it pardons. A ratchet, not a pardon: when the owner removes a string its
 * entry must go in the same change (a stale entry is red, below), a second occurrence of a pardoned string is a new
 * violation (the count is exact), and the number of entries is pinned in the test below, so pardoning one more string means
 * changing that number in the diff where a reviewer reads it. Nothing of this slice's own may be listed.
 */
const PENDING: readonly Pardon[] = [
  // Spec C §11 removes the ready step and the pinning step of the onboarding window; slice 2 (signed-out window) deletes them.
  { file: 'Onboarding.tsx', text: 'Open the control center.', owner: 'slice 2, signed-out window (spec C §11, rows #21 and #22)', count: 1 },
  { file: 'Onboarding.tsx', text: 'No folders are pinned by default. You can make any folder recursively available offline now or later from the control center.', owner: 'slice 2, signed-out window (spec C §11, row #22)', count: 1 },
  { file: 'Onboarding.tsx', text: 'Control center is ready', owner: 'slice 2, signed-out window (spec C §11, row #21)', count: 1 },
  { file: 'Onboarding.tsx', text: 'Use the app for sync health, lock state, offline folders, shared roots, versions, conflicts, and diagnostics. Finder remains the file surface.', owner: 'slice 2, signed-out window (spec C §11, row #21)', count: 1 },
  { file: 'Onboarding.tsx', text: 'Engine', owner: 'slice 2, signed-out window (spec C §11, row #21)', count: 1 },
  { file: 'Onboarding.tsx', text: 'Open control center', owner: 'slice 2, signed-out window (spec C §11, row #21)', count: 1 },
]
/** Raising either number is how a string is pardoned: it has to be done here, in the diff, on purpose. */
const PENDING_ENTRIES = 6
const PENDING_OCCURRENCES = 6

describe('the guard itself', () => {
  const flagged = (source: string, file = 'x.tsx', commands: string[] = []) => violationsInSource(file, source, commands).violations.map((v) => v.rule)

  test('every word of the §10.1 list is flagged in JSX text, in an attribute, in a property and in a sentence', () => {
    const samples: Record<string, string> = {
      daemon: 'the daemon is busy',
      hydrate: 'it will hydrate the file',
      engine: 'the sync engine stopped',
      'control center': 'open the control center',
      'file surface': 'Finder is the file surface',
      namespace: 'a remote-first namespace',
      'state DB': 'recorded in the local state DB',
      socket: 'clears the local socket',
      telemetry: 'live telemetry',
      heartbeat: 'a missed heartbeat',
      wired: 'is not wired up yet',
      'this build': 'not available in this build',
      'Future native wiring': 'Future native wiring should clear it',
    }
    expect(Object.keys(samples).sort()).toEqual(DEVELOPER_WORDS.map((w) => w.id).sort())
    for (const [id, sentence] of Object.entries(samples)) {
      expect(flagged(`const A = () => <p>${sentence}</p>`), `${id} in JSX text`).toContain(id)
      expect(flagged(`const A = () => <button aria-label="${sentence}">x</button>`), `${id} in aria-label`).toContain(id)
      expect(flagged(`showToast({ title: 'Oops', message: '${sentence}' })`), `${id} in message`).toContain(id)
      expect(flagged(`const note = cond ? 'fine words here' : '${sentence}'`), `${id} in a sentence`).toContain(id)
      expect(flagged('const t = `' + sentence + ' for ${name}`'), `${id} in a template`).toContain(id)
    }
  })

  test('the words match as words, in any case, and not inside longer words', () => {
    expect(flagged('const a = <p>The DAEMON wakes</p>')).toContain('daemon')
    expect(flagged('const a = <p>Control Centre</p>')).toContain('control center')
    expect(flagged('const a = <p>It hydrates, then uploads</p>')).toContain('hydrate')
    expect(flagged('const a = <p>an engineer wired this up in the genuine engineering style</p>')).toEqual(['wired'])
    expect(flagged('const a = <p>the socketed adapter and an enginery of words</p>')).toEqual([])
  })

  test('every string-valued JSX attribute is copy unless it is on the machine denylist: a custom component\'s copy, footnote, confirmLabel, detail', () => {
    for (const name of ['copy', 'footnote', 'confirmLabel', 'detail', 'body', 'caption', 'actionLabel', 'emptyText', 'aNameNobodyHasMetYet']) {
      expect(flagged(`const A = () => <Card ${name}="Finder remains the file surface." />`), name).toEqual(['file surface'])
      expect(flagged(`const A = () => <Card ${name}={'the daemon wakes'} />`), `${name} as an expression`).toEqual(['daemon'])
      expect(flagged(`const A = () => <Card ${name}={ok ? 'fine words' : 'the daemon wakes'} />`), `${name} as a ternary`).toEqual(['daemon'])
    }
  })

  test('the machine denylist: a class, an id, a role, a link, a key, a type, data-*, a name, the SVG geometry', () => {
    const quiet = [
      '<div className="the daemon room" />',
      '<div id="the daemon" role="the control center" />',
      '<a href="https://example.com/the daemon" />',
      '<input type="the socket" name="the engine room" />',
      '<li key="the telemetry feed" data-row="the heartbeat monitor" />',
      '<path d="M0 0 the daemon" viewBox="0 0 24 the hydrate" />',
      '<Note kind="alert" surface="the control center" />',
    ]
    for (const jsx of quiet) expect(flagged(`const A = () => ${jsx}`), jsx).toEqual([])
  })

  test('strings inside calls and arrays are read: a call that builds copy, an array of lines, a nested ternary', () => {
    expect(flagged(`const A = () => <Card lines={['fine words here', 'then the daemon wakes']} />`)).toEqual(['daemon'])
    expect(flagged(`const A = () => <Card title={format('the daemon is busy', n)} />`)).toEqual(['daemon'])
    expect(flagged(`const lines = [['one two', 'the control center'], ['three four']]`)).toEqual(['control center'])
    expect(flagged(`const A = () => <Card copy={a ? (b ? 'fine words' : 'the socket is open') : 'other words'} />`)).toEqual(['socket'])
    expect(flagged(`showToast({ title: 'Oops', message: fmt('the daemon is busy') })`)).toEqual(['daemon'])
  })

  test('a string inside a machine call is still not copy, even inside a copy attribute', () => {
    expect(flagged(`const A = () => <Card copy={ok ? t('the daemon') : x.includes('the daemon here')} />`)).toEqual(['daemon'])
  })

  test('the ratchet: a pardon covers exactly its count, a repeat is a new violation, a vanished string makes the pardon stale', () => {
    const v = (line: number, text: string, rule = 'engine') => ({ file: 'Onboarding.tsx', line, text, how: 'jsx-text', rule })
    const one: Pardon[] = [{ file: 'Onboarding.tsx', text: 'Engine', owner: 'slice 2, signed-out window', count: 1 }]
    expect(pardon([v(10, 'Engine')], one)).toEqual({ open: [], stale: [] })
    // one string, two rules: still one occurrence
    expect(pardon([v(10, 'Engine'), v(10, 'Engine', 'daemon')], one)).toEqual({ open: [], stale: [] })
    // the same words again, on another line: pardoned once, the second is a violation
    const second = pardon([v(10, 'Engine'), v(300, 'Engine')], one)
    expect(second.open.map((x) => x.line)).toEqual([300])
    expect(second.stale).toEqual([])
    // the same words in another file are not pardoned
    expect(pardon([{ ...v(10, 'Engine'), file: 'Other.tsx' }], one).open).toHaveLength(1)
    // the string is gone: the pardon says so
    expect(pardon([], one).stale).toHaveLength(1)
    // a pardon for two that found one is stale too
    expect(pardon([v(10, 'Engine')], [{ ...one[0], count: 2 }]).stale).toHaveLength(1)
    // an unrelated violation is open
    expect(pardon([v(5, 'Hydrates files')], one).open).toHaveLength(1)
  })

  test('machine positions are not copy: a command, a case, a comparison, a class, a type, a match, a comment', () => {
    const quiet = [
      `command('hydrate the daemon')`,
      `invoke('engine control center')`,
      `switch (x) { case 'the daemon is busy': break }`,
      `if (kind === 'the sync engine') run()`,
      `const A = () => <div className="engine room daemon" id="state db" />`,
      `type Kind = 'the daemon' | 'the sync engine'`,
      `if (reason.includes('sync engine stopped')) run()`,
      `// the daemon and the control center, in a comment`,
      `/* hydrate the namespace */ const a = 1`,
      `import x from './the daemon and engine'`,
      `console.warn('engine is not wired in this build')`,
    ]
    for (const source of quiet) expect(flagged(source), source).toEqual([])
  })

  test('a function that builds copy is still read: a handler that raises a toast is not a machine position', () => {
    expect(flagged(`const A = () => <button onClick={() => showToast({ title: 'Oops', message: 'the daemon is busy' })}>x</button>`)).toEqual(['daemon'])
  })

  test('a raw command name inside a sentence is flagged; the name alone, as a command, is not', () => {
    const commands = ['list_remote_tree', 'set_recursive_pin']
    expect(flagged('const A = () => <p>The UI is ready for list_remote_tree and set_recursive_pin.</p>', 'x.tsx', commands)).toEqual(['raw command name'])
    expect(flagged(`await command('list_remote_tree')`, 'x.tsx', commands)).toEqual([])
    expect(flagged('const A = () => <p>Folders you choose stay on this Mac.</p>', 'x.tsx', commands)).toEqual([])
  })

  test('the strings it reads are found by the parser, so a multi-line paragraph is one string, at the line of its first word', () => {
    const found = personFacingStrings('x.tsx', ['const A = () => (', '  <p>', '    one two', '    three', '  </p>', ')'].join('\n'))
    expect(found).toEqual([{ file: 'x.tsx', line: 3, text: 'one two three', how: 'jsx-text' }])
  })

  test('reachability follows static imports from the roots, not type imports, and skips what no root reaches', () => {
    const dir = mkdtempSync(join(tmpdir(), 'copy-guard-'))
    try {
      mkdirSync(join(dir, 'src', 'pages'), { recursive: true })
      writeFileSync(join(dir, 'src', 'main.tsx'), `import MacApp from './MacApp'\nimport WindowsApp from './WindowsApp'\nimport './design.css'\n`)
      writeFileSync(join(dir, 'src', 'MacApp.tsx'), `import { a } from './pages/a'\nimport type { T } from './typeOnly'\nexport default () => null\n`)
      writeFileSync(join(dir, 'src', 'WindowsApp.tsx'), `import { w } from './windowsOnly'\nexport default () => null\n`)
      writeFileSync(join(dir, 'src', 'pages', 'a.tsx'), `import { b } from '../b'\nexport const a = 1\n`)
      writeFileSync(join(dir, 'src', 'b.ts'), `export const b = 'a string with the daemon in it'\n`)
      writeFileSync(join(dir, 'src', 'typeOnly.ts'), `export type T = 1\n`)
      writeFileSync(join(dir, 'src', 'windowsOnly.ts'), `export const w = 'the daemon on Windows'\n`)
      expect(mainImports(dir).sort()).toEqual(['MacApp.tsx', 'WindowsApp.tsx'])
      expect(macRoots(dir)).toEqual(['MacApp.tsx'])
      expect(reachableModules(dir)).toEqual(['MacApp.tsx', 'b.ts', 'pages/a.tsx'])
      const found = scan(dir)
      expect(found.modules).toEqual(['MacApp.tsx', 'b.ts', 'pages/a.tsx'])
      // `b.ts` holds a string with the word, but it is an export constant with no copy position: the sentence rule reads it.
      expect(found.violations.map((v) => `${v.file}:${v.rule}`)).toEqual(['b.ts:daemon'])
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })

  test('the Windows-only windows are exactly the three main.tsx mounts only for platform=windows', () => {
    expect([...WINDOWS_ONLY]).toEqual(['WindowsApp.tsx', 'WindowsTray.tsx', 'WindowsFirstRun.tsx'])
    const main = readFileSync(join(ROOT, 'src/main.tsx'), 'utf8')
    for (const file of WINDOWS_ONLY) expect(main).toContain(`from './${file.replace(/\.tsx$/, '')}'`)
    // The window components a Mac can mount. One that `main.tsx` gains has to be named here, on purpose, or the scan
    // would quietly start reading it (and a slice that stops mounting one on a Mac removes it from this list).
    const components = macRoots(ROOT).filter((file) => /^[A-Z][A-Za-z]*\.tsx$/.test(file)).sort()
    expect(components).toEqual(['AccountSessionBoundary.tsx', 'App.tsx', 'ConflictWindow.tsx', 'MacApp.tsx', 'Onboarding.tsx'])
  })

  test('the command names come from Rust\'s handler list (more than a hundred, none of them empty)', () => {
    const names = commandNames(ROOT)
    expect(names.length).toBeGreaterThan(100)
    expect(names).toContain('set_desktop_config')
    expect(names).toContain('account_security_score')
    expect(names.every((name) => /^[a-z0-9_]+$/.test(name))).toBe(true)
  })
})

describe('what a Mac can read', () => {
  const result = scan(ROOT)
  const left = pardon(result.violations, PENDING)

  test('the scan did its work: the windows a Mac mounts are all in it, and so are hundreds of strings', () => {
    for (const file of ['MacApp.tsx', 'MacSettings.tsx', 'desktopApi.ts', 'App.tsx', 'pages/Status.tsx', 'windows/ui.tsx']) {
      expect(result.modules, file).toContain(file)
    }
    for (const file of WINDOWS_ONLY) expect(result.modules, file).not.toContain(file)
    expect(result.modules.length).toBeGreaterThanOrEqual(40)
    expect(result.strings).toBeGreaterThanOrEqual(500)
  })

  test('no string a person reads on a Mac holds a developer word or a raw command name (§10.1)', () => {
    expect(left.open.map(describeViolation)).toEqual([])
  })

  test('every pending entry still holds exactly, so the list shrinks the moment its owner removes a string', () => {
    expect(left.stale).toEqual([])
  })

  test('the pardon list is pinned: its size, its total, and an owner on every entry', () => {
    expect(PENDING).toHaveLength(PENDING_ENTRIES)
    expect(PENDING.reduce((total, entry) => total + entry.count, 0)).toBe(PENDING_OCCURRENCES)
    for (const entry of PENDING) {
      expect(entry.owner.trim().length, entry.text).toBeGreaterThan(10)
      expect(Number.isInteger(entry.count) && entry.count >= 1, entry.text).toBe(true)
    }
  })

  test('a mutated copy of the real tree goes red: a developer word put in a Mac-only module, a shared one and a legacy page is found each time', () => {
    const scratch = mkdtempSync(join(tmpdir(), 'copy-guard-real-'))
    try {
      cpSync(join(ROOT, 'src'), join(scratch, 'src'), { recursive: true })
      mkdirSync(join(scratch, 'src-tauri/src'), { recursive: true })
      cpSync(join(ROOT, 'src-tauri/src/lib.rs'), join(scratch, 'src-tauri/src/lib.rs'))
      const before = scan(scratch).violations.length
      const inject = (file: string, from: string, to: string) => {
        const path = join(scratch, 'src', file)
        const source = readFileSync(path, 'utf8')
        expect(source, `${file} holds the anchor`).toContain(from)
        writeFileSync(path, source.replace(from, to))
      }
      inject('MacSettings.tsx', 'label="Beebeeb in Finder"', 'label="Beebeeb in Finder, hydrated by the daemon"')
      expect(scan(scratch).violations.length).toBe(before + 2) // hydrate, daemon
      inject('desktopApi.ts', 'export function commandUnavailableLabel(_commandName: string): string {', 'export const SHARED_NOTE = { message: \'Telemetry from the socket heartbeat\' }\nexport function commandUnavailableLabel(_commandName: string): string {')
      expect(scan(scratch).violations.length).toBe(before + 5) // telemetry, socket, heartbeat
      const all = scan(scratch).violations.map((v) => `${v.file}:${v.rule}`)
      expect(all).toContain('MacSettings.tsx:daemon')
      expect(all).toContain('desktopApi.ts:socket')
    } finally {
      rmSync(scratch, { recursive: true, force: true })
    }
  })
})

describe('what a command says (Rust sentences, which reach the screen as the reason of a toast)', () => {
  test('a sentence is found, a log line, a test item and a comment are not, and "engine" is Rust\'s own refusal word', () => {
    const source = [
      'fn a() -> Result<(), String> { Err("Start the daemon once before changing it.".to_string()) }',
      'fn b() { tracing::warn!("The daemon is not running yet."); }',
      '// "The daemon is a word in a comment."',
      'fn c() { assert!(ok, "The daemon must answer within a second."); }',
      'fn d() -> &\'static str { "The sync engine did not confirm it stopped. Restart Beebeeb." }',
      'fn e() -> String { format!("open state db: {e}") }',
      '#[cfg(test)]',
      'mod tests { fn t() { let _ = "The daemon answers in a test."; } }',
    ].join('\n')
    expect(rustSentencesIn('x.rs', source).map((s) => s.text)).toEqual([
      'Start the daemon once before changing it.',
      'The sync engine did not confirm it stopped. Restart Beebeeb.',
    ])
  })

  test('no sentence a command answers with holds a developer word', () => {
    const result = scanRust(ROOT)
    expect(result.files).toBeGreaterThanOrEqual(50)
    expect(result.sentences).toBeGreaterThanOrEqual(100)
    expect(result.violations.map(describeViolation)).toEqual([])
  })

  test('a mutated copy of the real tree goes red: a developer word in a command\'s error is found', () => {
    const scratch = mkdtempSync(join(tmpdir(), 'copy-guard-rust-'))
    try {
      cpSync(join(ROOT, 'src-tauri/src'), join(scratch, 'src-tauri/src'), { recursive: true })
      expect(scanRust(scratch).violations).toEqual([])
      const path = join(scratch, 'src-tauri/src/lib.rs')
      const source = readFileSync(path, 'utf8')
      const anchor = 'Choose a sync folder before changing offline availability.'
      expect(source).toContain(anchor)
      writeFileSync(path, source.replace(anchor, 'Choose a sync folder, then start the daemon, before changing offline availability.'))
      expect(scanRust(scratch).violations.map((v) => `${v.file}:${v.rule}`)).toEqual(['src-tauri/src/lib.rs:daemon'])
    } finally {
      rmSync(scratch, { recursive: true, force: true })
    }
  })
})
