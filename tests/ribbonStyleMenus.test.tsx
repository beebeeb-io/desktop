/**
 * Office ribbon (DW-1507): Font, Font size and Paragraph style are chevron buttons. The controls row is
 * `overflow-hidden`, so a list drawn inside it is clipped; a click that only calls `onRun` with no `uno`
 * does nothing at all. Each of the three must either open a listbox portaled to document.body (so the
 * toolbar cannot clip it) and dispatch the engine command on a choice, or be disabled with a title that
 * says it is not available in this version. Esc and an outside click close the open list.
 */
import { afterAll, beforeAll, expect, test } from 'bun:test'
import { allElements, click, installMiniDom, type MiniDomInstall, type MiniElement } from './fixtures/miniDom'

let dom: MiniDomInstall
let React: typeof import('react')
let client: typeof import('react-dom/client')

beforeAll(async () => {
  dom = installMiniDom('', { innerWidth: 1100, innerHeight: 700 })
  React = await import('react')
  client = await import('react-dom/client')
})
afterAll(() => dom.restore())

/** True when an ancestor clips its children (the ribbon's controls row is `overflow-hidden`). */
function clipping(el: MiniElement | null): boolean {
  for (let node = el; node; node = node.parentElement) {
    if (/\boverflow-hidden\b/.test(node.className)) return true
  }
  return false
}

const listbox = (label: string) =>
  allElements(dom.document.body).find((el) => el.getAttribute('role') === 'listbox' && el.getAttribute('aria-label') === label) ?? null

async function renderRibbon() {
  const { Ribbon } = await import('../src/v2/sections/open/office/Ribbon')
  const container = dom.document.createElement('div')
  dom.document.body.appendChild(container)
  const root = client.createRoot(container as never)
  const picked: { id: string; value: string }[] = []
  await React.act(async () => {
    root.render(
      <Ribbon
        app="writer" tab="Home" onTab={() => {}}
        onRun={(c) => { if (c.select) picked.push({ id: c.id, value: c.select.value }) }}
        dirty={false} saving={false} onSave={() => {}} onPalette={() => {}}
      />,
    )
  })
  return { container, root, picked }
}

const CASES = [
  { label: 'Font', id: 'font' },
  { label: 'Font size', id: 'size' },
  { label: 'Paragraph style', id: 'style' },
] as const

for (const { label, id } of CASES) {
  test(`${label}: opens a listbox outside the toolbar, or is disabled with a reason`, async () => {
    const { container, root, picked } = await renderRibbon()
    const button = allElements(dom.document.body).find((el) => el.tagName === 'BUTTON' && el.getAttribute('aria-label') === label) ?? null
    expect(button).not.toBeNull()

    const disabled = button!.disabled === true || button!.getAttribute('disabled') !== null
    if (disabled) {
      expect((button!.getAttribute('title') ?? '').toLowerCase()).toContain('not available in this version')
      await React.act(async () => root.unmount())
      return
    }

    // Open: the list leaves the toolbar and is not clipped by it.
    await React.act(async () => { click(container, button!) })
    const list = listbox(label)
    expect(list).not.toBeNull()
    const toolbar = allElements(dom.document.body).find((el) => el.getAttribute('role') === 'toolbar') ?? null
    expect(toolbar?.contains(list!)).toBe(false)
    expect(clipping(list)).toBe(false)
    const options = allElements(list!).filter((el) => el.getAttribute('role') === 'option')
    expect(options.length).toBeGreaterThan(0)

    // Esc closes it.
    await React.act(async () => {
      for (const fn of dom.document.listenersFor('keydown', true)) fn({ key: 'Escape', stopPropagation() {} })
    })
    expect(listbox(label)).toBeNull()

    // Reopen, then an outside click closes it.
    await React.act(async () => { click(container, button!) })
    expect(listbox(label)).not.toBeNull()
    const outside = dom.document.createElement('div')
    dom.document.body.appendChild(outside)
    await React.act(async () => {
      for (const fn of dom.document.listenersFor('mousedown', false)) fn({ target: outside })
    })
    expect(listbox(label)).toBeNull()

    // Reopen and pick the second option: the choice reaches onSelect and the list closes.
    await React.act(async () => { click(container, button!) })
    const second = allElements(listbox(label)!).filter((el) => el.getAttribute('role') === 'option')[1]
    const optionButton = allElements(second).find((el) => el.tagName === 'BUTTON') ?? null
    expect(optionButton).not.toBeNull()
    await React.act(async () => { click(dom.document.body, optionButton!) })
    expect(picked.length).toBe(1)
    expect(picked[0].id).toBe(id)
    expect(picked[0].value.length).toBeGreaterThan(0)
    expect(listbox(label)).toBeNull()

    await React.act(async () => root.unmount())
  })
}
