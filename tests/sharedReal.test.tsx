// Shared lists what was actually shared. The rows come from each item's shared state, an empty
// account says so, and a link share comes from the share command or is said to be unlistable.
import { afterAll, beforeAll, expect, test } from 'bun:test'
import { DriveSourceProvider, type DriveItem, type DriveSource, type ShareEntry, type SharedState } from '@beebeeb/shared/drive-ui'
import type { SectionProps } from '../src/v2/sections/types'
import { allElements, click, installMiniDom, type MiniDomInstall, type MiniElement, type MiniNode } from './fixtures/miniDom'

const DECOY = ['Anna', 'Lotte de Wit', 'Priya Shah']
const EMPTY_ONE = 'When someone shares a file or folder with you, it shows here with their name and the date.'
const EMPTY_TWO = 'What you share with people, or by a link, shows here as well.'
const UNLISTED = 'Link shares can’t be listed yet.'

let dom: MiniDomInstall
let React: typeof import('react')
let client: typeof import('react-dom/client')
let Section: (props: SectionProps) => import('react').ReactElement

beforeAll(async () => {
  dom = installMiniDom('')
  React = await import('react')
  client = await import('react-dom/client')
  Section = (await import('../src/v2/sections/shared')).default
})
afterAll(() => dom.restore())

function clearBody() {
  for (const child of [...dom.document.body.childNodes]) dom.document.body.removeChild(child)
}

function item(id: string, name: string, modifiedAt: string, shared: SharedState, extra: Partial<DriveItem> = {}): DriveItem {
  return {
    id, parentId: null, name, isFolder: false, size: 1000, modifiedAt, shared,
    local: 'online-only', hasThumbnail: false, ...extra,
  }
}

function entry(row: DriveItem, direction: ShareEntry['direction'], decoy: string | null, hasLink = false): ShareEntry {
  return {
    item: row,
    direction,
    people: decoy ? [{ name: decoy, role: 'editor' }] : [],
    hasLink,
  }
}

const STUDIO = item('studio', 'Studio assets', '2026-10-29T14:10:00', { kind: 'people', people: [{ name: 'Mara Jansen', initials: 'MJ' }] }, { isFolder: true, size: null, childCount: 4 })
const RATES = item('rates', 'Rates 2026.xlsx', '2026-09-02T08:00:00', { kind: 'none' })
const LISBON = item('lisbon', 'Lisbon trip', '2026-10-12T19:30:00', { kind: 'people', people: [{ name: 'Sam Okafor', initials: 'SO' }] }, { isFolder: true, size: null, childCount: 12 })
const MIRROR = item('mirror-link', 'Mirror only.pdf', '2026-08-01T10:00:00', { kind: 'link' })

const WITH_YOU = [
  entry(STUDIO, 'with-me', 'Anna'),
  entry(RATES, 'with-me', 'Lotte de Wit'),
]
const BY_YOU = [
  entry(LISBON, 'by-me', 'Priya Shah'),
  entry(MIRROR, 'by-me', null, true),
]

const EMPTY_LISTS = { invitations: [], links: [], requests: [], pending: [] }
const LISTED_LINKS = {
  ...EMPTY_LISTS,
  links: [{ id: 'budget', name: 'Budget 2026.xlsx', expiresAt: '2026-12-01T12:00:00', password: false }],
}

function host(answer: () => unknown) {
  dom.window.__TAURI_INTERNALS__ = {
    transformCallback() { return 1 },
    invoke(cmd: string) {
      if (cmd === 'share_list') return Promise.resolve().then(answer)
      return Promise.reject(new Error(`unknown command ${cmd}`))
    },
  }
}

async function settle() {
  for (let i = 0; i < 8; i++) await React.act(async () => { await new Promise((r) => setTimeout(r, 0)) })
}

function row(id: string): MiniElement {
  const found = allElements(dom.document.body).filter((el) => el.getAttribute('data-row') === id)
  if (found.length !== 1) throw new Error(`${found.length} rows for ${id}`)
  return found[0]!
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

async function show(entries: ShareEntry[], answer: () => unknown) {
  clearBody()
  host(answer)
  const opened: { section: string; id?: string | null }[] = []
  const files: string[] = []
  const source = {
    kind: 'desktop',
    shared: async () => entries,
    subscribe: () => () => {},
    open: async (id: string) => { files.push(id) },
  } as unknown as DriveSource
  const nav: SectionProps['nav'] = {
    route: { section: 'shared', id: null, sub: null },
    go: (route) => { opened.push({ section: route.section, id: route.id }) },
    back: () => {},
    forward: () => {},
  }
  const container = dom.document.createElement('div')
  dom.document.body.appendChild(container)
  const root = client.createRoot(container as never)
  await React.act(async () => { root.render(React.createElement(DriveSourceProvider, { source }, React.createElement(Section, { nav }))) })
  await settle()
  return {
    container: container as MiniElement,
    text: () => dom.document.body.textContent ?? '',
    opened,
    files,
    async unmount() {
      await React.act(async () => root.unmount())
      container.parentNode?.removeChild(container)
    },
  }
}

test('empty', async () => {
  const view = await show([], () => EMPTY_LISTS)
  const text = view.text()
  expect(text).toContain('Nothing shared yet')
  expect(text).toContain(EMPTY_ONE)
  expect(text).toContain(EMPTY_TWO)
  expect(text).not.toContain('No longer shared')
  expect(text).not.toContain(UNLISTED)
  expect(allElements(dom.document.body).some((el) => el.getAttribute('data-row'))).toBe(false)
  for (const name of DECOY) expect(text).not.toContain(name)
  await view.unmount()
})

test('shared-with-you', async () => {
  const view = await show(WITH_YOU, () => EMPTY_LISTS)
  const text = view.text()
  expect(text).toContain('Shared with you')
  expect(text).not.toContain('Nothing shared yet')
  const studio = row('studio')
  expect(studio.textContent).toContain('Studio assets')
  expect(studio.textContent).toContain('Mara Jansen')
  expect(studio.textContent).toContain('29 Oct 2026')
  expect(studio.textContent).not.toContain('Anna')
  expect(studio.getAttribute('data-open')).toBe('yes')
  const open = allElements(studio).filter((el) => el.tagName === 'BUTTON' && (el.textContent ?? '').trim() === 'Open')
  expect(open).toHaveLength(1)
  await React.act(async () => { click(view.container, open[0]!) })
  expect(view.opened).toEqual([{ section: 'files', id: 'studio' }])

  const rates = row('rates')
  expect(rates.textContent).toContain('Rates 2026.xlsx')
  expect(rates.textContent).toContain('No longer shared')
  expect(rates.textContent).not.toContain('Lotte de Wit')
  expect(rates.getAttribute('data-open')).toBeNull()
  expect(allElements(rates).some((el) => el.tagName === 'BUTTON')).toBe(false)
  await React.act(async () => { fire(view.container, rates, 'dblclick'); fire(view.container, rates, 'click') })
  expect(view.files).toEqual([])
  expect(view.opened).toEqual([{ section: 'files', id: 'studio' }])
  for (const name of DECOY) expect(text).not.toContain(name)
  await view.unmount()
})

test('shared-by-you', async () => {
  const listed = await show(BY_YOU, () => LISTED_LINKS)
  const text = listed.text()
  expect(text).toContain('Shared by you')
  const lisbon = row('lisbon')
  expect(lisbon.textContent).toContain('Lisbon trip')
  expect(lisbon.textContent).toContain('Sam Okafor')
  expect(lisbon.textContent).toContain('12 Oct 2026')
  expect(lisbon.textContent).not.toContain('Priya Shah')
  expect(text).toContain('Budget 2026.xlsx')
  expect(text).not.toContain('Mirror only.pdf')
  expect(text).not.toContain(UNLISTED)
  expect(text).not.toContain('Nothing shared yet')
  for (const name of DECOY) expect(text).not.toContain(name)
  await listed.unmount()

  const unlisted = await show([entry(MIRROR, 'by-me', null, true)], () => { throw new Error('unknown command share_list') })
  const again = unlisted.text()
  expect(again).toContain('Shared by you')
  expect(again).toContain(UNLISTED)
  expect(again).not.toContain('Mirror only.pdf')
  expect(again).not.toContain('Nothing shared yet')
  expect(again).not.toContain('Budget 2026.xlsx')
  await unlisted.unmount()
})
