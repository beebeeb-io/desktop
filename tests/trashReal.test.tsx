// Trash lists only what the trash command returned. Restore reports that command's count and leaves a failed
// row in place, with the reason. Delete for good asks for the password only after the command requires it.
// Run by `bun test tests/trashReal.test.tsx` (no production file imports it). No other test mounts this section
// against drive_trash / drive_restore_from_trash / drive_delete_permanently.
import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { DriveSourceProvider, type TrashEntry } from '@beebeeb/shared/drive-ui'
import { allElements, installMiniDom, MiniElement, type MiniDomInstall, type MiniNode } from './fixtures/miniDom'

const QUARTERLY = 'Quarterly.pdf'
const SCAN = 'Scan.pdf'

let dom: MiniDomInstall
let React: typeof import('react')
let client: typeof import('react-dom/client')
let Section: typeof import('../src/v2/sections/trash').default
let ToastHost: typeof import('../src/v2/chrome/Toast').ToastHost
let createDesktopSource: typeof import('../src/v2/source/desktop').createDesktopSource

beforeAll(async () => {
  dom = installMiniDom('')
  Object.defineProperty(dom.document, 'oninput', { value: null, configurable: true })
  Object.defineProperty(MiniElement.prototype, 'type', {
    configurable: true,
    get() { return this.getAttribute('type') ?? (this.tagName === 'INPUT' ? 'text' : '') },
    set(value: string) { this.setAttribute('type', String(value)) },
  })
  MiniElement.prototype.querySelectorAll = function querySelectorAll(selector: string) {
    return allElements(this).slice(1).filter((el) => matches(el, selector))
  }
  MiniElement.prototype.querySelector = function querySelector(selector: string) {
    return this.querySelectorAll(selector)[0] ?? null
  }
  MiniElement.prototype.closest = function closest(selector: string) {
    let node: MiniNode | null = this
    while (node) {
      if (node instanceof MiniElement && matches(node, selector)) return node
      node = node.parentNode
    }
    return null
  }
  const listen = new Map<string, Array<(event: Event) => void>>()
  const add = dom.window.addEventListener.bind(dom.window)
  const remove = dom.window.removeEventListener.bind(dom.window)
  dom.window.addEventListener = (type: string, fn: (event: Event) => void) => {
    const list = listen.get(type) ?? []
    list.push(fn)
    listen.set(type, list)
    add(type, fn)
  }
  dom.window.removeEventListener = (type: string, fn: (event: Event) => void) => {
    listen.set(type, (listen.get(type) ?? []).filter((entry) => entry !== fn))
    remove(type, fn)
  }
  dom.window.dispatchEvent = (event: Event) => {
    for (const fn of listen.get(event.type) ?? []) fn(event)
    return true
  }
  dom.window.innerWidth = 1280
  dom.window.innerHeight = 800
  dom.window.requestAnimationFrame = (fn: (time: number) => void) => { fn(0); return 1 }
  dom.window.cancelAnimationFrame = () => {}
  globalThis.requestAnimationFrame = dom.window.requestAnimationFrame
  globalThis.cancelAnimationFrame = dom.window.cancelAnimationFrame
  React = await import('react')
  client = await import('react-dom/client')
  Section = (await import('../src/v2/sections/trash')).default
  ToastHost = (await import('../src/v2/chrome/Toast')).ToastHost
  createDesktopSource = (await import('../src/v2/source/desktop')).createDesktopSource
})
afterAll(() => dom.restore())

function matches(el: MiniElement, selector: string): boolean {
  return selector.split(',').some((part) => {
    const body = part.trim()
    if (body.startsWith('[') && body.endsWith(']')) {
      const raw = body.slice(1, -1)
      const eq = raw.indexOf('=')
      if (eq < 0) return el.hasAttribute(raw)
      return el.getAttribute(raw.slice(0, eq)) === raw.slice(eq + 1).replace(/['"]/g, '')
    }
    if (/[: >]/.test(body)) return false
    return el.tagName === body.toUpperCase()
  })
}

function entry(id: string, name: string, originalPath: string, size: number, trashedAt: string): TrashEntry {
  return {
    item: {
      id, parentId: null, name, isFolder: false, size, modifiedAt: trashedAt,
      shared: { kind: 'none' }, local: null, hasThumbnail: false,
    },
    trashedAt, originalPath,
  }
}

const quarterly = entry('file-a', QUARTERLY, 'Documents', 1500, '2026-11-08T09:00:00')
const scan = entry('file-b', SCAN, 'Photos', 2500, '2026-11-07T09:00:00')

interface Script {
  listing: TrashEntry[]
  restore: 'fail' | 'hold'
  /** `ask` rejects until a password is sent. `allow` deletes with no password. */
  deleteMode: 'ask' | 'allow'
}

function installInvoke(script: Script) {
  const calls: { name: string; args?: Record<string, unknown> }[] = []
  let restorePending: ((value: unknown) => void) | null = null
  let deletePending: (() => void) | null = null
  dom.window.__TAURI_INTERNALS__ = {
    transformCallback() { return 1 },
    invoke(name: string, args?: Record<string, unknown>) {
      calls.push({ name, args })
      if (name === 'drive_trash') return Promise.resolve(script.listing.map((row) => ({ ...row, item: { ...row.item } })))
      if (name === 'account_profile') return Promise.resolve({ totp_enabled: true, user_id: 'u', email: 'a@b.c' })
      if (name === 'plugin:event|listen' || name === 'plugin:event|unlisten') return Promise.resolve(1)
      if (name === 'drive_restore_from_trash') {
        if (script.restore === 'fail') return Promise.reject(new Error('not-found: drive'))
        return new Promise((resolve) => { restorePending = resolve })
      }
      if (name === 'drive_delete_permanently') {
        const password = typeof args?.password === 'string' ? args.password : ''
        if (script.deleteMode === 'allow' && password.length === 0) {
          return new Promise((resolve) => {
            deletePending = () => {
              const ids = Array.isArray(args?.ids) ? args.ids.map(String) : []
              script.listing = script.listing.filter((row) => !ids.includes(row.item.id))
              resolve(null)
            }
          })
        }
        return Promise.reject(new Error('invalid_credentials'))
      }
      return Promise.reject(new Error(`unscripted ${name}`))
    },
  }
  return {
    calls,
    releaseRestore(value: unknown) { const go = restorePending; restorePending = null; go?.(value) },
    releaseDelete() { const go = deletePending; deletePending = null; go?.() },
  }
}

async function settle() {
  for (let i = 0; i < 8; i++) await React.act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)) })
}

function fire(container: MiniNode, target: MiniElement, type: string, extra: Record<string, unknown> = {}) {
  const event = {
    type, target, srcElement: target, currentTarget: container,
    bubbles: true, cancelable: true, defaultPrevented: false, isTrusted: true,
    button: type === 'contextmenu' ? 2 : 0, clientX: 24, clientY: 24,
    ctrlKey: false, metaKey: false, shiftKey: false, key: '', timeStamp: Date.now(),
    preventDefault() { this.defaultPrevented = true },
    stopPropagation() {},
    ...extra,
  }
  for (const fn of container.listenersFor(type, true)) fn(event)
  for (const fn of container.listenersFor(type, false)) fn(event)
}

async function press(container: MiniNode, target: MiniElement, type: string, extra: Record<string, unknown> = {}) {
  await React.act(async () => { fire(container, target, type, extra) })
  await settle()
}

function byText(container: MiniNode, tag: string, label: string): MiniElement {
  const hits = allElements(container).filter((el) => el.tagName === tag && el.textContent.replace(/\s+/g, ' ').trim() === label)
  if (hits.length !== 1) throw new Error(`expected one ${tag} "${label}", found ${hits.length}`)
  return hits[0]!
}

function row(container: MiniNode, id: string): MiniElement {
  const hits = allElements(container).filter((el) => el.getAttribute('data-row') === id)
  if (hits.length !== 1) throw new Error(`expected one row ${id}, found ${hits.length}`)
  return hits[0]!
}

const goneTo: { section: string }[] = []
const nav = {
  route: { section: 'trash' as const, id: null, sub: null },
  go(next: { section: string }) { goneTo.push(next) },
  back() {},
  forward() {},
}

async function mount(script: Script) {
  const api = installInvoke(script)
  const container = dom.document.createElement('div') as MiniElement
  dom.document.body.appendChild(container)
  const root = client.createRoot(container as never)
  await React.act(async () => {
    root.render(React.createElement(DriveSourceProvider, { source: createDesktopSource() },
      React.createElement(ToastHost, null, React.createElement(Section, { nav }))))
  })
  await settle()
  return {
    container, api,
    async unmount() {
      await React.act(async () => { root.unmount() })
      container.parentNode?.removeChild(container)
    },
  }
}

describe.serial('Trash restore and delete use the trash commands', () => {
  test('an empty trash says nothing is there and links back to Files', async () => {
    goneTo.length = 0
    const view = await mount({ listing: [], restore: 'fail', deleteMode: 'ask' })
    const text = view.container.textContent
    expect(view.api.calls.filter((call) => call.name === 'drive_trash').length).toBeGreaterThan(0)
    expect(text).toContain('Nothing in the trash')
    expect(text).not.toContain('Trash is empty')
    expect(text).not.toContain(QUARTERLY)
    expect(text).not.toContain(SCAN)
    const link = allElements(view.container).find((el) => el.tagName === 'A' && el.textContent.includes('Files'))
    expect(link?.getAttribute('href')).toContain('section=files')
    await press(view.container, link!, 'click')
    expect(goneTo).toEqual([{ section: 'files' }])
    await view.unmount()
  })

  test('a two-item trash lists those rows, and restore reports the command count', async () => {
    const view = await mount({ listing: [quarterly, scan], restore: 'hold', deleteMode: 'ask' })
    const text = () => view.container.textContent
    expect(view.api.calls.map((call) => call.name)).toContain('drive_trash')
    expect(text()).toContain(QUARTERLY)
    expect(text()).toContain(SCAN)
    expect(text()).toContain('2 items')
    expect(text()).toContain('4 KB')
    expect(text()).not.toContain('Nothing in the trash')
    expect(text()).not.toContain('Harbourlight')
    expect(allElements(view.container).filter((el) => el.getAttribute('data-row'))).toHaveLength(2)
    await press(view.container, row(view.container, 'file-a'), 'click')
    await press(view.container, row(view.container, 'file-b'), 'click', { ctrlKey: true })
    expect(row(view.container, 'file-a').getAttribute('aria-selected')).toBe('true')
    expect(row(view.container, 'file-b').getAttribute('aria-selected')).toBe('true')
    await press(view.container, row(view.container, 'file-b'), 'contextmenu')
    await press(view.container, byText(view.container, 'BUTTON', 'Restore'), 'click')
    const sent = view.api.calls.filter((call) => call.name === 'drive_restore_from_trash')
    expect(sent).toHaveLength(1)
    expect([...(sent[0]!.args?.ids as string[])].sort()).toEqual(['file-a', 'file-b'])
    expect(JSON.stringify(sent)).not.toContain(QUARTERLY)
    expect(text()).toContain(QUARTERLY)
    expect(text()).toContain(SCAN)
    expect(text()).not.toContain('1 item restored')
    await React.act(async () => { view.api.releaseRestore({ count: 1 }) })
    await settle()
    expect(text()).toContain('1 item restored')
    expect(text()).not.toContain('2 items restored')
    expect(text()).not.toContain('Restored to Documents')
    expect(text()).toContain(QUARTERLY)
    expect(text()).toContain(SCAN)
    expect(allElements(view.container).filter((el) => el.getAttribute('data-row'))).toHaveLength(2)
    await view.unmount()
  })

  test('a failed restore keeps the item and says why', async () => {
    const lines: string[] = []
    const methods = ['log', 'info', 'warn', 'error', 'debug'] as const
    const previous = methods.map((method) => console[method])
    for (const method of methods) {
      console[method] = (...args: unknown[]) => { lines.push(args.map((arg) => String(arg)).join(' ')) }
    }
    const view = await mount({ listing: [quarterly], restore: 'fail', deleteMode: 'ask' })
    try {
      expect(view.container.textContent).toContain(QUARTERLY)
      await press(view.container, row(view.container, 'file-a'), 'contextmenu')
      await press(view.container, byText(view.container, 'BUTTON', 'Restore'), 'click')
      expect(view.container.textContent).toContain(QUARTERLY)
      expect(view.container.textContent).toContain('It isn’t in the trash')
      expect(view.container.textContent).not.toContain('not-found')
      expect(view.container.textContent).not.toContain('1 item restored')
      expect(allElements(view.container).filter((el) => el.getAttribute('data-row'))).toHaveLength(1)
      const sent = view.api.calls.filter((call) => call.name === 'drive_restore_from_trash')
      expect(JSON.stringify(sent)).not.toContain(QUARTERLY)
      expect(lines.join('\n')).not.toContain(QUARTERLY)
    } finally {
      methods.forEach((method, i) => { console[method] = previous[i]! })
      await view.unmount()
    }
  })

  test('delete for good asks for the password only when the command requires it', async () => {
    const script: Script = { listing: [quarterly, scan], restore: 'fail', deleteMode: 'ask' }
    const view = await mount(script)
    const text = () => view.container.textContent
    expect(text()).not.toContain('Password')
    await press(view.container, row(view.container, 'file-a'), 'contextmenu')
    await press(view.container, byText(view.container, 'BUTTON', 'Delete permanently…'), 'click')
    const asked = view.api.calls.filter((call) => call.name === 'drive_delete_permanently')
    expect(asked).toHaveLength(1)
    expect(asked[0]!.args?.password ?? null).toBeNull()
    expect(text()).toContain(QUARTERLY)
    const password = allElements(view.container).find((el) => el.tagName === 'INPUT' && el.getAttribute('aria-label') === 'Password')
    expect(password).toBeTruthy()
    expect(text()).not.toContain('Authenticator code')
    await React.act(async () => { typeInto(view.container, password!, 'wrong-password') })
    await settle()
    await press(view.container, byText(view.container, 'BUTTON', 'Delete permanently'), 'click')
    const tried = view.api.calls.filter((call) => call.name === 'drive_delete_permanently')
    expect(tried).toHaveLength(2)
    expect(tried[1]!.args?.password).toBe('wrong-password')
    expect(text()).toContain('Incorrect password.')
    expect(text()).not.toContain('invalid_credentials')
    expect(text()).toContain(QUARTERLY)
    expect(text()).toContain(SCAN)
    expect(text()).not.toContain('Deleted for good')
    await press(view.container, byText(view.container, 'BUTTON', 'Cancel'), 'click')
    expect(allElements(view.container).some((el) => el.getAttribute('aria-label') === 'Password')).toBe(false)
    script.deleteMode = 'allow'
    await press(view.container, row(view.container, 'file-b'), 'contextmenu')
    await press(view.container, byText(view.container, 'BUTTON', 'Delete permanently…'), 'click')
    expect(allElements(view.container).some((el) => el.getAttribute('aria-label') === 'Password')).toBe(false)
    expect(text()).toContain(SCAN)
    expect(text()).not.toContain('Deleted for good')
    await React.act(async () => { view.api.releaseDelete() })
    await settle()
    expect(text()).toContain('Deleted for good')
    expect(text()).toContain(QUARTERLY)
    expect(text()).not.toContain(SCAN)
    expect(allElements(view.container).filter((el) => el.getAttribute('data-row')).map((el) => el.getAttribute('data-row'))).toEqual(['file-a'])
    await view.unmount()
  })
})

/** Set the input the way React's value tracker notices, then the input event. */
function typeInto(container: MiniNode, input: MiniElement, value: string) {
  const descriptor = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(input), 'value')
  descriptor?.set?.call(input, value)
  fire(container, input, 'input')
  fire(container, input, 'change')
}
