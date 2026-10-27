/**
 * Task 2243: the tray's pollers read the state DB only while their window is on screen. On Guus's PC the hidden tray
 * read it every 3 and 5 seconds, each time from a cold connection (about 1,100 and 5,500 page reads).
 */
import { afterEach, beforeEach, describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import ts from 'typescript'
import { pollWhileVisible, windowVisible, type PollWindow } from '../src/windows/visiblePoll'

let intervals: Array<() => unknown>
let cleared: number[]
let previousWindow: unknown

beforeEach(() => {
  intervals = []
  cleared = []
  previousWindow = (globalThis as any).window
  ;(globalThis as any).window = {
    setInterval(fn: () => unknown) {
      intervals.push(fn)
      return intervals.length
    },
    clearInterval(id: number) {
      cleared.push(id)
    },
  }
})

afterEach(() => {
  ;(globalThis as any).window = previousWindow
})

/** A window whose visibility the test sets, and whose "shown again" listeners the test fires. */
function fakeWindow(visible: boolean) {
  const listeners: Array<() => void> = []
  let unsubscribed = 0
  const win: PollWindow & { visible: boolean } = {
    visible,
    async isVisible() {
      return win.visible
    },
    async onShown(listener) {
      listeners.push(listener)
      return () => {
        unsubscribed += 1
      }
    },
  }
  return {
    win,
    show() {
      win.visible = true
      for (const l of listeners) l()
    },
    unsubscribed: () => unsubscribed,
  }
}

async function settle() {
  for (let i = 0; i < 5; i++) await Promise.resolve()
}

async function fireTicks(n: number) {
  for (let i = 0; i < n; i++) for (const fn of intervals) await fn()
  await settle()
}

describe('pollWhileVisible (task 2243)', () => {
  test('a hidden window reads once on mount, then not on any tick', async () => {
    const { win } = fakeWindow(false)
    let reads = 0
    const stop = pollWhileVisible(() => { reads += 1 }, 3000, win)
    await settle()
    expect(reads).toBe(1)
    await fireTicks(10)
    expect(reads).toBe(1)
    stop()
  })

  test('a visible window reads on every tick', async () => {
    const { win } = fakeWindow(true)
    let reads = 0
    const stop = pollWhileVisible(() => { reads += 1 }, 3000, win)
    await fireTicks(4)
    expect(reads).toBe(5)
    stop()
  })

  test('showing the window reads at once, and the ticks read again while it stays shown', async () => {
    const shown = fakeWindow(false)
    let reads = 0
    const stop = pollWhileVisible(() => { reads += 1 }, 3000, shown.win)
    await fireTicks(3)
    expect(reads).toBe(1)
    shown.show()
    await settle()
    expect(reads).toBe(2)
    await fireTicks(2)
    expect(reads).toBe(4)
    shown.win.visible = false
    await fireTicks(3)
    expect(reads).toBe(4)
    stop()
  })

  test('stop clears the interval and the shown listener, and no read follows it', async () => {
    const shown = fakeWindow(true)
    let reads = 0
    const stop = pollWhileVisible(() => { reads += 1 }, 3000, shown.win)
    await settle()
    stop()
    expect(cleared).toEqual([1])
    expect(shown.unsubscribed()).toBe(1)
    await fireTicks(2)
    shown.show()
    await settle()
    expect(reads).toBe(1)
  })

  test('no window to ask (outside Tauri) reads on every tick, as before', async () => {
    let reads = 0
    const stop = pollWhileVisible(() => { reads += 1 }, 3000, null)
    await fireTicks(3)
    expect(reads).toBe(4)
    stop()
  })

  test('a window that cannot answer counts as visible', async () => {
    const broken: PollWindow = {
      isVisible: async () => { throw new Error('ipc down') },
      onShown: async () => () => undefined,
    }
    expect(await windowVisible(broken)).toBe(true)
    let reads = 0
    const stop = pollWhileVisible(() => { reads += 1 }, 3000, broken)
    await fireTicks(2)
    expect(reads).toBe(3)
    stop()
  })
})

/** The source text of the function `name` declared at the top level of `src/<file>`. */
function functionText(file: string, name: string): string {
  const source = readFileSync(new URL(`../src/${file}`, import.meta.url), 'utf8')
  const ast = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX)
  const found = ast.statements.find((n) => ts.isFunctionDeclaration(n) && n.name?.text === name)
  if (!found) throw new Error(`${name} not found in ${file}`)
  return found.getText(ast)
}

describe('the surfaces a hidden window keeps loaded poll through it (task 2243)', () => {
  const surfaces: Array<[string, string]> = [
    ['WindowsTray.tsx', 'WindowsTray'],
    ['windows/syncActivityUi.tsx', 'useSyncActivitySummary'],
    ['WindowsApp.tsx', 'WindowsApp'],
  ]
  for (const [file, name] of surfaces) {
    test(`${name} polls with pollWhileVisible, never a bare setInterval`, () => {
      const text = functionText(file, name)
      expect(text).toContain('pollWhileVisible(')
      expect(text).not.toContain('setInterval(')
    })
  }

  test('the tray reads only the newest rows, never the whole file overview', () => {
    const text = functionText('WindowsTray.tsx', 'WindowsTray')
    expect(text).toContain('desktopRecentFiles(TRAY_RECENT_LIMIT)')
    expect(text).not.toContain('desktopFileOverview')
  })

  test('an engine-status event reads the summary only while the window is on screen', () => {
    const text = functionText('windows/syncActivityUi.tsx', 'useSyncActivitySummary')
    expect(text).toMatch(/listen\(ENGINE_STATUS_EVENT[\s\S]*windowVisible\(win\)[\s\S]*refresh\(\)/)
  })
})
