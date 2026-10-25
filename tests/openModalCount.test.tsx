/**
 * The count of open sheets in one webview (task 1683 slice 1, two-surface spec §8.2): the macOS Beebeeb window reads
 * it to keep ⌘1, ⌘[ and ⌘, from moving the window while a sheet is open, and tells Rust so the menu items are
 * disabled too. It is the shared `Modal`'s own count, so a sheet anywhere in the window (Sign out, Repair, Choose
 * folders) counts without the window knowing which one it is.
 *
 * Mounted with the real `react-dom` on the minimal DOM, because the count lives in `Modal`'s effects.
 */
import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { installMiniDom, type MiniDomInstall } from './fixtures/miniDom'

let dom: MiniDomInstall
let React: typeof import('react')
let client: typeof import('react-dom/client')

beforeAll(async () => {
  dom = installMiniDom('')
  React = await import('react')
  client = await import('react-dom/client')
  ;(globalThis as any).IS_REACT_ACT_ENVIRONMENT = true
})
afterAll(() => dom.restore())

describe('openModalCount', () => {
  test('counts each open Modal, follows open and close, and tells subscribers', async () => {
    const ui = await import('../src/windows/ui')
    const container = dom.document.createElement('div')
    dom.document.body.appendChild(container)
    const root = client.createRoot(container as never)
    let told = 0
    const unsubscribe = ui.subscribeOpenModals(() => { told += 1 })
    const render = (a: boolean, b: boolean) =>
      React.act(async () => {
        root.render(
          <>
            <ui.Modal open={a} onClose={() => {}} title="First">one</ui.Modal>
            <ui.Modal open={b} onClose={() => {}} title="Second">two</ui.Modal>
          </>,
        )
      })

    await render(false, false)
    expect(ui.openModalCount()).toBe(0)
    await render(true, false)
    expect(ui.openModalCount()).toBe(1)
    await render(true, true)
    expect(ui.openModalCount()).toBe(2)
    await render(false, true)
    expect(ui.openModalCount()).toBe(1)
    expect(told).toBeGreaterThanOrEqual(3)
    await React.act(async () => root.unmount())
    expect(ui.openModalCount()).toBe(0)
    unsubscribe()
  })
})
