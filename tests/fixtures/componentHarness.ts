/**
 * Minimal harness that EXECUTES a production component declaration with controlled hooks
 * (same technique as `tests/syncRoot.test.ts`, extracted so task 1683's one-error-surface
 * tests can drive click -> await backend -> re-render without a DOM library).
 *
 * What it proves: the component's real handlers and real render output for a scripted
 * sequence of backend answers. What it does NOT prove: browser layout, React scheduling,
 * focus, or native macOS behaviour (the Playwright screenshots in the task evidence cover
 * the real-DOM side locally).
 */
import { readFileSync } from 'node:fs'
import { createElement, Fragment } from 'react'
import ts from 'typescript'

export function loadComponent(file: string, name: string, bindings: Record<string, unknown>) {
  const source = readFileSync(new URL(`../../src/${file}`, import.meta.url), 'utf8')
  const ast = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX)
  const declaration = ast.statements.find((n) => ts.isFunctionDeclaration(n) && n.name?.text === name)
  if (!declaration) throw new Error(`Missing component ${name} in ${file}`)
  const compiled = ts.transpileModule(declaration.getText(ast).replace(/^export (default )?/, ''), {
    compilerOptions: { jsx: ts.JsxEmit.React, target: ts.ScriptTarget.ES2022 },
  }).outputText
  return new Function(...Object.keys(bindings), `${compiled}; return ${name}`)(...Object.values(bindings))
}

export interface TreeNode {
  type: unknown
  props: Record<string, any>
}

export function elementsOf(node: any): TreeNode[] {
  if (Array.isArray(node)) return node.flatMap(elementsOf)
  return node?.props ? [node, ...elementsOf(node.props.children)] : []
}

export function textOf(node: any): string {
  if (Array.isArray(node)) return node.map(textOf).join('')
  if (node?.props) return textOf(node.props.children)
  return node == null || typeof node === 'boolean' ? '' : String(node)
}

/**
 * Resolve function components into the intrinsic elements they render, so text, roles and
 * handlers inside presentational components (task 1683 slice 4) are readable. Only meant for
 * components without hooks: they are called directly, once. Fragments and DOM elements pass
 * through; children are resolved recursively.
 */
export function expand(node: any): any {
  if (Array.isArray(node)) return node.map(expand)
  if (!node || typeof node !== 'object' || !node.props) return node
  if (typeof node.type === 'function') return expand(node.type({ ...node.props }))
  return { ...node, props: { ...node.props, children: expand(node.props.children) } }
}

export type Handler = (args: any) => unknown | Promise<unknown>

export interface Mounted {
  toasts: any[]
  calls: Array<{ name: string; args: any }>
  tree: () => any
  elements: () => TreeNode[]
  render: () => void
  flush: () => Promise<void>
  /**
   * Run the cleanup every effect returned (what React does when the component unmounts).
   * Cleanups run on unmount only, never on a dependency change, so a test that needs to count
   * live subscriptions must keep its effect dependencies stable.
   */
  unmount: () => void
  /** Click the button whose text is exactly `label`; resolves after the handler and a re-render. */
  click: (label: string) => Promise<void>
  /** Start the click but do not wait for it (to inspect the in-flight render). */
  clickNoWait: (label: string) => Promise<void>
  close: () => void
}

/**
 * A custom hook (or any function that calls hooks) from another module, executed with THIS mount's
 * controlled hooks so it and the component under test share one state/effect/ref store, as they do
 * in React. It is bound into the component's scope under `name`; `bindings` are what the hook
 * itself references (the component's own `bindings` are not visible to it).
 */
export interface HookModule {
  file: string
  name: string
  bindings?: Record<string, unknown>
}

export function mount(
  file: string,
  name: string,
  opts: {
    backend: Record<string, Handler>
    bindings?: Record<string, unknown>
    props?: any
    expand?: boolean
    hookModules?: HookModule[]
  },
): Mounted {
  const states: any[] = []
  const deps: any[][] = []
  const effects: Array<{ index: number; run: () => unknown }> = []
  const cleanups = new Map<number, () => void>()
  const toasts: any[] = []
  const calls: Array<{ name: string; args: any }> = []
  let cursor = 0
  let tree: any
  const previousWindow = (globalThis as any).window
  ;(globalThis as any).window = {
    setInterval: () => 0,
    clearInterval() {},
    // A timer fires on the next microtask (no real delay), so a debounced effect settles inside `flush`.
    setTimeout: (fn: () => void) => { queueMicrotask(fn); return 0 },
    clearTimeout() {},
    addEventListener() {},
    removeEventListener() {},
    __TAURI_INTERNALS__: {
      invoke: async (command: string, args: any) => {
        calls.push({ name: command, args })
        const handler = opts.backend[command]
        if (!handler) throw new Error(`unscripted command ${command}`)
        return handler(args)
      },
    },
  }
  const controlledHooks: Record<string, unknown> = {
    React: { createElement, Fragment },
    useState(initial: any) {
      const index = cursor++
      if (!(index in states)) states[index] = typeof initial === 'function' ? initial() : initial
      return [states[index], (next: any) => { states[index] = typeof next === 'function' ? next(states[index]) : next }]
    },
    useEffect(fn: any, next: any[]) {
      const index = cursor++
      if (!deps[index] || !next || next.some((v, i) => v !== deps[index][i])) effects.push({ index, run: fn })
      deps[index] = next
    },
    useMemo: (fn: any) => fn(),
    // Identity unless the caller asks for stable callbacks (opts.expand): a memoised
    // callback keeps an effect that depends on it from re-running on every render.
    useCallback(fn: any, next: any[]) {
      if (!opts.expand) return fn
      const index = cursor++
      const slot = states[index]
      if (slot && next && slot.deps && slot.deps.length === next.length && next.every((v: any, i: number) => v === slot.deps[i])) return slot.fn
      states[index] = { fn, deps: next }
      return fn
    },
    useRef(initial: any) {
      const index = cursor++
      if (!(index in states)) states[index] = { current: initial }
      return states[index]
    },
    useSyncExternalStore: (_subscribe: unknown, getSnapshot: () => unknown) => getSnapshot(),
    useToast: () => ({ showToast: (toast: any) => toasts.push(toast), dismissToast() {}, clearToasts() {} }),
  }
  const hookFunctions: Record<string, unknown> = {}
  for (const hook of opts.hookModules ?? []) {
    hookFunctions[hook.name] = loadComponent(hook.file, hook.name, { ...controlledHooks, ...hook.bindings })
  }
  const View = loadComponent(file, name, { ...controlledHooks, ...hookFunctions, ...opts.bindings })
  const render = () => { cursor = 0; tree = View(opts.props ?? {}) }
  const flush = async () => {
    for (let i = 0; i < 12; i++) {
      while (effects.length) {
        const { index, run } = effects.shift()!
        const cleanup = run()
        if (typeof cleanup === 'function') cleanups.set(index, cleanup as () => void)
      }
      await Promise.resolve()
    }
    render()
  }
  const unmount = () => {
    for (const cleanup of cleanups.values()) cleanup()
    cleanups.clear()
  }
  const view = () => (opts.expand ? expand(tree) : tree)
  const findButton = (label: string) => {
    const match = elementsOf(view()).find((el) => el.type === 'button' && textOf(el.props.children).trim() === label)
    if (!match) throw new Error(`no button "${label}" in the current render`)
    return match
  }
  render()
  return {
    toasts,
    calls,
    tree: () => tree,
    elements: () => elementsOf(view()),
    render,
    flush,
    unmount,
    click: async (label) => { await findButton(label).props.onClick(); await flush() },
    clickNoWait: async (label) => { void findButton(label).props.onClick(); render() },
    close() { (globalThis as any).window = previousWindow },
  }
}

/**
 * Every error surface currently visible: inline notices styled as errors (`notice error`),
 * anything explicitly marked `data-error-surface`, plus error toasts. Returned as labelled
 * strings so a failing assertion shows WHICH surfaces rendered.
 */
export function visibleErrorSurfaces(m: Mounted): string[] {
  const inline = m.elements()
    .filter((el) => {
      const cls = String(el.props.className ?? '')
      return (/\bnotice\b/.test(cls) && /\berror\b/.test(cls)) || el.props['data-error-surface'] != null
    })
    .map((el) => `inline: ${textOf(el.props.children).trim()}`)
  const toasts = m.toasts.filter((t) => t.variant === 'error').map((t) => `toast: ${t.title} — ${t.message}`)
  return [...inline, ...toasts]
}
