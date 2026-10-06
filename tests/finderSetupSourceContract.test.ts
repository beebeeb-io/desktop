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
