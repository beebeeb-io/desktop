// File actions do what they say: rename, move, share and copy link run the real source command, keep the
// dialog and the person's input on a refusal, and the row menu offers only what the item supports.
import { afterAll, beforeAll, expect, test } from 'bun:test'
import { DriveSourceProvider, DriveSourceError, type DriveItem, type DriveSource, type Listing } from '@beebeeb/shared/drive-ui'
import { allElements, click, installMiniDom, MiniElement, type MiniDomInstall, type MiniNode } from './fixtures/miniDom'

let dom: MiniDomInstall
let React: typeof import('react')
let client: typeof import('react-dom/client')
let Dialogs: typeof import('../src/v2/sections/files/Dialogs')
let shareLink: typeof import('../src/v2/sections/files/shareLink')
let menus: typeof import('../src/v2/sections/files/menus')

beforeAll(async () => {
  dom = installMiniDom('')
  // React only treats a field as a text input when the document supports `input` and the element has a type.
  Object.defineProperty(dom.document, 'oninput', { value: null, configurable: true })
  Object.defineProperty(MiniElement.prototype, 'type', {
    configurable: true,
    get() { return this.getAttribute('type') ?? (this.tagName === 'INPUT' ? 'text' : '') },
    set(value: string) { this.setAttribute('type', String(value)) },
  })
  React = await import('react')
  client = await import('react-dom/client')
  const proto = dom.window.HTMLElement.prototype as { querySelector: () => null; querySelectorAll: () => [] }
  proto.querySelector = () => null
  proto.querySelectorAll = () => []
  if (typeof globalThis.requestAnimationFrame !== 'function') {
    globalThis.requestAnimationFrame = (fn: FrameRequestCallback) => setTimeout(() => fn(Date.now()), 0) as unknown as number
    globalThis.cancelAnimationFrame = (id: number) => clearTimeout(id as unknown as ReturnType<typeof setTimeout>)
  }
  // The toast host listens on the window; the mini DOM has no event bus of its own.
  dom.window.dispatchEvent = () => true
  Dialogs = await import('../src/v2/sections/files/Dialogs')
  shareLink = await import('../src/v2/sections/files/shareLink')
  menus = await import('../src/v2/sections/files/menus')
})
afterAll(() => dom.restore())

function clearBody() {
  for (const child of [...dom.document.body.childNodes]) dom.document.body.removeChild(child)
}

async function settle() {
  for (let i = 0; i < 10; i++) await React.act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)) })
}

function mount(node: import('react').ReactElement) {
  const container = dom.document.createElement('div') as MiniElement
  dom.document.body.appendChild(container)
  const root = client.createRoot(container as never)
  return {
    container,
    root,
    show: async (next: import('react').ReactElement) => { await React.act(async () => { root.render(next) }); await settle() },
  }
}

function byText(container: MiniNode, tag: string, text: string): MiniElement {
  const hits = allElements(container).filter((el) => el.tagName === tag && (el.textContent ?? '').replace(/\s+/g, ' ').trim() === text)
  if (hits.length !== 1) throw new Error(`expected one ${tag} "${text}", found ${hits.length}`)
  return hits[0]!
}

function byLabel(container: MiniNode, label: string): MiniElement {
  const hits = allElements(container).filter((el) => el.getAttribute('aria-label') === label)
  if (hits.length !== 1) throw new Error(`expected one control "${label}", found ${hits.length}`)
  return hits[0]!
}

function fire(container: MiniNode, target: MiniElement, type: string) {
  const event = {
    type, target, srcElement: target, currentTarget: container,
    bubbles: true, cancelable: true, defaultPrevented: false, isTrusted: true,
    button: 0, clientX: 1, clientY: 1, key: 'Enter', timeStamp: Date.now(),
    preventDefault() { this.defaultPrevented = true },
    stopPropagation() {},
  }
  for (const fn of container.listenersFor(type, true)) fn(event)
  for (const fn of container.listenersFor(type, false)) fn(event)
}

/** Set the field the way React's value tracker notices, then the event React listens for. */
function setValue(input: MiniElement, value: string) {
  const descriptor = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(input), 'value')
  descriptor?.set?.call(input, value)
}

async function type(container: MiniNode, input: MiniElement, value: string) {
  await React.act(async () => {
    setValue(input, value)
    fire(container, input, 'input')
    fire(container, input, 'change')
  })
}

async function choose(container: MiniNode, select: MiniElement, value: string) {
  await React.act(async () => {
    setValue(select, value)
    fire(container, select, 'change')
  })
}

function item(over: Partial<DriveItem> & Pick<DriveItem, 'id' | 'name'>): DriveItem {
  return {
    parentId: null, isFolder: false, size: 1000, modifiedAt: '2026-11-01T10:00:00',
    shared: { kind: 'none' }, local: 'downloaded', hasThumbnail: false, ...over,
  }
}

const FOLDER = item({ id: 'd1', name: 'Documents', isFolder: true, size: null, childCount: 2 })
const LISTING: Listing = {
  folder: { id: null, name: 'Files' },
  path: [{ id: null, name: 'Files' }],
  items: [FOLDER],
  asOf: '2026-11-01T10:00:00',
  stale: false,
}

function sourceOf(over: Partial<DriveSource>): DriveSource {
  return {
    kind: 'desktop',
    list: async () => LISTING,
    rename: async () => {},
    move: async () => {},
    ...over,
  } as unknown as DriveSource
}

test('rename runs the source and keeps the name on a refusal', async () => {
  clearBody()
  const calls: { id: string; name: string }[] = []
  const done: boolean[] = []
  const file = item({ id: 'f1', name: 'Contract draft.pdf' })
  const source = sourceOf({ rename: async (id, name) => { calls.push({ id, name }) } })
  const view = mount(
    <DriveSourceProvider source={source}>
      <Dialogs.RenameDialog item={file} siblings={[]} onDone={(renamed) => done.push(renamed)} />
    </DriveSourceProvider>,
  )
  await view.show(
    <DriveSourceProvider source={source}>
      <Dialogs.RenameDialog item={file} siblings={[]} onDone={(renamed) => done.push(renamed)} />
    </DriveSourceProvider>,
  )
  await type(view.container, byLabel(view.container, 'Name'), 'Contract final.pdf')
  await React.act(async () => { click(view.container, byText(view.container, 'BUTTON', 'Rename')) })
  await settle()
  expect(calls).toEqual([{ id: 'f1', name: 'Contract final.pdf' }])
  expect(done).toEqual([true])
  const title = allElements(view.container).find((el) => el.getAttribute('title') === 'Contract draft.pdf')
  expect(title?.className).toContain('truncate')
  await React.act(async () => view.root.unmount())

  clearBody()
  const refused: boolean[] = []
  const taken = sourceOf({ rename: async () => { throw new DriveSourceError('name-taken', 'drive_rename') } })
  const copy = item({ id: 'f2', name: 'Contract draft copy.pdf' })
  const second = mount(
    <DriveSourceProvider source={taken}>
      <Dialogs.RenameDialog item={copy} siblings={[]} onDone={(renamed) => refused.push(renamed)} />
    </DriveSourceProvider>,
  )
  await second.show(
    <DriveSourceProvider source={taken}>
      <Dialogs.RenameDialog item={copy} siblings={[]} onDone={(renamed) => refused.push(renamed)} />
    </DriveSourceProvider>,
  )
  await type(second.container, byLabel(second.container, 'Name'), 'Contract draft.pdf')
  await React.act(async () => { click(second.container, byText(second.container, 'BUTTON', 'Rename')) })
  await settle()
  expect(refused).toEqual([])
  expect(byLabel(second.container, 'Name').value).toBe('Contract draft.pdf')
  expect(second.container.textContent).toContain('There’s already a file called Contract draft.pdf here. Choose another name.')
  await React.act(async () => second.root.unmount())
})

test('move runs the source and keeps the chosen folder on a refusal', async () => {
  clearBody()
  const calls: { ids: string[]; target: string | null }[] = []
  const done: boolean[] = []
  const file = item({ id: 'f1', name: 'Contract draft.pdf' })
  const source = sourceOf({ move: async (ids, target) => { calls.push({ ids, target }) } })
  const view = mount(
    <DriveSourceProvider source={source}>
      <Dialogs.MoveToDialog items={[file]} onDone={(moved) => done.push(moved)} />
    </DriveSourceProvider>,
  )
  await view.show(
    <DriveSourceProvider source={source}>
      <Dialogs.MoveToDialog items={[file]} onDone={(moved) => done.push(moved)} />
    </DriveSourceProvider>,
  )
  const row = allElements(view.container).find((el) => el.getAttribute('data-move-folder') === 'd1')
  if (!row) throw new Error('the folder row is missing')
  await React.act(async () => { click(view.container, row) })
  await React.act(async () => { click(view.container, byText(view.container, 'BUTTON', 'Move to Documents')) })
  await settle()
  expect(calls).toEqual([{ ids: ['f1'], target: 'd1' }])
  expect(done).toEqual([true])
  await React.act(async () => view.root.unmount())

  clearBody()
  const refused: boolean[] = []
  const offline = sourceOf({ move: async () => { throw new DriveSourceError('offline', 'drive_move') } })
  const second = mount(
    <DriveSourceProvider source={offline}>
      <Dialogs.MoveToDialog items={[file]} onDone={(moved) => refused.push(moved)} />
    </DriveSourceProvider>,
  )
  await second.show(
    <DriveSourceProvider source={offline}>
      <Dialogs.MoveToDialog items={[file]} onDone={(moved) => refused.push(moved)} />
    </DriveSourceProvider>,
  )
  const again = allElements(second.container).find((el) => el.getAttribute('data-move-folder') === 'd1')
  if (!again) throw new Error('the folder row is missing')
  await React.act(async () => { click(second.container, again) })
  await React.act(async () => { click(second.container, byText(second.container, 'BUTTON', 'Move to Documents')) })
  await settle()
  expect(refused).toEqual([])
  expect(second.container.textContent).toContain('Couldn’t move it. Check your connection.')
  expect(again.getAttribute('aria-selected')).toBe('true')
  await React.act(async () => second.root.unmount())
})

test('share runs the link command with the dialog’s expiry and password, and keeps them on a refusal', async () => {
  clearBody()
  const calls: { cmd: string; args?: Record<string, unknown> }[] = []
  let answer: () => unknown = () => ({ display: 'https://share.example/p/k9m2' })
  dom.window.__TAURI_INTERNALS__ = {
    transformCallback() { return 1 },
    invoke(cmd: string, args?: Record<string, unknown>) {
      calls.push({ cmd, args })
      return Promise.resolve().then(answer)
    },
  }
  const closed: number[] = []
  const view = mount(
    <Dialogs.ShareDialog name="Contract draft.pdf" size={1_200_000} ids={['f1']} onClose={() => closed.push(1)} />,
  )
  await view.show(
    <Dialogs.ShareDialog name="Contract draft.pdf" size={1_200_000} ids={['f1']} onClose={() => closed.push(1)} />,
  )
  await choose(view.container, byLabel(view.container, 'Link expires'), 'In 7 days')
  await React.act(async () => { click(view.container, byLabel(view.container, 'Password')) })
  await type(view.container, byLabel(view.container, 'Link password'), 'hunter2')
  await React.act(async () => { click(view.container, byText(view.container, 'BUTTON', 'Copy link')) })
  await settle()
  expect(calls).toEqual([{ cmd: 'share_link_copy', args: { itemId: 'f1', itemIds: null, expiresHours: 168, password: 'hunter2' } }])
  expect(closed).toEqual([1])
  await React.act(async () => view.root.unmount())

  clearBody()
  answer = () => { throw new Error('offline') }
  const refused: number[] = []
  const second = mount(
    <Dialogs.ShareDialog name="Contract draft.pdf" size={1_200_000} ids={['f1']} onClose={() => refused.push(1)} />,
  )
  await second.show(
    <Dialogs.ShareDialog name="Contract draft.pdf" size={1_200_000} ids={['f1']} onClose={() => refused.push(1)} />,
  )
  await choose(second.container, byLabel(second.container, 'Link expires'), 'In 7 days')
  await React.act(async () => { click(second.container, byLabel(second.container, 'Password')) })
  await type(second.container, byLabel(second.container, 'Link password'), 'hunter2')
  await React.act(async () => { click(second.container, byText(second.container, 'BUTTON', 'Copy link')) })
  await settle()
  expect(refused).toEqual([])
  expect(second.container.textContent).toContain('You’re offline. Try again when you’re connected.')
  expect(byLabel(second.container, 'Link expires').value).toBe('In 7 days')
  expect(byLabel(second.container, 'Link password').value).toBe('hunter2')
  await React.act(async () => second.root.unmount())
})

test('copy link says Link copied only when the clipboard took it, and says what to do when it did not', async () => {
  clearBody()
  const calls: { cmd: string; args?: Record<string, unknown> }[] = []
  let answer: () => unknown = () => ({ display: 'https://share.example/p/k9m2' })
  dom.window.__TAURI_INTERNALS__ = {
    transformCallback() { return 1 },
    invoke(cmd: string, args?: Record<string, unknown>) {
      calls.push({ cmd, args })
      return Promise.resolve().then(answer)
    },
  }
  const toasts: { kind: string; text: string }[] = []
  const ok = await shareLink.copyShareLink(['f1'], { expires: 'In 30 days', password: null }, (message) => toasts.push(message))
  expect(ok.ok).toBe(true)
  expect(ok.text).toContain('Link copied')
  expect(toasts).toEqual([{ kind: 'done', text: ok.text }])
  expect(calls[0]?.args).toEqual({ itemId: 'f1', itemIds: null, expiresHours: 720, password: null })

  answer = () => { throw new Error('clipboard') }
  const failed = await shareLink.copyShareLink(['f1'], { expires: 'In 30 days', password: null }, (message) => toasts.push(message))
  expect(failed.ok).toBe(false)
  expect(failed.text).toContain('clipboard')
  expect(failed.text).toContain('try again')
  expect(failed.text).not.toContain('Link copied')
  expect(toasts[toasts.length - 1]).toEqual({ kind: 'failed', text: failed.text })
})

test('the row menu offers only what the item supports', () => {
  const file = item({ id: 'f1', name: 'Contract draft.pdf' })
  const folder = item({ id: 'd1', name: 'Documents', isFolder: true, size: null })
  const readOnly = item({ id: 'f2', name: 'Shared.pdf', readOnly: true })
  const ids = (entries: { id: string }[]) => entries.map((entry) => entry.id)
  expect(ids(menus.fileMenu([file]))).toContain('rename')
  expect(ids(menus.fileMenu([file]))).toContain('copy-link')
  expect(ids(menus.fileMenu([file], { sharedWithMe: true }))).not.toContain('rename')
  expect(ids(menus.fileMenu([folder]))).toContain('copy-link')
  expect(ids(menus.fileMenu([folder], { linkShare: false }))).not.toContain('copy-link')
  expect(ids(menus.fileMenu([readOnly]))).not.toContain('rename')
})
