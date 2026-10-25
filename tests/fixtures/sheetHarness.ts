/**
 * What the Review and Trash tests share (task 1683 slice 4): a stand-in for the window's `Modal` that renders its
 * title, children and footer as plain elements and remembers the props it was given, so a test can read what a sheet
 * says, press what it offers, and ask it to close the way Esc and the close button do. The real `Modal` is a hook
 * component over the DOM (focus trap, Esc listener, the open-sheet count); its own behaviour is
 * tests/openModalCount.test.tsx and tests/uiAccessibility.test.ts.
 */
import { createElement } from 'react'
import { expand, textOf, type Mounted, type TreeNode } from './componentHarness'

export interface ModalCall {
  open: boolean
  title: unknown
  ariaLabel?: string
  maxWidth?: number
  initialFocusRef?: unknown
  footer?: unknown
  onClose: () => void
}

export function modalStub() {
  const calls: ModalCall[] = []
  const Modal = (props: any) => {
    calls.push(props)
    if (!props.open) return null
    return createElement(
      'div',
      { role: 'dialog', 'aria-modal': 'true', 'aria-label': props.ariaLabel ?? (typeof props.title === 'string' ? props.title : undefined), 'data-max-width': props.maxWidth },
      createElement('h2', { 'data-sheet-title': '' }, props.title),
      createElement('button', { type: 'button', 'aria-label': 'Close', onClick: props.onClose }, '×'),
      createElement('div', { 'data-sheet-body': '' }, props.children),
      props.footer === undefined ? createElement('button', { type: 'button', onClick: props.onClose }, 'Close') : props.footer,
    )
  }
  return { Modal, calls, last: () => calls[calls.length - 1] }
}

/**
 * The props the sheet was last drawn with. The stand-in only runs when the tree is expanded, so read the tree first:
 * a sheet nobody has looked at has not been drawn.
 */
export function modalNow(m: Mounted, modal: { last: () => ModalCall }): ModalCall {
  m.elements()
  return modal.last()
}

export const settle = async (m: Mounted) => { await m.flush(); await m.flush(); await m.flush(); await m.flush() }
export const find = (m: Mounted, match: (el: TreeNode) => boolean) => m.elements().filter(match)
export const text = (m: Mounted) => textOf(expand(m.tree()))
export const buttonLabel = (el: TreeNode) => el.props['aria-label'] ?? textOf(expand(el.props.children)).trim()
export const buttons = (m: Mounted) => find(m, (el) => el.type === 'button')
export function button(m: Mounted, label: string): TreeNode {
  const hit = buttons(m).filter((el) => buttonLabel(el) === label)
  if (hit.length !== 1) throw new Error(`expected one button "${label}", found ${hit.length}: ${buttons(m).map(buttonLabel).join(' | ')}`)
  return hit[0]
}
export async function press(m: Mounted, label: string) {
  await button(m, label).props.onClick()
  m.render()
  await settle(m)
}
/** Press the first button with this label, for labels a list repeats (one "Restore" per row). */
export async function pressFirst(m: Mounted, label: string) {
  const hit = buttons(m).filter((el) => buttonLabel(el) === label)
  if (hit.length === 0) throw new Error(`no button "${label}": ${buttons(m).map(buttonLabel).join(' | ')}`)
  await hit[0].props.onClick()
  m.render()
  await settle(m)
}
/** Every text a person reads, one string per text node, with icons skipped. */
export function lines(m: Mounted): string[] {
  const out: string[] = []
  const walk = (n: any) => {
    if (Array.isArray(n)) return n.forEach(walk)
    if (n == null || typeof n === 'boolean') return
    if (typeof n !== 'object' || !n.props) { out.push(String(n)); return }
    if (n.type === 'svg') return
    walk(n.props.children)
  }
  walk(expand(m.tree()))
  return out.map((s) => s.trim()).filter(Boolean)
}
