// Search shows what the drive source's search returned. Nothing else is added, a miss says so and keeps the
// query, the index-building line follows the list check until it finishes, and a failed search leaves the rows
// already on screen.
import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import React, { createElement, type ReactElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { DriveSourceError, DriveSourceProvider, type DriveEvent, type DriveItem } from '@beebeeb/shared/drive-ui'
import { actionFailure } from '../src/v2/sections/files/model'
import Section, { followListing, searchMenu, type ListingProgress } from '../src/v2/sections/search'
import { byPill, Highlight, type SearchHit } from '../src/v2/sections/search/SearchView'
import { allElements, click, installMiniDom, MiniElement, type MiniDomInstall, type MiniNode } from './fixtures/miniDom'

const QUERY = 'cafe'

function item(over: Partial<DriveItem> & Pick<DriveItem, 'id' | 'name'>): DriveItem {
  return {
    parentId: null,
    isFolder: false,
    size: 1200,
    modifiedAt: '2026-11-01T10:00:00.000Z',
    shared: { kind: 'none' },
    local: 'downloaded',
    hasThumbnail: false,
    ...over,
  }
}

const HITS: DriveItem[] = [
  item({ id: 'folder', name: 'Café Folder', isFolder: true, size: null, childCount: 2 }),
  item({ id: 'photo', name: 'Cafe Shot.png', mime: 'image/png' }),
  item({ id: 'doc', name: 'cafe-plan.pdf', mime: 'application/pdf' }),
  item({ id: 'other', name: 'cafe.dat' }),
  item({ id: 'markup', name: 'cafe <img src=x onerror=alert(1)>.txt' }),
]

function asHits(rows: DriveItem[]): SearchHit[] {
  return rows.map((row) => ({ item: row, where: '' }))
}

function bolds(html: string): string[] {
  return [...html.matchAll(/<b\b[^>]*>([\s\S]*?)<\/b>/g)].map((match) => match[1]!
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&quot;/g, '"')
    .replace(/&#x27;|&#39;|&apos;/g, "'")
    .replace(/&amp;/g, '&'))
}

function highlight(name: string, query: string): string[] {
  return bolds(renderToStaticMarkup(createElement(Highlight, { name, query })))
}

let dom: MiniDomInstall
let client: typeof import('react-dom/client')
let listing: ListingProgress | undefined
const statusReads: ListingProgress[] = []

beforeAll(async () => {
  dom = installMiniDom('')
  Object.defineProperty(dom.document, 'oninput', { value: null, configurable: true })
  Object.defineProperty(MiniElement.prototype, 'type', {
    configurable: true,
    get() { return this.getAttribute('type') ?? (this.tagName === 'INPUT' ? 'text' : '') },
    set(value: string) { this.setAttribute('type', String(value)) },
  })
  const memory = new Map<string, string>()
  Object.defineProperty(globalThis, 'localStorage', {
    configurable: true,
    value: {
      getItem: (key: string) => memory.get(key) ?? null,
      setItem: (key: string, value: string) => { memory.set(key, value) },
      removeItem: (key: string) => { memory.delete(key) },
    },
  })
  dom.window.__TAURI_INTERNALS__ = {
    invoke: async (cmd: string) => {
      if (cmd === 'drive_status') {
        statusReads.push(listing ? { done: listing.done, total: listing.total } : undefined as unknown as ListingProgress)
        return { listing }
      }
      if (cmd === 'drive_paths') return {}
      throw new Error(`${cmd} refused`)
    },
  }
  client = await import('react-dom/client')
})

afterAll(() => dom.restore())

function nav(sub: string) {
  return { route: { section: 'search' as const, id: null, sub }, go() {}, back() {}, forward() {} }
}

async function settle() {
  for (let i = 0; i < 8; i++) await React.act(async () => { await Promise.resolve() })
}

function buttons(root: MiniNode, label: string): MiniElement[] {
  return allElements(root).filter((el) => el.tagName === 'BUTTON' && el.textContent.trim() === label)
}

function field(root: MiniNode): MiniElement {
  const found = allElements(root).filter((el) => el.tagName === 'INPUT' && el.getAttribute('aria-label') === 'Search names')
  if (found.length !== 1) throw new Error(`${found.length} search fields`)
  return found[0]!
}

function rows(root: MiniNode): string[] {
  return allElements(root).filter((el) => el.hasAttribute('data-result')).map((el) => el.getAttribute('data-result') ?? '')
}

function pill(root: MiniNode, name: string): MiniElement {
  const found = allElements(root).filter((el) => el.tagName === 'BUTTON' && (el.getAttribute('data-pill') === name || el.textContent.trim().startsWith(name)))
  if (found.length !== 1) throw new Error(`${found.length} pills named ${name}`)
  return found[0]!
}

/** Set the field the way the value tracker notices, then the input event React listens for. */
function typeQuery(container: MiniNode, input: MiniElement, value: string) {
  const descriptor = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(input), 'value')
  descriptor?.set?.call(input, value)
  const event = {
    type: 'input',
    target: input,
    srcElement: input,
    currentTarget: container,
    bubbles: true,
    cancelable: false,
    defaultPrevented: false,
    isTrusted: true,
    preventDefault() { this.defaultPrevented = true },
    stopPropagation() {},
  }
  for (const fn of container.listenersFor('input', true)) fn(event)
  for (const fn of container.listenersFor('input', false)) fn(event)
}

async function show(node: ReactElement) {
  const container = dom.document.createElement('div')
  dom.document.body.appendChild(container)
  const root = client.createRoot(container as never)
  await React.act(async () => { root.render(node) })
  await settle()
  return {
    container: container as MiniElement,
    text: () => container.textContent ?? '',
    async unmount() {
      await React.act(async () => root.unmount())
      container.parentNode?.removeChild(container)
    },
  }
}

describe('Search finds what is there', () => {
  test('hits are the drive source search, highlighted with its fold, and pill counts match the rows', async () => {
    expect(typeof byPill).toBe('function')
    expect(typeof Highlight).toBe('function')
    expect(typeof followListing).toBe('function')
    expect(typeof searchMenu).toBe('function')
    const marked = HITS[4]!
    const injected = renderToStaticMarkup(createElement(Highlight, { name: marked.name, query: '<img src=x onerror=alert(1)>' }))
    expect(injected).not.toContain('<img')
    expect(injected).toContain('&lt;img')
    expect(highlight(marked.name, '<img src=x onerror=alert(1)>')).toEqual(['<img src=x onerror=alert(1)>'])
    expect(highlight('Café Folder', QUERY)).toEqual(['Café'])
    expect(highlight('İstanbul', 'stan')).toEqual(['stan'])
    expect(highlight('İstanbul', 'istanbul')).toEqual(['İstanbul'])
    expect(highlight('ﬁle', 'file')).toEqual(['ﬁle'])
    expect(highlight('ΟΣ', 'ς')).toEqual(['Σ'])
    expect(highlight('ΟΣ', 'σ')).toEqual([])
    expect(highlight('strasse', 'ß')).toEqual([])

    listing = undefined
    statusReads.length = 0
    const asked: string[] = []
    const other: string[] = []
    const listeners = new Set<(event: DriveEvent) => void>()
    const source = {
      search: async (query: string) => { asked.push(query); return HITS },
      subscribe: (listener: (event: DriveEvent) => void) => { listeners.add(listener); return () => listeners.delete(listener) },
      list: async () => { other.push('list'); return { items: [] } },
      recent: async () => { other.push('recent'); return [] },
      photos: async () => { other.push('photos'); return { entries: [] } },
      shared: async () => { other.push('shared'); return [] },
      trash: async () => { other.push('trash'); return [] },
    }
    const page = await show(createElement(DriveSourceProvider, { source: source as never }, createElement(Section, { nav: nav(QUERY) })))
    expect(asked).toEqual([QUERY])
    expect(other).toEqual([])
    expect(rows(page.container)).toEqual(HITS.map((row) => row.id))
    expect(page.text()).toContain('Café Folder')
    expect(allElements(page.container).filter((el) => el.tagName === 'B').map((el) => el.textContent)).toContain('Café')
    expect(allElements(page.container).some((el) => el.tagName === 'IMG')).toBe(false)
    expect(page.text()).toContain('<img src=x onerror=alert(1)>')
    expect(searchMenu(HITS[0]!).some((entry) => entry.id === 'enclosing')).toBe(true)
    const read = asHits(HITS)
    for (const name of ['All', 'Folders', 'Photos', 'Documents'] as const) {
      const button = pill(page.container, name)
      const count = byPill(read, name).length
      expect(button.getAttribute('data-count')).toBe(String(count))
      expect(button.textContent.replace(/\s+/g, ' ').trim()).toBe(`${name} ${count}`)
    }
    await React.act(async () => { click(page.container, pill(page.container, 'Photos')) })
    await settle()
    expect(asked).toEqual([QUERY])
    expect(rows(page.container)).toEqual(byPill(read, 'Photos').map((hit) => hit.item.id))
    expect(rows(page.container)).toHaveLength(byPill(read, 'Photos').length)
    await page.unmount()
  })

  test('no hits says No files match and the query stays', async () => {
    listing = undefined
    const asked: string[] = []
    const source = {
      search: async (query: string) => { asked.push(query); return [] },
      subscribe: () => () => {},
    }
    const page = await show(createElement(DriveSourceProvider, { source: source as never }, createElement(Section, { nav: nav(QUERY) })))
    expect(asked).toEqual([QUERY])
    expect(page.text()).toContain('No files match')
    expect(page.text()).not.toContain('No results')
    expect(field(page.container).value).toBe(QUERY)
    expect(rows(page.container)).toEqual([])
    await page.unmount()
  })

  test('while the index is building the page says so and followListing stops at the terminal state', async () => {
    const line = (progress: ListingProgress) => `The index is still building: ${progress.done.toLocaleString('en-US')} of ${progress.total.toLocaleString('en-US')}.`
    const steps: (ListingProgress | undefined)[] = [{ done: 18, total: 40 }, { done: 19, total: 40 }, undefined]
    const seen: (ListingProgress | undefined)[] = []
    const listeners = new Set<(event: DriveEvent) => void>()
    const direct = { subscribe: (listener: (event: DriveEvent) => void) => { listeners.add(listener); return () => { listeners.delete(listener) } } }
    let at = 0
    const stop = followListing(direct, async () => steps[Math.min(at, steps.length - 1)], (progress) => { seen.push(progress) }, 0)
    await new Promise((resolve) => setTimeout(resolve, 5))
    at = 1
    listeners.forEach((listener) => listener({ kind: 'changed', folderId: null }))
    await new Promise((resolve) => setTimeout(resolve, 5))
    at = 2
    listeners.forEach((listener) => listener({ kind: 'sync', sync: {} as never }))
    await new Promise((resolve) => setTimeout(resolve, 5))
    expect(seen).toEqual(steps)
    expect(listeners.size).toBe(0)
    stop()
    listeners.forEach((listener) => listener({ kind: 'changed', folderId: null }))
    await new Promise((resolve) => setTimeout(resolve, 5))
    expect(seen).toEqual(steps)

    listing = { done: 18, total: 40 }
    statusReads.length = 0
    const pageListeners = new Set<(event: DriveEvent) => void>()
    const source = {
      search: async () => [HITS[3]!],
      subscribe: (listener: (event: DriveEvent) => void) => { pageListeners.add(listener); return () => pageListeners.delete(listener) },
    }
    const page = await show(createElement(DriveSourceProvider, { source: source as never }, createElement(Section, { nav: nav(QUERY) })))
    expect(page.text()).toContain(line(listing))
    expect(page.text()).not.toContain('1 of 1')
    expect(pageListeners.size).toBe(1)
    listing = { done: 19, total: 40 }
    await React.act(async () => { pageListeners.forEach((listener) => listener({ kind: 'changed', folderId: null })) })
    await settle()
    expect(page.text()).toContain(line(listing))
    listing = undefined
    await React.act(async () => { pageListeners.forEach((listener) => listener({ kind: 'changed', folderId: null })) })
    await React.act(async () => { await new Promise((resolve) => setTimeout(resolve, 1100)) })
    await settle()
    expect(page.text()).not.toContain('The index is still building')
    expect(pageListeners.size).toBe(0)
    const reads = statusReads.length
    await React.act(async () => { pageListeners.forEach((listener) => listener({ kind: 'sync', sync: {} as never })) })
    await React.act(async () => { await new Promise((resolve) => setTimeout(resolve, 50)) })
    expect(statusReads.length).toBe(reads)
    expect(rows(page.container)).toEqual([HITS[3]!.id])
    await page.unmount()
  })

  test('a failed search shows the error and Try again, and the previous results stay', async () => {
    listing = undefined
    const kept = HITS[2]!
    const next = item({ id: 'later', name: 'Other Notes.txt' })
    let fail = false
    const asked: string[] = []
    const source = {
      search: async (query: string) => {
        asked.push(query)
        if (fail) throw new DriveSourceError('offline')
        return query === QUERY ? [kept] : [next]
      },
      subscribe: () => () => {},
    }
    const page = await show(createElement(DriveSourceProvider, { source: source as never }, createElement(Section, { nav: nav(QUERY) })))
    expect(rows(page.container)).toEqual([kept.id])
    fail = true
    await React.act(async () => { typeQuery(page.container, field(page.container), 'other') })
    await settle()
    expect(field(page.container).value).toBe('other')
    expect(page.text()).toContain(actionFailure('search', 'offline').text)
    expect(buttons(page.container, 'Try again')).toHaveLength(1)
    expect(rows(page.container)).toEqual([kept.id])
    expect(page.text()).toContain(kept.name)
    expect(page.text()).not.toContain('No files match')
    fail = false
    await React.act(async () => { click(page.container, buttons(page.container, 'Try again')[0]!) })
    await settle()
    expect(asked.filter((query) => query === 'other').length).toBeGreaterThanOrEqual(2)
    expect(rows(page.container)).toEqual([next.id])
    expect(page.text()).not.toContain(actionFailure('search', 'offline').text)
    expect(buttons(page.container, 'Try again')).toHaveLength(0)
    await page.unmount()
  })
})
