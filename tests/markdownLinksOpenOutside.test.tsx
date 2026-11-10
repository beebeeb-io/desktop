// A markdown note's links leave the window the way every other outside link does: an http(s) address goes to the
// OS through openExternal (never a new web view), a beebeeb://file/ address opens that file in the window, and
// any other scheme is not opened at all. A refused open says so in the note.
import { afterAll, beforeAll, expect, mock, test } from 'bun:test'
import { allElements, installMiniDom, MiniElement, type MiniDomInstall, type MiniNode } from './fixtures/miniDom'

const externalPath = new URL('../src/v2/external.ts', import.meta.url).pathname
const opened: string[] = []
let answer: { ok: boolean; reason?: string; unsupported?: boolean } = { ok: true }

mock.module(externalPath, () => ({
  openExternal: async (url: string) => {
    opened.push(url)
    return answer
  },
  isExternal: (href: string) => /^(https?:|mailto:|ms-settings:)/i.test(href),
}))

let dom: MiniDomInstall
let React: typeof import('react')
let client: typeof import('react-dom/client')
let Viewers: typeof import('../src/v2/sections/open/Viewers')

beforeAll(async () => {
  dom = installMiniDom('')
  React = await import('react')
  client = await import('react-dom/client')
  Viewers = await import('../src/v2/sections/open/Viewers')
})
afterAll(async () => {
  await settle()
  dom.restore()
})

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

/** A click as the WebView delivers it, returning the event so preventDefault can be read. */
function clickEvent(container: MiniNode, target: MiniElement) {
  const event = {
    type: 'click', target, srcElement: target, currentTarget: container,
    bubbles: true, cancelable: true, defaultPrevented: false, isTrusted: true,
    button: 0, timeStamp: Date.now(),
    preventDefault() { this.defaultPrevented = true },
    stopPropagation() {},
  }
  for (const fn of container.listenersFor('click', true)) fn(event)
  for (const fn of container.listenersFor('click', false)) fn(event)
  return event
}

test('an https link opens in the OS and never navigates the window', async () => {
  opened.length = 0
  answer = { ok: true }
  const { container, root, show } = mount(<Viewers.MarkdownViewer text="[the site](https://beebeeb.io/notes)" mode="rendered" />)
  await show(<Viewers.MarkdownViewer text="[the site](https://beebeeb.io/notes)" mode="rendered" />)
  const link = byText(container, 'A', 'the site')
  expect(link.getAttribute('href')).toBe('https://beebeeb.io/notes')
  const event = await React.act(async () => clickEvent(container, link))
  await settle()
  expect(event.defaultPrevented).toBe(true)
  expect(opened).toEqual(['https://beebeeb.io/notes'])
  expect(dom.window.location.href).toBe('http://localhost/')
  await React.act(async () => { root.unmount() })
})

test('a beebeeb://file/ link opens that file in the window and never the OS', async () => {
  opened.length = 0
  answer = { ok: true }
  const seen: string[] = []
  const { container, root, show } = mount(<Viewers.MarkdownViewer text="[Budget](beebeeb://file/Budget.xlsx)" mode="rendered" onOpenFile={(name) => seen.push(name)} />)
  await show(<Viewers.MarkdownViewer text="[Budget](beebeeb://file/Budget.xlsx)" mode="rendered" onOpenFile={(name) => seen.push(name)} />)
  const link = byText(container, 'A', 'Budget')
  const event = await React.act(async () => clickEvent(container, link))
  await settle()
  expect(event.defaultPrevented).toBe(true)
  expect(seen).toEqual(['Budget.xlsx'])
  expect(opened).toEqual([])
  await React.act(async () => { root.unmount() })
})

test('a refused open says so, and another scheme is not opened', async () => {
  opened.length = 0
  answer = { ok: false, reason: 'failed' }
  const { container, root, show } = mount(<Viewers.MarkdownViewer text={'[the site](https://beebeeb.io/notes)\n\n[mail](mailto:hello@beebeeb.io)'} mode="rendered" />)
  await show(<Viewers.MarkdownViewer text={'[the site](https://beebeeb.io/notes)\n\n[mail](mailto:hello@beebeeb.io)'} mode="rendered" />)
  await React.act(async () => { clickEvent(container, byText(container, 'A', 'the site')) })
  await settle()
  const alert = allElements(container).find((el) => el.getAttribute('role') === 'alert')
  expect(alert?.textContent).toBe('Couldn’t open the link.')
  expect(allElements(container).filter((el) => el.tagName === 'A' && el.getAttribute('href') === 'mailto:hello@beebeeb.io')).toHaveLength(0)
  expect(opened).toEqual(['https://beebeeb.io/notes'])
  await React.act(async () => { root.unmount() })
})
