// Dev only (batch M2), never bundled: replays a live state's steps in the page, through the real DOM events the app
// listens to (click, contextmenu, dblclick, keydown, input), so the real handlers move the app into the frame's state.
// Each step waits up to a few seconds for its control (lazy sections, fixtures that answer a tick later).

export type Target = string

/** A context or app menu's item by its id (the menus' `data-menu-item`). */
export const menuItem = (id: string): Target => `[data-menu-item="${id}"]`
export type Step =
  | { click: Target }
  | { dbl: Target }
  | { context: Target }
  | { ctrl: Target }
  | { shift: Target }
  | { hover: Target }
  | { key: string; on?: Target }
  | { fill: Target; value: string }
  | { wait: Target }
  | { scroll: Target; top: number }
  /** Types into a contenteditable editor (CodeMirror) as a person would: the browser's own insertText. */
  | { type: Target; text: string }
  | { pause: number }

const visible = (e: Element) => {
  const el = e as HTMLElement
  if (typeof el.checkVisibility === 'function') return el.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true })
  return el.offsetParent !== null || el.getClientRects().length > 0
}
const norm = (s: string | null | undefined) => (s ?? '').replace(/\s+/g, ' ').trim()
const CONTROLS = 'button,a[href],[role=button],[role=link],[role=menuitem],[role=menuitemradio],[role=menuitemcheckbox],[role=tab],[role=switch],[role=checkbox],[role=radio],[role=option],[role=treeitem],summary,select'
const nameOf = (e: Element) => norm(e.getAttribute('aria-label') || (e as HTMLElement).innerText || e.textContent || e.getAttribute('title'))

/**
 * Targets: `row:<id>` (a Files/Backups row or tile), `btn:<name>` (a control by its accessible name, exact first),
 * `menu:<label>` (a menu item), `text:<words>` (the element that holds exactly these words), `field:<label>` (an input
 * by label, aria-label or placeholder), `any:<css>` (the first match, even visually hidden: a code field drawn as boxes),
 * or a CSS selector.
 */
export function find(target: Target): HTMLElement | null {
  const [kind, ...rest] = target.split(':')
  const arg = rest.join(':')
  const pick = (list: Iterable<Element>) => [...list].find(visible) as HTMLElement | undefined
  if (kind === 'any') return document.querySelector(arg) as HTMLElement | null
  if (kind === 'row') return pick(document.querySelectorAll(`[data-row="${CSS.escape(arg)}"],[data-tile="${CSS.escape(arg)}"]`)) ?? null
  if (kind === 'btn' || kind === 'menu') {
    const sel = kind === 'menu' ? '[role=menuitem],[role=menuitemradio],[role=menuitemcheckbox]' : CONTROLS
    const all = [...document.querySelectorAll(sel)].filter(visible)
    return (all.find((e) => nameOf(e) === arg) ?? all.find((e) => nameOf(e).startsWith(arg)) ?? all.find((e) => nameOf(e).includes(arg)) ?? null) as HTMLElement | null
  }
  if (kind === 'text') {
    const all = [...document.querySelectorAll('body *')].filter((e) => norm(e.textContent) === arg && visible(e))
    // The deepest element with exactly these words, so a click lands where a person would click.
    return (all.find((e) => ![...e.children].some((c) => norm(c.textContent) === arg)) ?? null) as HTMLElement | null
  }
  if (kind === 'field') {
    const inputs = [...document.querySelectorAll('input,textarea,[contenteditable=true]')].filter(visible) as HTMLElement[]
    return inputs.find((i) => norm(i.getAttribute('aria-label')) === arg || norm(i.getAttribute('placeholder')) === arg || norm((i as HTMLInputElement).labels?.[0]?.textContent) === arg)
      ?? inputs.find((i) => norm(i.getAttribute('aria-label')).includes(arg) || norm(i.getAttribute('placeholder')).includes(arg) || norm((i as HTMLInputElement).labels?.[0]?.textContent).includes(arg))
      ?? null
  }
  return pick(document.querySelectorAll(target)) ?? null
}

async function waitFor(target: Target, ms = 4000): Promise<HTMLElement | null> {
  const until = Date.now() + ms
  for (;;) {
    const el = find(target)
    if (el) return el
    if (Date.now() > until) return null
    await new Promise((r) => setTimeout(r, 60))
  }
}

function mouse(el: HTMLElement, type: string, init: MouseEventInit = {}) {
  const r = el.getBoundingClientRect()
  const base = { bubbles: true, cancelable: true, view: window, clientX: r.left + Math.min(24, r.width / 2), clientY: r.top + r.height / 2 }
  const E = type.startsWith('pointer') ? PointerEvent : MouseEvent
  el.dispatchEvent(new E(type, { ...base, ...init }))
}

function clickOn(el: HTMLElement, init: MouseEventInit = {}) {
  el.scrollIntoView?.({ block: 'nearest' })
  mouse(el, 'pointerdown', init); mouse(el, 'mousedown', init)
  el.focus?.({ preventScroll: true })
  mouse(el, 'pointerup', init); mouse(el, 'mouseup', init); mouse(el, 'click', { detail: 1, ...init })
}

function keyFrom(spec: string): KeyboardEventInit {
  const parts = spec.split('+')
  const key = parts.pop()!
  const named: Record<string, string> = { Esc: 'Escape', Del: 'Delete', Space: ' ', Up: 'ArrowUp', Down: 'ArrowDown', Left: 'ArrowLeft', Right: 'ArrowRight' }
  return { key: named[key] ?? key, ctrlKey: parts.includes('Ctrl'), shiftKey: parts.includes('Shift'), altKey: parts.includes('Alt'), metaKey: parts.includes('Win'), bubbles: true, cancelable: true }
}

/** React reads a controlled input's value through the native setter, then the `input` event. */
function fillIn(el: HTMLElement, value: string) {
  el.focus()
  const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype
  const setter = Object.getOwnPropertyDescriptor(proto, 'value')?.set
  if (setter && (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement)) setter.call(el, value)
  else el.textContent = value
  el.dispatchEvent(new Event('input', { bubbles: true }))
  el.dispatchEvent(new Event('change', { bubbles: true }))
}

const tick = () => new Promise((r) => setTimeout(r, 140))

/** Runs the steps; resolves to null when all ran, or to why one could not. */
export async function run(steps: Step[]): Promise<string | null> {
  for (const [i, step] of steps.entries()) {
    const label = `step ${i + 1} ${JSON.stringify(step)}`
    if ('pause' in step) { await new Promise((r) => setTimeout(r, step.pause)); continue }
    if ('key' in step) {
      const on = step.on ? await waitFor(step.on) : (document.activeElement as HTMLElement | null) ?? document.body
      if (!on) return `${label}: no control`
      on.dispatchEvent(new KeyboardEvent('keydown', keyFrom(step.key)))
      on.dispatchEvent(new KeyboardEvent('keyup', keyFrom(step.key)))
      await tick()
      continue
    }
    const target = 'click' in step ? step.click : 'dbl' in step ? step.dbl : 'context' in step ? step.context : 'ctrl' in step ? step.ctrl
      : 'shift' in step ? step.shift : 'hover' in step ? step.hover : 'fill' in step ? step.fill : 'scroll' in step ? step.scroll : 'type' in step ? step.type : step.wait
    const el = await waitFor(target)
    if (!el) return `${label}: no control`
    if ('click' in step) clickOn(el)
    else if ('ctrl' in step) clickOn(el, { ctrlKey: true })
    else if ('shift' in step) clickOn(el, { shiftKey: true })
    else if ('dbl' in step) { clickOn(el); clickOn(el, { detail: 2 }); mouse(el, 'dblclick', { detail: 2 }) }
    else if ('context' in step) mouse(el, 'contextmenu', { button: 2 })
    else if ('hover' in step) { mouse(el, 'pointerover'); mouse(el, 'mouseover'); mouse(el, 'pointerenter'); mouse(el, 'mouseenter') }
    else if ('fill' in step) fillIn(el, step.value)
    else if ('type' in step) { el.focus(); document.execCommand('insertText', false, step.text) }
    else if ('scroll' in step) { el.scrollTop = step.top; el.dispatchEvent(new Event('scroll', { bubbles: true })) }
    await tick()
  }
  return null
}
