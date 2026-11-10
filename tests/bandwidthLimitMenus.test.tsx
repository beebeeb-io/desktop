/**
 * Settings > Bandwidth: the Upload limit and Download limit menus open in full.
 *
 * The limits sit at the bottom of the page, so a menu that only ever opens below its button runs past the
 * bottom of the window (the 2026-11-09 headless pass found it clipped and off the viewport on the DW-0785
 * and DW-0786 frames). The open listbox must leave the overflow-hidden card and stay inside the viewport.
 * Every option must be inside the viewport and clickable.
 */
import { afterAll, beforeAll, expect, test } from 'bun:test'
import { allElements, installMiniDom, MiniElement, type MiniDomInstall } from './fixtures/miniDom'

const VIEW = { width: 1100, height: 600 }
const OPTIONS = [
  { value: '0', label: 'No limit' },
  { value: '1', label: '1 Mbps' },
  { value: '2', label: '2 Mbps' },
  { value: '5', label: '5 Mbps' },
  { value: '10', label: '10 Mbps' },
  { value: '20', label: '20 Mbps' },
  { value: '50', label: '50 Mbps' },
  { value: '100', label: '100 Mbps' },
]
/** The model's layout: one option row is 31.5px; the list adds its 4px padding and 1px border on each side. */
const ROW = 31.5
const LIST_PAD = 10

let dom: MiniDomInstall
let React: typeof import('react')
let client: typeof import('react-dom/client')
const realRect = MiniElement.prototype.getBoundingClientRect

beforeAll(async () => {
  dom = installMiniDom('', { innerWidth: VIEW.width, innerHeight: VIEW.height })
  // The fixture models no layout. Give the two limit buttons a place at the bottom of the window and give
  // the portaled list its real height, so the placement under test is the one a 600px window produces.
  MiniElement.prototype.getBoundingClientRect = function (this: MiniElement) {
    if (this.getAttribute('aria-haspopup') === 'listbox') {
      const top = this.getAttribute('aria-label') === 'Upload limit' ? 548 : 552
      return { top, left: 796, right: 936, bottom: top + 28, width: 140, height: 28, x: 796, y: top }
    }
    if (this.getAttribute('role') === 'listbox') {
      return { top: 0, left: 796, right: 936, bottom: 0, width: 140, height: LIST_PAD + this.childNodes.length * ROW, x: 796, y: 0 }
    }
    if (this.getAttribute('role') === 'option') {
      return { top: 0, left: 796, right: 936, bottom: 0, width: 140, height: ROW, x: 796, y: 0 }
    }
    return realRect.call(this)
  }
  React = await import('react')
  client = await import('react-dom/client')
})

afterAll(() => {
  MiniElement.prototype.getBoundingClientRect = realRect
  dom.restore()
})

function clipping(el: MiniElement | null): boolean {
  for (let node = el; node; node = node.parentElement) {
    if (/\boverflow-hidden\b/.test(node.className)) return true
  }
  return false
}

/** Open both limit menus (as the frames do) and check the one under test. */
async function openMenu(label: 'Upload limit' | 'Download limit') {
  const kit = await import('../src/v2/sections/settings/shell/kit')
  const container = dom.document.createElement('div')
  dom.document.body.appendChild(container)
  const root = client.createRoot(container as never)
  const picked: string[] = []
  await React.act(async () => {
    root.render(
      <kit.Group>
        <kit.Row label="Upload" right={<kit.Choice label="Upload limit" value="0" open={label === 'Upload limit'} options={OPTIONS} onPick={(v) => picked.push(v)} />} />
        <kit.Row label="Download" right={<kit.Choice label="Download limit" value="0" open={label === 'Download limit'} options={OPTIONS} onPick={(v) => picked.push(v)} />} />
      </kit.Group>,
    )
  })
  return { dom, root, picked }
}

for (const label of ['Upload limit', 'Download limit'] as const) {
  test(`the open ${label} menu is inside the viewport, outside the card, and every option fits`, async () => {
    const { dom: d, root, picked } = await openMenu(label)
    const list = allElements(d.document.body).find((el) => el.getAttribute('role') === 'listbox' && el.getAttribute('aria-label') === label) ?? null
    expect(list).not.toBeNull()

    const group = allElements(d.document.body).find((el) => el.getAttribute('data-testid') === 'group') ?? null
    expect(group?.contains(list)).toBe(false)
    expect(clipping(list)).toBe(false)

    const top = Number.parseFloat(String(list?.style.top ?? ''))
    const height = list?.getBoundingClientRect().height ?? 0
    expect(top).toBeGreaterThanOrEqual(8)
    expect(top + height).toBeLessThanOrEqual(VIEW.height - 8)
    // No option may hang below the list's own bottom: every option is inside the viewport it sits in.
    const options = allElements(list!).filter((el) => el.getAttribute('role') === 'option')
    expect(options.map((o) => o.textContent)).toEqual(['No limit', '1 Mbps', '2 Mbps', '5 Mbps', '10 Mbps', '20 Mbps', '50 Mbps', '100 Mbps'])
    expect(options.length).toBe(8)
    for (const [i, option] of options.entries()) {
      const optionTop = top + 5 + i * ROW
      expect(optionTop).toBeGreaterThanOrEqual(8)
      expect(optionTop + ROW).toBeLessThanOrEqual(VIEW.height - 8)
      expect(list?.contains(option)).toBe(true)
    }
    expect(picked).toEqual([])
    await React.act(async () => root.unmount())
  })
}
