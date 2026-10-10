/**
 * The smallest DOM that `react-dom/client` needs to mount, update and REMOUNT a real component tree in
 * `bun test` (this repo has no DOM library). It exists for the tests where React's own reconciliation is
 * the thing under test, such as `AccountSessionBoundary` remounting its keyed children; the component
 * harness (`componentHarness.ts`) cannot show that, because it executes one declaration with its own hooks.
 *
 * What it models: element and text nodes, attributes, inline style, the tree operations React calls,
 * focus, `<select>` options, and event listeners. `click` sends a click through the listener React
 * registered on the root container, so a handler runs inside React's event system as it does in a
 * WebView. What it does NOT model: layout, CSS, real focus order, or native macOS behaviour.
 *
 * `installMiniDom` replaces `window`, `document` and the element constructors on `globalThis`;
 * the returned `restore` puts the previous values back, so other test files see what they saw before.
 */

type Listener = { fn: (event: any) => void; capture: boolean }

const HTML_NS = 'http://www.w3.org/1999/xhtml'

export class MiniNode {
  nodeType = 0
  nodeName = ''
  parentNode: MiniNode | null = null
  childNodes: MiniNode[] = []
  ownerDocument: MiniDocument | null = null
  onclick: unknown = null
  private listeners = new Map<string, Listener[]>()

  get firstChild() { return this.childNodes[0] ?? null }
  get lastChild() { return this.childNodes[this.childNodes.length - 1] ?? null }
  get nextSibling() {
    const siblings = this.parentNode?.childNodes ?? []
    return siblings[siblings.indexOf(this) + 1] ?? null
  }
  get previousSibling() {
    const siblings = this.parentNode?.childNodes ?? []
    return siblings[siblings.indexOf(this) - 1] ?? null
  }
  get parentElement() { return this.parentNode instanceof MiniElement ? this.parentNode : null }

  appendChild<T extends MiniNode>(child: T): T {
    child.parentNode?.removeChild(child)
    child.parentNode = this
    this.childNodes.push(child)
    return child
  }
  insertBefore<T extends MiniNode>(child: T, before: MiniNode | null): T {
    if (before === null) return this.appendChild(child)
    child.parentNode?.removeChild(child)
    const at = this.childNodes.indexOf(before)
    if (at < 0) throw new Error('insertBefore: the reference node is not a child of this node')
    child.parentNode = this
    this.childNodes.splice(at, 0, child)
    return child
  }
  removeChild<T extends MiniNode>(child: T): T {
    const at = this.childNodes.indexOf(child)
    if (at < 0) throw new Error('removeChild: the node is not a child of this node')
    this.childNodes.splice(at, 1)
    child.parentNode = null
    return child
  }
  contains(node: MiniNode | null): boolean {
    for (let at = node; at; at = at.parentNode) if (at === this) return true
    return false
  }
  get textContent(): string { return this.childNodes.map((child) => child.textContent).join('') }
  set textContent(value: string) {
    for (const child of this.childNodes) child.parentNode = null
    this.childNodes = []
    if (value) this.appendChild(this.ownerDocument!.createTextNode(String(value)))
  }

  addEventListener(type: string, fn: (event: any) => void, options?: boolean | { capture?: boolean }) {
    const capture = typeof options === 'boolean' ? options : options?.capture === true
    const list = this.listeners.get(type) ?? []
    list.push({ fn, capture })
    this.listeners.set(type, list)
  }
  removeEventListener(type: string, fn: (event: any) => void, options?: boolean | { capture?: boolean }) {
    const capture = typeof options === 'boolean' ? options : options?.capture === true
    const list = this.listeners.get(type) ?? []
    this.listeners.set(type, list.filter((entry) => entry.fn !== fn || entry.capture !== capture))
  }
  /** The listeners for one phase, in registration order. */
  listenersFor(type: string, capture: boolean) {
    return (this.listeners.get(type) ?? []).filter((entry) => entry.capture === capture).map((entry) => entry.fn)
  }
}

export class MiniText extends MiniNode {
  data: string
  constructor(data: string) {
    super()
    this.nodeType = 3
    this.nodeName = '#text'
    this.data = data
  }
  get nodeValue() { return this.data }
  set nodeValue(value: string) { this.data = String(value) }
  override get textContent() { return this.data }
  override set textContent(value: string) { this.data = String(value) }
}

export class MiniComment extends MiniNode {
  data: string
  constructor(data: string) {
    super()
    this.nodeType = 8
    this.nodeName = '#comment'
    this.data = data
  }
  override get textContent() { return '' }
}

function styleObject() {
  const style: Record<string, unknown> = {}
  Object.defineProperty(style, 'setProperty', { value: (name: string, value: string) => { style[name] = value } })
  Object.defineProperty(style, 'removeProperty', { value: (name: string) => { delete style[name] } })
  return style
}

export class MiniElement extends MiniNode {
  tagName: string
  namespaceURI: string
  attributes = new Map<string, string>()
  style = styleObject()
  /** Set by React for `<option selected>` and by `<select>` updates. */
  selected = false
  private ownValue: string | undefined

  constructor(tag: string, namespace = HTML_NS) {
    super()
    this.nodeType = 1
    this.namespaceURI = namespace
    this.tagName = namespace === HTML_NS ? tag.toUpperCase() : tag
    this.nodeName = this.tagName
  }
  setAttribute(name: string, value: unknown) { this.attributes.set(name, String(value)) }
  setAttributeNS(_namespace: string | null, name: string, value: unknown) { this.setAttribute(name, value) }
  getAttribute(name: string) { return this.attributes.get(name) ?? null }
  hasAttribute(name: string) { return this.attributes.has(name) }
  removeAttribute(name: string) { this.attributes.delete(name) }
  removeAttributeNS(_namespace: string | null, name: string) { this.removeAttribute(name) }
  get id() { return this.getAttribute('id') ?? '' }
  get className() { return this.getAttribute('class') ?? '' }
  focus() { if (this.ownerDocument) this.ownerDocument.activeElement = this }
  blur() { if (this.ownerDocument?.activeElement === this) this.ownerDocument.activeElement = this.ownerDocument.body }
  querySelectorAll(): MiniElement[] { return [] }
  getBoundingClientRect() { return { top: 0, left: 0, right: 0, bottom: 0, width: 0, height: 0, x: 0, y: 0 } }

  /** `<option>`: its value attribute, else its text. `<select>`/`<input>`: what was set, else the selected option. */
  get value(): string {
    if (this.ownValue !== undefined) return this.ownValue
    if (this.tagName === 'OPTION') return this.getAttribute('value') ?? this.textContent
    if (this.tagName === 'SELECT') return this.options.find((option) => option.selected)?.value ?? this.options[0]?.value ?? ''
    return this.getAttribute('value') ?? ''
  }
  set value(next: string) {
    if (this.tagName === 'SELECT') {
      for (const option of this.options) option.selected = option.value === String(next)
      return
    }
    this.ownValue = String(next)
  }
  get options(): MiniElement[] {
    const found: MiniElement[] = []
    const walk = (node: MiniNode) => {
      for (const child of node.childNodes) {
        if (child instanceof MiniElement) {
          if (child.tagName === 'OPTION') found.push(child)
          walk(child)
        }
      }
    }
    walk(this)
    return found
  }
}

export class MiniDocument extends MiniNode {
  documentElement: MiniElement
  head: MiniElement
  body: MiniElement
  activeElement: MiniElement | null
  defaultView: any = null

  constructor() {
    super()
    this.nodeType = 9
    this.nodeName = '#document'
    this.ownerDocument = null
    this.documentElement = this.createElement('html')
    this.head = this.createElement('head')
    this.body = this.createElement('body')
    this.appendChild(this.documentElement)
    this.documentElement.appendChild(this.head)
    this.documentElement.appendChild(this.body)
    this.activeElement = this.body
  }
  private own<T extends MiniNode>(node: T): T {
    node.ownerDocument = this
    return node
  }
  createElement(tag: string) { return this.own(new MiniElement(tag)) }
  createElementNS(namespace: string, tag: string) { return this.own(new MiniElement(tag, namespace)) }
  createTextNode(text: string) { return this.own(new MiniText(text)) }
  createComment(text: string) { return this.own(new MiniComment(text)) }
  getElementById(id: string): MiniElement | null {
    const walk = (node: MiniNode): MiniElement | null => {
      for (const child of node.childNodes) {
        if (child instanceof MiniElement && child.id === id) return child
        const hit = walk(child)
        if (hit) return hit
      }
      return null
    }
    return walk(this)
  }
}

/** Every element under `root` (depth first), `root` included. */
export function allElements(root: MiniNode): MiniElement[] {
  const found: MiniElement[] = []
  const walk = (node: MiniNode) => {
    if (node instanceof MiniElement) found.push(node)
    for (const child of node.childNodes) walk(child)
  }
  walk(root)
  return found
}

/**
 * A click as a WebView delivers it: through the capture then the bubble listeners React put on the
 * root container, with the clicked node as the target (React walks the fibers from there).
 */
export function click(container: MiniNode, target: MiniElement) {
  const event = {
    type: 'click',
    target,
    srcElement: target,
    currentTarget: container,
    bubbles: true,
    cancelable: true,
    defaultPrevented: false,
    isTrusted: true,
    button: 0,
    timeStamp: Date.now(),
    preventDefault() { this.defaultPrevented = true },
    stopPropagation() {},
  }
  for (const fn of container.listenersFor('click', true)) fn(event)
  for (const fn of container.listenersFor('click', false)) fn(event)
}

export interface MiniDomInstall {
  document: MiniDocument
  window: any
  restore: () => void
}

/** Install the DOM on `globalThis`. `window` carries what the app's own code reads; `extra` adds the rest. */
export function installMiniDom(search: string, extra: Record<string, unknown> = {}): MiniDomInstall {
  const document = new MiniDocument()
  const windowListeners = new MiniNode()
  const window: any = {
    document,
    location: { search, href: `http://localhost/${search}` },
    navigator: { userAgent: 'mini-dom', clipboard: undefined },
    HTMLElement: MiniElement,
    HTMLIFrameElement: class HTMLIFrameElement {},
    Node: MiniNode,
    Element: MiniElement,
    setTimeout: (fn: () => void, ms?: number) => globalThis.setTimeout(fn, ms),
    clearTimeout: (id: ReturnType<typeof setTimeout>) => globalThis.clearTimeout(id),
    setInterval: (fn: () => void, ms?: number) => globalThis.setInterval(fn, ms),
    clearInterval: (id: ReturnType<typeof setInterval>) => globalThis.clearInterval(id),
    addEventListener: windowListeners.addEventListener.bind(windowListeners),
    removeEventListener: windowListeners.removeEventListener.bind(windowListeners),
    matchMedia: () => ({ matches: false, addEventListener() {}, removeEventListener() {} }),
    getSelection: () => null,
    open: () => null,
    ...extra,
  }
  document.defaultView = window
  const names = ['window', 'document', 'location', 'navigator', 'HTMLElement', 'HTMLIFrameElement', 'Node', 'Element', 'IS_REACT_ACT_ENVIRONMENT'] as const
  const previous = new Map<string, PropertyDescriptor | undefined>()
  for (const name of names) previous.set(name, Object.getOwnPropertyDescriptor(globalThis, name))
  const set = (name: string, value: unknown) => Object.defineProperty(globalThis, name, { value, configurable: true, writable: true })
  set('window', window)
  set('document', document)
  set('location', window.location)
  set('navigator', window.navigator)
  set('HTMLElement', MiniElement)
  set('HTMLIFrameElement', window.HTMLIFrameElement)
  set('Node', MiniNode)
  set('Element', MiniElement)
  set('IS_REACT_ACT_ENVIRONMENT', true)
  return {
    document,
    window,
    restore() {
      for (const [name, descriptor] of previous) {
        if (descriptor) Object.defineProperty(globalThis, name, descriptor)
        else delete (globalThis as any)[name]
      }
    },
  }
}
