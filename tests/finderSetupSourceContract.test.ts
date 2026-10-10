/**
 * Spec 2026-10-06 §13.1: "a source-contract test that no macOS code path calls
 * install_finder_location" — extended to every install-era Finder command. Static for the
 * macOS-only units, a census for everything else; the mixed pages are proven by behaviour
 * (tests/statusFinderSetup.test.tsx, the macOS describe in tests/finderInstallOneSurface.test.tsx),
 * including the failure paths a static check cannot see (a Mac whose desktop_platform read fails).
 *
 * What this file cannot show: a command named by a computed string. The census below reads the
 * string literals of every source file (any quoting style, template literals too, comments
 * excluded), so only `'install_' + 'finder_location'` slips through; nothing in src builds a
 * command name that way, and a review would flag the first.
 */
import { describe, expect, test } from 'bun:test'
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join, relative } from 'node:path'
import ts from 'typescript'
import { censusOfSource, familyLiterals, KEPT_FOLDER_READERS, type Site, type Violation, type ViolationKind } from './fixtures/finderResultCensus'

const SRC = new URL('../src/', import.meta.url).pathname
const FORBIDDEN_ON_MACOS = ['install_finder_location', 'continue_without_finder_location', 'finder_location_state', 'finder_domain_user_enabled']

function functionText(file: string, name: string): string {
  const source = readFileSync(join(SRC, file), 'utf8')
  const ast = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX)
  const found = ast.statements.find((n) => ts.isFunctionDeclaration(n) && n.name?.text === name)
  if (!found) throw new Error(`${name} not found in ${file}`)
  return found.getText(ast)
}

function allSources(dir = SRC, out = new Map<string, string>()): Map<string, string> {
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry)
    if (statSync(path).isDirectory()) allSources(path, out)
    else if (/\.(ts|tsx)$/.test(entry)) out.set(relative(SRC, path), readFileSync(path, 'utf8'))
  }
  return out
}

/** Every string literal in a source file: any quoting style, template literal parts included, comments excluded. */
function stringLiterals(file: string, text: string): string[] {
  const ast = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true, file.endsWith('x') ? ts.ScriptKind.TSX : ts.ScriptKind.TS)
  const found: string[] = []
  const visit = (node: ts.Node) => {
    if (ts.isStringLiteralLike(node) || ts.isTemplateHead(node) || ts.isTemplateMiddle(node) || ts.isTemplateTail(node)) found.push(node.text)
    ts.forEachChild(node, visit)
  }
  visit(ast)
  return found
}

const namesCommand = (file: string, text: string, command: string) => stringLiterals(file, text).some((literal) => literal.includes(command))

describe('no macOS code path calls the install-era Finder commands (R5)', () => {
  test('the census reads the whole of src, so an empty read cannot pass for a clean one', () => {
    const sources = allSources()
    expect(sources.size).toBeGreaterThan(30)
    for (const file of ['Onboarding.tsx', 'MacSettings.tsx', 'pages/SyncFolder.tsx', 'pages/Status.tsx', 'windows/views/SettingsView.tsx']) {
      expect(sources.has(file)).toBe(true)
    }
  })

  test('the census helper sees every quoting style and a template, and ignores a comment', () => {
    const cmd = 'finder_location_state'
    expect(namesCommand('a.ts', `command('${cmd}')`, cmd)).toBe(true)
    expect(namesCommand('a.ts', `command("${cmd}")`, cmd)).toBe(true)
    expect(namesCommand('a.ts', 'command(`' + cmd + '`)', cmd)).toBe(true)
    expect(namesCommand('a.ts', 'command(`' + cmd.slice(0, 7) + '${x}` + `' + cmd.slice(7) + '`)', cmd)).toBe(false)
    expect(namesCommand('a.tsx', `const x = <b title="${cmd}" />`, cmd)).toBe(true)
    expect(namesCommand('a.ts', `// the old \`${cmd}\` read\nconst x = 1`, cmd)).toBe(false)
  })

  test('macOS-only units never name them', () => {
    const units: Array<[string, string | null]> = [
      ['MacSettings.tsx', null],
      ['macSettingsModel.ts', null],
      ['finderSetup.ts', null],
      ['finderSetupCopy.ts', null],
      ['Onboarding.tsx', 'MacFinderStep'],
      ['windows/views/SettingsView.tsx', 'MacFinderIntegrationPanel'],
      ['pages/Status.tsx', 'macFinderSetupState'],
    ]
    for (const [file, fn] of units) {
      const text = fn ? functionText(file, fn) : readFileSync(join(SRC, file), 'utf8')
      const named = FORBIDDEN_ON_MACOS.filter((command) => text.includes(command))
      expect({ unit: `${file}${fn ? `#${fn}` : ''}`, named }).toEqual({ unit: `${file}${fn ? `#${fn}` : ''}`, named: [] })
    }
  })

  test('outside them, only the Windows/Linux branches that keep these commands name them', () => {
    const allowed: Record<string, string[]> = {
      install_finder_location: ['Onboarding.tsx', 'pages/SyncFolder.tsx'],
      continue_without_finder_location: ['Onboarding.tsx'],
      finder_location_state: ['Onboarding.tsx', 'pages/Status.tsx', 'pages/SyncFolder.tsx'],
      finder_domain_user_enabled: [],
    }
    const sources = [...allSources()]
    for (const [command, files] of Object.entries(allowed)) {
      const named = sources.filter(([file, text]) => namesCommand(file, text, command)).map(([file]) => file).sort()
      expect({ command, named }).toEqual({ command, named: [...files].sort() })
    }
  })

  test('Onboarding routes macOS to MacFinderStep; the old step is not reachable there', () => {
    expect(functionText('Onboarding.tsx', 'OnboardingView')).toMatch(/platform === 'macos' \? \(\s*<MacFinderStep/)
    expect(functionText('Onboarding.tsx', 'FinderInstallStep')).not.toContain('finder_domain_user_enabled')
  })

  test('the main-window Settings panel dispatches macOS to MacFinderIntegrationPanel', () => {
    expect(functionText('windows/views/SettingsView.tsx', 'ShellOrFinderPanel')).toMatch(/name === 'macos' \? <MacFinderIntegrationPanel/)
    expect(functionText('windows/views/SettingsView.tsx', 'shellIntegrationCommandsFor')).not.toContain('finder_')
  })

  test('the Windows/Linux Explorer panel carries no macOS install path any more', () => {
    const panel = functionText('windows/views/SettingsView.tsx', 'ExplorerIntegrationPanel')
    expect(FORBIDDEN_ON_MACOS.filter((command) => panel.includes(command))).toEqual([])
    expect(panel).not.toMatch(/finderInstall|open_login_items_and_extensions_settings/)
    expect(panel).not.toMatch(/['"]Install['"]|Installing/)
  })
})

describe('the mixed pages take the reconciler from useFinderSetup and never run it off a Mac (ruling 7b)', () => {
  const own = ['loadFinderSetup', 'subscribeFinderSetup', 'runFinderSetupAction', 'listen(']

  for (const [file, fn] of [
    ['pages/SyncFolder.tsx', 'SyncFolder'],
    ['pages/Status.tsx', 'Status'],
  ] as const) {
    test(`${fn} calls the hook, switched on only for a Mac, and holds no load/subscribe/action logic of its own`, () => {
      const body = functionText(file, fn)
      expect(body).toMatch(/useFinderSetup\(\{ enabled: platform === 'macos' \}\)/)
      expect(own.filter((name) => body.includes(name))).toEqual([])
    })
  }

  test('MacFinderIntegrationPanel takes the load, the subscription and the failed-action toast from the hook', () => {
    const body = functionText('windows/views/SettingsView.tsx', 'MacFinderIntegrationPanel')
    expect(body).toMatch(/useFinderSetup\(\)/)
    expect([...own, 'showToast'].filter((name) => body.includes(name))).toEqual([])
  })
})

/**
 * Task 17b (lead ruling T4-⚠2): a failed "Open in Finder" on a Mac is mapped to one sentence ONCE.
 * The two surfaces that invoke `open_finder_location` on a Mac take the toast from the helper; the
 * sentence itself is named by the copy module and that helper only. (`WindowsTray` also names the
 * command, but it opens only a sync root from the status, and a Mac's status never carries one
 * (`sync_status` returns `None` there, src-tauri/src/lib.rs). tests/syncRoot.test.ts pins that a
 * null root disables "Open folder" and sends no command, so the tray cannot reach it on a Mac.)
 */
describe('a failed Open in Finder on a Mac is mapped once (task 17b)', () => {
  test('both macOS call sites take the toast from finderOpenFailedToast and never write the sentence themselves', () => {
    for (const [file, fn] of [
      ['pages/SyncFolder.tsx', 'SyncFolder'],
      ['WindowsApp.tsx', 'SyncFolderCard'],
    ] as const) {
      const body = functionText(file, fn)
      expect(body).toContain('finderOpenFailedToast()')
      expect(body).not.toContain('FINDER_OPEN_FAILED')
    }
  })

  test('the sentence is named by the copy module and the helper only', () => {
    const named = [...allSources()].filter(([, text]) => text.includes('FINDER_OPEN_FAILED')).map(([file]) => file).sort()
    expect(named).toEqual(['finderSetup.ts', 'finderSetupCopy.ts'])
  })
})

/**
 * Task 17b, the sweep, fix round 1: a census of CALL SITES, not files. The first sweep listed the files
 * that name a Finder command; that proved a file named a command and nothing about what it rendered,
 * and it missed two paths (a successful repair's `warnings`, and `open_in_finder`). This one reads every
 * invocation of a Finder command (and of the wrappers and `finder.run` that hand its result on), finds
 * the variable that holds the result, and reports every use that can run on a Mac and renders the reason
 * or the warnings. The engine is tests/fixtures/finderResultCensus.ts, proved on snippets in
 * tests/finderResultCensus.test.ts; it fails closed on a guard it does not recognise.
 *
 * The tables below are the pinned truth as of this fix. A new call site, a new use, or a changed guard
 * fails here until somebody has looked at what it renders on a Mac.
 */
describe('the sweep: every call site of a Finder command, and what it does with the result on a Mac (task 17b)', () => {
  type Tally = Partial<Record<ViolationKind, number>>
  const tally = (violations: Violation[]): Tally => {
    const out: Tally = {}
    for (const v of violations) out[v.kind] = (out[v.kind] ?? 0) + 1
    return out
  }
  const sources = [...allSources()].filter(([file]) => file !== 'desktopApi.ts')
  const sites = sources.flatMap(([file, text]) => censusOfSource(file, text))
  const bySite = new Map<string, Site[]>()
  for (const site of sites) bySite.set(`${site.file} | ${site.family}`, [...(bySite.get(`${site.file} | ${site.family}`) ?? []), site])
  const column = <T,>(pick: (group: Site[]) => T): Record<string, T> =>
    Object.fromEntries([...bySite].sort(([a], [b]) => a.localeCompare(b)).map(([key, group]) => [key, pick(group)]))
  const only = (violations: Violation[], reachable: boolean) => tally(violations.filter((v) => v.macReachable === reachable))

  const TRAY = 'WindowsTray.tsx | open_finder_location'
  const OPEN_SETTINGS = 'pages/SyncFolder.tsx | open_login_items_and_extensions_settings'
  const RETRY = 'finderSetup.ts | wrapper:runFinderSetupAction'

  test('the census reads the whole of src and finds the call sites, so an empty read cannot pass for a clean one', () => {
    expect(sources.length).toBeGreaterThan(30)
    expect(sites.length).toBeGreaterThan(15)
    expect(familyLiterals('desktopApi.ts', readFileSync(join(SRC, 'desktopApi.ts'), 'utf8'))).toEqual({})
  })

  test('call sites: the pinned count of invocations per file and command', () => {
    expect(column((group) => group.length)).toEqual({
      'DesktopQuickSearch.tsx | open_in_finder': 1,
      'MacSettings.tsx | finder.run': 1,
      'MacSettings.tsx | reset_macos_integration': 1,
      'Onboarding.tsx | finder.run': 1,
      'Onboarding.tsx | wrapper:loadFinderSetup': 1,
      'WindowsApp.tsx | open_finder_location': 1,
      [TRAY]: 1,
      'finderSetup.ts | finder_setup_*': 4,
      'finderSetup.ts | open_login_items_and_extensions_settings': 1,
      'finderSetup.ts | wrapper:copyFinderSetupDetails': 1,
      'finderSetup.ts | wrapper:loadFinderSetup': 1,
      'finderSetup.ts | wrapper:runFinderSetupAction': 1,
      'pages/Shared.tsx | open_in_finder': 1,
      'pages/SyncFolder.tsx | finder.run': 1,
      'pages/SyncFolder.tsx | open_finder_location': 1,
      [OPEN_SETTINGS]: 1,
      'pages/SyncFolder.tsx | reset_macos_integration': 1,
      'windows/views/SettingsView.tsx | finder.run': 1,
    })
  })

  test('every string literal that names a Finder command, calls or not: a command reached through a variable shows here', () => {
    const literals = Object.fromEntries(
      sources.map(([file, text]) => [file, familyLiterals(file, text)]).filter(([, counts]) => Object.keys(counts as object).length > 0),
    )
    expect(literals).toEqual({
      'DesktopQuickSearch.tsx': { open_in_finder: 2 },
      'MacSettings.tsx': { reset_macos_integration: 1 },
      'WindowsApp.tsx': { open_finder_location: 2 },
      'WindowsTray.tsx': { open_finder_location: 1 },
      'finderSetup.ts': { 'finder_setup_*': 7, open_login_items_and_extensions_settings: 2 },
      'pages/Shared.tsx': { open_in_finder: 2 },
      'pages/SyncFolder.tsx': { open_finder_location: 2, open_login_items_and_extensions_settings: 2, reset_macos_integration: 2 },
    })
  })

  test('no use that can run on a Mac renders a reason or the warnings, except the three pinned exemptions', () => {
    const leaks = Object.fromEntries(
      Object.entries(column((group) => only(group.flatMap((site) => site.violations), true))).filter(([, found]) => Object.keys(found).length > 0),
    )
    expect(leaks).toEqual({ [TRAY]: { reason: 1 }, [OPEN_SETTINGS]: { reason: 1 }, [RETRY]: { reason: 2 } })
  })

  /**
   * Exemption 3 (must-render row 15, lead ruling on M3 / FA-M4): useFinderSetup.run shows the reason of
   * a failed Try again verbatim, because `finder_setup_retry` can only fail with Rust's fixed sentences,
   * which carry the remedy. This pins both halves: the hook reads the reason for `try_again` alone (and
   * not for an IPC-level `unsupported` failure), and the Rust command's every `Err` is a constant. The
   * hook's one other read is the 17b-M3 console trace, which keeps a `domain code` pair and turns any
   * other text into `other`: a console, not a surface.
   */
  test('exemption 3, useFinderSetup.run: only Try again\'s reason, and finder_setup_retry fails only with fixed sentences', () => {
    const run = functionText('finderSetup.ts', 'useFinderSetup').replace(/\/\/.*$/gm, '')
    expect(run.match(/result\.reason/g)).toHaveLength(2)
    const guard = run.indexOf("if (action === 'try_again' && !result.unsupported)")
    expect(guard).toBeGreaterThan(-1)
    expect(run.indexOf('setActionNote(result.reason)')).toBeGreaterThan(guard)
    const trace = run.indexOf('const said = result.reason.trim()')
    expect(trace).toBeGreaterThan(run.indexOf('} else {', guard))
    expect(run.slice(trace)).toMatch(/^const said = result\.reason\.trim\(\)\s*console\.warn\(action, result\.unsupported \? 'unsupported' : \/\^\[A-Za-z\]\[\\w\.\]\*\\s-\?\\d\+\$\/\.test\(said\) \? said : 'other'\)/)

    const lib = readFileSync(new URL('../src-tauri/src/lib.rs', import.meta.url), 'utf8')
    const body = (text: string, start: string) => text.slice(text.indexOf(start), text.indexOf('\n}\n', text.indexOf(start)))
    const retry = body(lib, 'fn finder_setup_retry_impl(state: &AppState) -> Result<(), String> {')
    expect(retry.match(/Err\(|\?;|\?\n/g)).toEqual(['?;', 'Err('])
    expect(retry).toContain('.ok_or_else(|| FINDER_SETUP_MACOS_ONLY.to_string())?;')
    expect(retry).toContain('return Err(FINDER_SETUP_HELD.to_string());')
    expect(retry.trim().endsWith('handle.trigger(finder_setup::core::Trigger::TryAgain)')).toBe(true)
    const tryAgain = body(lib, 'async fn finder_setup_try_again(state: &AppState, base_url: &str) -> Result<(), String> {')
    expect(tryAgain).not.toMatch(/\?;|Err\(/)
    expect(tryAgain.trim().endsWith('finder_setup_retry_impl(state)')).toBe(true)
    const driver = readFileSync(new URL('../src-tauri/src/finder_setup/driver.rs', import.meta.url), 'utf8')
    expect(body(driver, 'pub fn trigger(&self, trigger: Trigger) -> Result<(), String> {')).toContain('.map_err(|_| NOT_RUNNING.to_string())')
  })

  test('the uses that sit only on a non-macOS side are seen, so the analysis did read them (pinned)', () => {
    const seen = Object.fromEntries(
      Object.entries(column((group) => only(group.flatMap((site) => site.violations), false))).filter(([, found]) => Object.keys(found).length > 0),
    )
    expect(seen).toEqual({
      'DesktopQuickSearch.tsx | open_in_finder': { reason: 1 },
      'WindowsApp.tsx | open_finder_location': { reason: 1 },
      'pages/Shared.tsx | open_in_finder': { reason: 1 },
      'pages/SyncFolder.tsx | open_finder_location': { reason: 1 },
      // reason 2: the Reset's error text off a Mac, and (task 1882 r4, Windows and Linux only) its repair-removed check.
      'pages/SyncFolder.tsx | reset_macos_integration': { reason: 2, warnings: 2 },
    })
  })

  /**
   * Exemption 4 (task 1882, rebase onto main, lead ruling): MacSettings' Repair and SyncFolder's Reset hand the
   * repair's `.value` to the kept-folder readers, which show the folder macOS kept and nothing else. The census lets
   * them through by name, so this pins what they read: every use of their parameter is its `preserved_location`, or a
   * hand-off to another reader. A reader that starts reading `.reason` or `.warnings`, or passes its argument anywhere
   * else, turns this red.
   */
  test('exemption 4, the kept-folder readers: each reads only preserved_location, never a reason or the warnings', () => {
    expect([...KEPT_FOLDER_READERS].sort()).toEqual(['preservedFilesLine', 'preservedFilesNote'])
    const source = readFileSync(join(SRC, 'macSettingsModel.ts'), 'utf8')
    const ast = ts.createSourceFile('macSettingsModel.ts', source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS)
    for (const name of KEPT_FOLDER_READERS) {
      const fn = ast.statements.find((n): n is ts.FunctionDeclaration => ts.isFunctionDeclaration(n) && n.name?.text === name)
      if (!fn || !fn.body) throw new Error(`${name} not found in macSettingsModel.ts`)
      expect(fn.parameters).toHaveLength(1)
      const param = fn.parameters[0].name
      if (!ts.isIdentifier(param)) throw new Error(`${name} destructures its parameter`)
      const uses: string[] = []
      const visit = (node: ts.Node) => {
        if (ts.isIdentifier(node) && node.text === param.text && node !== param) {
          const parent = node.parent
          if (ts.isPropertyAccessExpression(parent) && parent.expression === node) uses.push(`.${parent.name.text}`)
          else if (ts.isCallExpression(parent) && parent.arguments.includes(node)) uses.push(`call:${parent.expression.getText(ast)}`)
          else uses.push(`other:${parent.getText(ast)}`)
        }
        ts.forEachChild(node, visit)
      }
      visit(fn.body)
      expect(uses.length).toBeGreaterThan(0)
      for (const use of uses) {
        const allowed = use === '.preserved_location' || KEPT_FOLDER_READERS.some((reader) => use === `call:${reader}`)
        expect({ reader: name, use, allowed }).toEqual({ reader: name, use, allowed: true })
      }
      expect(fn.body.getText(ast)).not.toMatch(/\.reason|\.warnings|\[['"](reason|warnings)['"]\]/)
    }
  })

  test('nothing reads the result of finder.run: it is discarded or handed to an event handler that discards it', () => {
    const runs = sites.filter((site) => site.family === 'finder.run')
    expect(runs).toHaveLength(4)
    for (const run of runs) expect(['returned', 'discarded']).toContain(run.consumption)
  })

  test('exemption 1, WindowsTray: it opens only the status root, and a Mac\'s status never has one', () => {
    const rust = readFileSync(new URL('../src-tauri/src/lib.rs', import.meta.url), 'utf8')
    expect(rust).toMatch(/#\[cfg\(target_os = "macos"\)\]\s*let sync_root = None::<String>;/)
    const tray = functionText('WindowsTray.tsx', 'WindowsTray')
    const open = tray.indexOf("command<void>('open_finder_location'")
    const guard = tray.indexOf('if (!current?.sync_root)')
    expect(guard).toBeGreaterThan(-1)
    expect(open).toBeGreaterThan(guard)
    expect(tray).toContain('disabled={opening || !status?.sync_root}')
  })

  test('exemption 2, SyncFolder.openSystemSettings: reachable only from the non-macOS user_disabled notice', () => {
    const text = readFileSync(join(SRC, 'pages/SyncFolder.tsx'), 'utf8')
    expect(text).toContain('const finderNotice = isMacos ? null :')
    expect(text.match(/openSystemSettings/g)).toHaveLength(2) // the definition and one use
    const notice = text.indexOf("finderNotice?.kind === 'user_disabled'")
    const use = text.indexOf('onClick={() => void openSystemSettings()}')
    expect(notice).toBeGreaterThan(-1)
    expect(use).toBeGreaterThan(notice)
  })

  test('the failed Reset, the failed open, and the failed show-file each take their Mac toast from one helper, and the sentences are named by the copy module and the helpers only', () => {
    const syncFolder = functionText('pages/SyncFolder.tsx', 'SyncFolder')
    expect(syncFolder).toContain('finderRepairFailedToast()')
    expect(syncFolder).toContain('finderRepairWarningNote(result.value)')
    for (const [file, fn] of [
      ['pages/Shared.tsx', 'Shared'],
      ['DesktopQuickSearch.tsx', 'DesktopQuickSearch'],
    ] as const) {
      const body = functionText(file, fn)
      expect(body).toContain('finderShowFileFailedToast()')
      expect(body).not.toMatch(/FINDER_[A-Z_]+/)
    }
    for (const [constant, files] of [
      ['FINDER_REPAIR_FAILED', ['finderSetup.ts', 'finderSetupCopy.ts']],
      ['FINDER_REPAIR_PARTIAL', ['finderSetupCopy.ts']],
      ['FINDER_SHOW_FILE_FAILED', ['finderSetup.ts', 'finderSetupCopy.ts']],
    ] as const) {
      const named = sources.filter(([, text]) => text.includes(constant)).map(([file]) => file).sort()
      expect({ constant, named }).toEqual({ constant, named: [...files] })
    }
  })

  test('MacSettings reports a failed repair without rendering a reason, and a repair\'s note never carries the warnings', () => {
    const body = readFileSync(join(SRC, 'MacSettings.tsx'), 'utf8')
    const repair = body.slice(body.indexOf('const runRepair'), body.indexOf('const toggleFolder'))
    expect(repair).toContain('reset_macos_integration')
    expect(repair).toContain('setRepairFailed(true)')
    expect(repair).toContain('repairNote(result.value)')
    expect(repair).not.toMatch(/\.reason|\.warnings|showToast/)
    // repairNote (the sanitiser it hands the value to) asks the copy module and never reads `.warnings`.
    const note = functionText('macSettingsModel.ts', 'repairNote')
    expect(note).toContain('finderRepairWarningNote(result)')
    expect(note).not.toContain('.warnings')
  })
})
