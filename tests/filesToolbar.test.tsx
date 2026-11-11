// The Files toolbar is the board's MacToolbar (DW-0401): search is an inline field that reads "Search names" with
// Ctrl+F beside it, Enter opens the existing search section, and there is no Filter control. Sort and the view
// buttons stay. The field never searches on a letter: only Enter calls onFind.
import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import React, { createElement, type ReactElement } from 'react'
import { Toolbar } from '../src/v2/sections/files/FilesPane'
import { DEFAULT_PREFS } from '../src/v2/sections/files/model'
import { allElements, installMiniDom, MiniElement, type MiniDomInstall, type MiniNode } from './fixtures/miniDom'

let dom: MiniDomInstall
let client: typeof import('react-dom/client')

beforeAll(async () => {
  dom = installMiniDom('')
  // React's change plugin takes the input-event polyfill path without this, and that path reads a focused
  // instance the mini DOM does not track. `oninput` on the document makes it use the native input event.
  Object.defineProperty(dom.document, 'oninput', { value: null, configurable: true })
  client = await import('react-dom/client')
})

afterAll(() => dom.restore())

function props(onFind: () => void) {
  return {
    p: {
      listing: { folder: { id: null, name: 'Files' }, path: [{ id: null, name: 'Files' }], items: [], asOf: '2026-11-10T00:00:00.000Z', stale: false },
      items: [],
      prefs: DEFAULT_PREFS,
      selection: new Set<string>(),
    },
    h: { onFind },
  }
}

async function show(node: ReactElement) {
  const container = dom.document.createElement('div')
  dom.document.body.appendChild(container)
  const root = client.createRoot(container as never)
  await React.act(async () => { root.render(node) })
  return {
    container: container as MiniElement,
    async unmount() {
      await React.act(async () => root.unmount())
      container.parentNode?.removeChild(container)
    },
  }
}

/** A keydown as a WebView delivers it: through the listeners React put on the root container. */
function keydown(container: MiniNode, target: MiniElement, key: string) {
  const event = {
    type: 'keydown',
    key,
    target,
    srcElement: target,
    currentTarget: container,
    bubbles: true,
    cancelable: true,
    defaultPrevented: false,
    isTrusted: true,
    preventDefault() { this.defaultPrevented = true },
    stopPropagation() {},
  }
  for (const fn of container.listenersFor('keydown', true)) fn(event)
  for (const fn of container.listenersFor('keydown', false)) fn(event)
}

function searchFields(root: MiniNode): MiniElement[] {
  return allElements(root).filter((el) => el.tagName === 'INPUT' && el.getAttribute('aria-label') === 'Search names')
}

function buttonsNamed(root: MiniNode, name: string): MiniElement[] {
  // The old search control's text was "Search names Ctrl+F", so an exact match would miss it. A button
  // counts when its accessible name (aria-label) or its visible text contains the name.
  return allElements(root).filter((el) => el.tagName === 'BUTTON' && (el.getAttribute('aria-label') === name || el.textContent.includes(name)))
}

describe('the Files toolbar matches the frame', () => {
  test('search is one field named Search names with Ctrl+F beside it, not a button', async () => {
    const page = await show(createElement(Toolbar, props(() => {})))
    const fields = searchFields(page.container)
    expect(fields).toHaveLength(1)
    expect(fields[0]!.getAttribute('placeholder')).toBe('Search names')
    expect(page.container.textContent).toContain('Ctrl+F')
    expect(buttonsNamed(page.container, 'Search names')).toHaveLength(0)
    await page.unmount()
  })

  test('Enter on the field calls onFind once; another key does not', async () => {
    let calls = 0
    const page = await show(createElement(Toolbar, props(() => { calls += 1 })))
    const field = searchFields(page.container)[0]!
    keydown(page.container, field, 'a')
    expect(calls).toBe(0)
    keydown(page.container, field, 'Enter')
    expect(calls).toBe(1)
    await page.unmount()
  })

  test('no Filter control, and Sort, as List and as Grid stay', async () => {
    const page = await show(createElement(Toolbar, props(() => {})))
    expect(buttonsNamed(page.container, 'Filter')).toHaveLength(0)
    expect(buttonsNamed(page.container, 'Sort')).toHaveLength(1)
    expect(buttonsNamed(page.container, 'as List')).toHaveLength(1)
    expect(buttonsNamed(page.container, 'as Grid')).toHaveLength(1)
    await page.unmount()
  })
})
