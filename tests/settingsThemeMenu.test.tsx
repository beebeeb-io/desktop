/**
 * Settings > General > Theme: the choice menu is clipped by the card's overflow-hidden.
 * The open listbox must leave the card, and its place must be computed under the button.
 */
import { afterAll, beforeAll, expect, test } from 'bun:test'
import { allElements, installMiniDom, type MiniDomInstall, type MiniElement } from './fixtures/miniDom'

let dom: MiniDomInstall
let React: typeof import('react')
let client: typeof import('react-dom/client')

beforeAll(async () => {
  dom = installMiniDom('')
  React = await import('react')
  client = await import('react-dom/client')
})
afterAll(() => dom.restore())

function clipping(el: MiniElement | null): boolean {
  for (let node = el; node; node = node.parentElement) {
    if (/\boverflow-hidden\b/.test(node.className)) return true
  }
  return false
}

test('the open theme listbox is not clipped and sits below the button', async () => {
  const kit = await import('../src/v2/sections/settings/shell/kit')
  const container = dom.document.createElement('div')
  dom.document.body.appendChild(container)
  const root = client.createRoot(container as never)
  await React.act(async () => {
    root.render(
      <kit.Group>
        <kit.Choice
          label="Theme"
          value="system"
          open
          options={[
            { value: 'system', label: 'System' },
            { value: 'light', label: 'Light' },
            { value: 'dark', label: 'Dark' },
          ]}
          onPick={() => {}}
        />
      </kit.Group>,
    )
  })
  const list = allElements(dom.document.body).find((el) => el.getAttribute('role') === 'listbox') ?? null
  const group = allElements(dom.document.body).find((el) => el.getAttribute('data-testid') === 'group') ?? null
  expect(list).not.toBeNull()
  expect(group?.contains(list)).toBe(false)
  expect(clipping(list)).toBe(false)
  const button = { top: 40, right: 220, bottom: 68, left: 80, width: 140, height: 28 }
  const place = kit.popoverPlace(button, { width: 800, height: 600 })
  expect(place.top).toBe(button.bottom + 4)
  const measured = list ? { top: 0, right: 0, bottom: 0, left: 0, width: 0, height: 0 } : button
  const view = { width: window.innerWidth || 800, height: window.innerHeight || 600 }
  expect(String(list?.style.top ?? '')).toBe(`${kit.popoverPlace(measured, view).top}px`)
  expect(list?.className ?? '').not.toContain('amber')
  expect(list?.className ?? '').toContain('text-[13px]')
  const trigger = allElements(dom.document.body).find((el) => el.getAttribute('aria-haspopup') === 'listbox')
  expect(trigger?.getAttribute('aria-haspopup')).toBe('listbox')
  await React.act(async () => root.unmount())
})
