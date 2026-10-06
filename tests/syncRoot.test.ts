import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { createElement, Fragment } from 'react'
import ts from 'typescript'
import { command, loadSyncStatus, type SyncStatus } from '../src/desktopApi'
import * as finderInstallCard from '../src/finderInstallCard'
import { finderOpenFailedToast } from '../src/finderSetup'
import { FINDER_OPEN_FAILED } from '../src/finderSetupCopy'

// Execute the production component declarations/handlers with controlled hooks.
// This does not claim browser layout, React scheduling, or native Explorer proof.
// No copy of the path-selection logic lives in the test harness.
function component(file: string, name: string, bindings: Record<string, unknown>) {
  const source = readFileSync(new URL(`../src/${file}`, import.meta.url), 'utf8')
  const ast = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX)
  const declaration = ast.statements.find(n => ts.isFunctionDeclaration(n) && n.name?.text === name)
  if (!declaration) throw new Error(`Missing component ${name}`)
  const compiled = ts.transpileModule(declaration.getText(ast).replace(/^export (default )?/, ''), {
    compilerOptions: { jsx: ts.JsxEmit.React, target: ts.ScriptTarget.ES2022 },
  }).outputText
  return new Function(...Object.keys(bindings), `${compiled}; return ${name}`)(...Object.values(bindings))
}
function elements(node: any): any[] {
  if (Array.isArray(node)) return node.flatMap(elements)
  return node?.props ? [node, ...elements(node.props.children)] : []
}
function content(node: any): string {
  if (Array.isArray(node)) return node.map(content).join(' ')
  if (node?.props) return content(node.props.children)
  return node == null || typeof node === 'boolean' ? '' : String(node)
}
const customRoot = 'D:\\Private files\\資料\\Sync'
const suggestion = 'C:\\Users\\Fixture\\Beebeeb'
const initialStatus: SyncStatus = { logged_in: true, engine: 'running', sync_root: customRoot, syncing: 0, cloud_only: 0, conflicts: 0 }

function harness(file: string, name: string, overrides: Record<string, unknown> = {}) {
  const states: any[] = []
  const deps: any[][] = []
  const effects: Array<() => unknown> = []
  const intervals: Array<() => Promise<void>> = []
  const calls: Array<{ name: string; args: any }> = []
  const toasts: any[] = []
  const failures = new Map<string, string>()
  let cursor = 0
  let backend: SyncStatus | null = { ...initialStatus }
  let props: any = { status: backend }
  let tree: any
  const previousWindow = globalThis.window
  const fakeWindow = {
    setInterval(fn: any) { intervals.push(fn); return intervals.length }, clearInterval() {},
    __TAURI_INTERNALS__: { invoke: async (name: string, args: any) => {
      calls.push({ name, args })
      if (failures.has(name)) throw new Error(failures.get(name))
      if (name === 'sync_status') {
        if (!backend) throw new Error('status unavailable')
        return backend
      }
      if (name === 'pick_sync_root') return backend?.sync_root ?? null
      if (name === 'desktop_platform') return 'windows'
      if (name === 'finder_location_state') return { installed: true, path: suggestion }
      if (name === 'default_sync_root') return suggestion
      if (name === 'open_finder_location') return undefined
      if (name === 'windows_shell_integration_state') return { installed: true, path: suggestion }
      throw new Error(`Unexpected command: ${name}`)
    } },
  }
  globalThis.window = fakeWindow as any
  const View = component(file, name, {
    React: { createElement, Fragment }, T: {}, command, loadSyncStatus, ...finderInstallCard,
    useState(initial: any) {
      const index = cursor++
      if (!(index in states)) states[index] = typeof initial === 'function' ? initial() : initial
      return [states[index], (next: any) => { states[index] = typeof next === 'function' ? next(states[index]) : next }]
    },
    useEffect(fn: any, next: any[]) {
      const index = cursor++
      if (!deps[index] || next.some((v, i) => v !== deps[index][i])) effects.push(fn)
      deps[index] = next
    },
    useMemo: (fn: any) => fn(), useToast: () => ({ showToast: (toast: any) => toasts.push(toast) }),
    Card: 'card', NavIcon: 'icon', PrimaryBtn: 'button', PageHeader: 'header',
    SyncFolderCard: 'sync-folder', FilesLoadingState: 'loading', FilesErrorState: 'error',
    FilesEmptyState: 'empty', ManageBackupCard: 'backup', FilesMetricStrip: 'metrics',
    ByStatusBar: 'by-status', RecentlyChanged: 'recent', FILES_RECENT_LIMIT: 15,
    desktopFileOverview: async () => ({ ok: true, value: { total_files: 0, total_bytes: 0, recent: [] } }),
    commandUnavailableLabel: () => 'Unavailable',
    usePlatformName: () => 'windows',
    // SyncFolder calls the macOS Finder hook and the capability snapshot unconditionally (a hook
    // cannot be conditional). Here, on Windows, neither has anything to say: the real hook's
    // behaviour off a Mac is pinned in tests/finderInstallOneSurface.test.tsx and useFinderSetup.test.ts.
    finderOpenFailedToast,
    useFinderSetup: () => ({ load: { status: 'loading' }, presentation: { kind: 'quiet', line: '' }, retry: async () => {}, run: async () => ({ ok: true, value: undefined }) }),
    useCapabilities: () => null,
    usePlatform: () => ({ name: 'windows', resolved: true }), useRegionLabel: () => 'End-to-end encrypted',
    shellIntegrationLabel: () => 'Explorer integration', thisDeviceNoun: () => 'this PC',
    shellIntegrationCommandsFor: () => ({ state: 'windows_shell_integration_state', install: 'install_windows_shell_integration' }),
    SettingsSectionShell: 'section', Chip: 'chip',
    getCurrentWindow: () => ({ onFocusChanged: async () => () => {}, hide: async () => {} }),
    BrandMark: 'brand', ActionIcon: 'icon', FileGlyph: 'glyph', ActionButton: 'action',
    TRAY_RECENT_LIMIT: 50, TRAY_VIEW_ALL_THRESHOLD: 8,
    ...overrides,
  })
  function render() { cursor = 0; tree = View(props) }
  async function flush() {
    for (let i = 0; i < 8; i++) { while (effects.length) effects.shift()!(); await Promise.resolve() }
    render()
  }
  render()
  return {
    calls, toasts, failCommand(name: string, reason: string) { failures.set(name, reason) }, setBackend(next: SyncStatus | null) { backend = next }, elements: () => elements(tree), content: () => content(tree),
    async refresh(next = backend) { backend = next; props = { status: await loadSyncStatus() }; for (const fn of intervals) await fn(); render(); await flush() },
    setProps(next: any) { props = next; render() }, flush,
    close() { globalThis.window = previousWindow },
  }
}

describe('runtime sync root (1646)', () => {
  test('Files renders the backend status root, updates it, and does not ask for a suggestion', async () => {
    const h = harness('WindowsApp.tsx', 'SyncFolderCard')
    try {
      await h.refresh()
      expect(h.content()).toContain(customRoot)
      expect(h.content()).not.toContain(suggestion)
      expect(h.calls.filter(c => c.name === 'sync_status')).toHaveLength(1)
      expect(h.calls.filter(c => c.name === 'default_sync_root')).toHaveLength(0)
      const changed = 'E:\\Another root'
      await h.refresh({ ...initialStatus, sync_root: changed })
      expect(h.content()).toContain(changed)
      expect(h.content()).not.toContain(customRoot)
      expect(h.elements().some(e => e.props.title === changed)).toBe(true)
    } finally { h.close() }
  })

  test('Mac Files keeps the named Finder location actionable with its intentionally null status root', async () => {
    const h = harness('WindowsApp.tsx', 'SyncFolderCard', { usePlatformName: () => 'macos' })
    try {
      await h.refresh({ ...initialStatus, sync_root: null })
      expect(h.content()).toContain('Beebeeb in Finder')
      expect(h.content()).toContain('Open in Finder')
      const button = h.elements().find(e => e.type === 'button')
      expect(button.props.disabled).toBe(false)
      await button.props.onClick(); await h.flush()
      expect(h.calls.filter(c => c.name === 'open_finder_location')).toEqual([{ name: 'open_finder_location', args: {} }])
    } finally { h.close() }
  })

  // Task 17b (lead ruling T4-⚠2): on a Mac every FpError reaching the frontend is redacted to a
  // domain and a code, so a failed "Open in Finder" says one sentence and never `result.reason`.
  // Off a Mac the card keeps its own title and the error text, and never evaluates the macOS helper.
  test('Mac Files: a failed Open in Finder is one toast with the one sentence, and the bridge code is rendered nowhere', async () => {
    const h = harness('WindowsApp.tsx', 'SyncFolderCard', { usePlatformName: () => 'macos' })
    try {
      h.failCommand('open_finder_location', 'io.beebeeb.bridge 3')
      await h.refresh({ ...initialStatus, sync_root: null })
      const button = h.elements().find(e => e.type === 'button')
      await button.props.onClick(); await h.flush()
      expect(h.calls.filter(c => c.name === 'open_finder_location')).toHaveLength(1)
      expect(h.toasts).toEqual([{ variant: 'error', message: FINDER_OPEN_FAILED }])
      expect(JSON.stringify(h.toasts)).not.toContain('io.beebeeb')
      expect(h.content()).not.toContain('io.beebeeb')
    } finally { h.close() }
  })

  for (const [file, name, label] of [
    ['WindowsApp.tsx', 'SyncFolderCard', 'Open in Explorer'],
    ['WindowsTray.tsx', 'WindowsTray', 'Open folder'],
    ['pages/SyncFolder.tsx', 'SyncFolder', 'Open in Finder'],
  ] as const) {
    test(`${name} on Windows: a failed open keeps its title and shows the error text (unchanged, task 17b)`, async () => {
      const reason = 'open Explorer: access is denied'
      const title = name === 'SyncFolder' ? 'Couldn’t open the sync folder' : 'Couldn’t open folder'
      const h = harness(file, name, { finderOpenFailedToast: () => { throw new Error('the macOS helper was reached on Windows') } })
      try {
        h.failCommand('open_finder_location', reason)
        await h.refresh()
        const action = h.elements().find(e => e.props.label === label || (e.type === 'button' && content(e).trim() === label))
        await action.props.onClick(); await h.flush()
        expect(h.calls.filter(c => c.name === 'open_finder_location')).toEqual([{ name: 'open_finder_location', args: { path: customRoot } }])
        expect(h.toasts).toEqual([{ variant: 'error', title, message: reason }])
      } finally { h.close() }
    })
  }

  test('Files passes the current backend root when opening', async () => {
    const h = harness('WindowsApp.tsx', 'SyncFolderCard')
    try {
      await h.refresh()
      const button = h.elements().find(e => e.type === 'button')
      expect(button.props.disabled).toBe(false)
      await button.props.onClick(); await h.flush()
      expect(h.calls.filter(c => c.name === 'open_finder_location')).toEqual([{ name: 'open_finder_location', args: { path: customRoot } }])
    } finally { h.close() }
  })

  for (const [file, name, label] of [
    ['WindowsApp.tsx', 'SyncFolderCard', 'Open in Explorer'],
    ['WindowsTray.tsx', 'WindowsTray', 'Open folder'],
    ['pages/SyncFolder.tsx', 'SyncFolder', 'Open in Finder'],
  ]) {
    for (const outcome of ['cleared', 'changed', 'unavailable'] as const) {
      test(`${name}: root ${outcome} after last poll is resolved at click time`, async () => {
        const h = harness(file, name)
        try {
          await h.refresh()
          const action = () => h.elements().find(e => e.props.label === label ||
            (e.type === 'button' && content(e).trim() === label))
          expect(action().props.disabled).toBe(false)
          const changed = 'E:\\New current root'
          h.setBackend(outcome === 'unavailable' ? null : {
            ...initialStatus, sync_root: outcome === 'cleared' ? null : changed,
          })
          const reads = h.calls.filter(c => c.name === 'sync_status').length
          action().props.onClick()
          await h.flush()
          expect(h.calls.filter(c => c.name === 'sync_status')).toHaveLength(reads + 1)
          const opens = h.calls.filter(c => c.name === 'open_finder_location')
          if (outcome === 'changed') {
            expect(opens).toEqual([{ name: 'open_finder_location', args: { path: changed } }])
            expect(h.content()).toContain(changed)
          } else {
            expect(opens).toHaveLength(0)
            expect(action().props.disabled).toBe(true)
            expect(h.content()).toContain(outcome === 'cleared' ? 'Not configured on this PC yet' : 'Sync folder unavailable')
            expect(h.content()).not.toContain(customRoot)
          }
          expect(h.calls.filter(c => c.name === 'default_sync_root')).toHaveLength(0)
        } finally { h.close() }
      })
    }
  }

  test('compact settings recovers its root display after a failed click and successful folder selection', async () => {
    const h = harness('pages/SyncFolder.tsx', 'SyncFolder')
    try {
      await h.refresh()
      h.setBackend(null)
      h.elements().find(e => e.type === 'button' && content(e).trim() === 'Open in Finder').props.onClick()
      await h.flush()
      expect(h.content()).toContain('Sync folder unavailable')
      h.setBackend(initialStatus)
      h.elements().find(e => e.type === 'button' && content(e).trim() === 'Choose location').props.onClick()
      await h.flush()
      expect(h.content()).toContain(customRoot)
      expect(h.content()).not.toContain('Sync folder unavailable')
    } finally { h.close() }
  })

  for (const missing of ['unconfigured', 'unavailable'] as const) {
    test(`Files reports ${missing} and cannot open an invented root`, async () => {
      const h = harness('WindowsApp.tsx', 'SyncFolderCard')
      try {
        await h.refresh(missing === 'unconfigured' ? { ...initialStatus, sync_root: null } : null)
        expect(h.content()).toContain(missing === 'unconfigured' ? 'Not configured on this PC yet' : 'Sync folder unavailable')
        expect(h.content()).not.toContain(suggestion)
        expect(h.elements().find(e => e.type === 'button').props.disabled).toBe(true)
        expect(h.calls.filter(c => c.name === 'open_finder_location')).toHaveLength(0)
      } finally { h.close() }
    })
  }

  for (const result of [null, { ok: true, value: { total_files: 0, total_bytes: 0 } }, { ok: true, value: { total_files: 2, total_bytes: 10, recent: [] } }, { ok: false, reason: 'fixture error', unsupported: false }, { ok: false, reason: 'missing command', unsupported: true }]) {
    test(`Files has one status-backed folder card with overview ${JSON.stringify(result)}`, async () => {
      const h = harness('WindowsApp.tsx', 'FilesView', { desktopFileOverview: () => result ? Promise.resolve(result) : new Promise(() => {}) })
      try {
        await h.flush()
        const cards = h.elements().filter(e => e.type === 'sync-folder')
        expect(cards).toHaveLength(1)
        expect(cards[0].props.status?.sync_root).toBe(customRoot)
      } finally { h.close() }
    })
  }

  test('Settings displays status root even when the installation probe has a different path', async () => {
    const h = harness('windows/views/SettingsView.tsx', 'ExplorerIntegrationPanel')
    try {
      await h.refresh()
      expect(h.content()).toContain(customRoot)
      expect(h.content()).not.toContain(suggestion)
      await h.refresh({ ...initialStatus, sync_root: null })
      expect(h.content()).not.toContain(customRoot)
      expect(h.content()).toContain('Not configured on this PC yet')
    } finally { h.close() }
  })

  test('Tray displays the polled root and disables Open folder after status loses its root', async () => {
    const h = harness('WindowsTray.tsx', 'WindowsTray')
    try {
      await h.refresh()
      expect(h.content()).toContain(customRoot)
      const action = h.elements().find(e => e.props.label === 'Open folder')
      expect(action.props.disabled).toBe(false)
      await action.props.onClick(); await h.flush()
      expect(h.calls.filter(c => c.name === 'open_finder_location')).toEqual([{ name: 'open_finder_location', args: { path: customRoot } }])
      await h.refresh({ ...initialStatus, sync_root: null })
      expect(h.content()).not.toContain(customRoot)
      expect(h.content()).toContain('Not configured on this PC yet')
      expect(h.elements().find(e => e.props.label === 'Open folder').props.disabled).toBe(true)
    } finally { h.close() }
  })

  for (const [file, name, route, child] of [
    ['WindowsApp.tsx', 'renderContent', 'files', 'FilesView'],
    ['windows/views/SettingsView.tsx', 'renderPanel', 'explorer-integration', 'ShellOrFinderPanel'],
  ]) {
    test(`${name} passes its live status into ${child}`, () => {
      const source = readFileSync(new URL(`../src/${file}`, import.meta.url), 'utf8')
      const ast = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX)
      let expression: ts.Expression | undefined
      function visit(node: ts.Node) {
        if (ts.isVariableDeclaration(node) && node.name.getText(ast) === name) expression = node.initializer
        ts.forEachChild(node, visit)
      }
      visit(ast)
      if (!expression) throw new Error(`Missing production route ${name}`)
      const js = ts.transpileModule(`const render = ${expression.getText(ast)}`, {
        compilerOptions: { jsx: ts.JsxEmit.React, target: ts.ScriptTarget.ES2022 },
      }).outputText
      const bindings = { React: { createElement }, supportsRoute: () => true, caps: {}, loggedIn: true,
        activeNav: route, status: initialStatus, [child]: 'child' }
      const render = new Function(...Object.keys(bindings), `${js}; return render`)(...Object.values(bindings))
      expect(render().props.status).toBe(initialStatus)
    })
  }

  test('Tray action forwards disabled state to the native button', () => {
    const h = harness('WindowsTray.tsx', 'ActionButton')
    try {
      h.setProps({ icon: 'folder', label: 'Open folder', onClick() {}, disabled: true })
      expect(h.elements().find(e => e.type === 'button').props.disabled).toBe(true)
    } finally { h.close() }
  })
})
