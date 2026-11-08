// The app's half of the Office engine relay (B08b). The engine page (bb-office-host.html, `beebeeb-io/office`
// bridge/) is served from the `bboffice` scheme, a different origin from this window, so `frame.contentWindow.bbOffice`
// can't be read. Its relay (bridge/bb-office-relay.js) answers postMessage calls instead.
//
// Checked on every message: it comes from the engine frame's window and from the engine's origin. Calls go to that
// origin only, never "*". Bytes travel as transferable ArrayBuffers, never as base64.
import type { OfficeBridge, UnoArg, UnoCommandState } from '@beebeeb/shared/drive-ui'

type Reply = { bbOffice: 1; id: number; ok: true; value: unknown } | { bbOffice: 1; id: number; ok: false; error: string }
type EventMsg = { bbOffice: 1; event: string; payload: unknown }

export type RelayBridge = OfficeBridge & { close(): void }

/** The origin a frame URL loads from. `URL.origin` is "null" for a custom scheme (`bboffice://localhost`), so build it. */
export function originOf(url: string): string {
  const u = new URL(url)
  return `${u.protocol}//${u.host}`
}

/** A Uint8Array copied into a buffer of its own, so it can be transferred without detaching the caller's bytes. */
function ownedBuffer(bytes: Uint8Array): ArrayBuffer {
  const copy = new Uint8Array(bytes.byteLength)
  copy.set(bytes)
  return copy.buffer
}

/** Open the relay to the engine in `frame` (loaded from `engineOrigin`). Resolves once the engine takes calls. */
export function connectRelay(frame: HTMLIFrameElement, engineOrigin: string, timeoutMs = 180_000): Promise<RelayBridge> {
  let next = 1
  let closed = false
  const pending = new Map<number, { resolve: (v: unknown) => void; reject: (e: Error) => void }>()
  const listeners = new Map<string, (payload: unknown) => void>()

  const onMessage = (e: MessageEvent) => {
    if (e.origin !== engineOrigin || e.source !== frame.contentWindow) return
    const m = e.data as Reply | EventMsg | null
    if (!m || m.bbOffice !== 1) return
    if ('event' in m) {
      listeners.get(m.event)?.(m.payload)
      return
    }
    const p = pending.get(m.id)
    if (!p) return
    pending.delete(m.id)
    if (m.ok) p.resolve(m.value)
    else p.reject(new Error(m.error))
  }
  window.addEventListener('message', onMessage)

  const call = <T>(method: string, args: unknown[] = [], transfer: Transferable[] = [], onId?: (id: number) => void): Promise<T> => {
    const target = frame.contentWindow
    if (closed || !target) return Promise.reject(new Error('boot: the editor frame is gone'))
    const id = next++
    onId?.(id)
    return new Promise<T>((resolve, reject) => {
      pending.set(id, { resolve: resolve as (v: unknown) => void, reject })
      target.postMessage({ bbOffice: 1, id, method, args }, engineOrigin, transfer)
    })
  }
  const bytesOut = (v: unknown) => new Uint8Array(v as ArrayBuffer)
  const subscribe = async <P>(method: string, head: unknown[], cb: (payload: P) => void): Promise<() => void> => {
    const sub = `s${next++}`
    listeners.set(sub, cb as (payload: unknown) => void)
    await call(method, [...head, sub])
    return () => {
      listeners.delete(sub)
      void call('unsubscribe', [sub]).catch(() => {})
    }
  }

  const bridge: RelayBridge = {
    open(bytes, filename) {
      const buf = ownedBuffer(bytes)
      return call('open', [buf, filename], [buf])
    },
    save: () => call<ArrayBuffer>('save').then(bytesOut),
    insertImage(bytes, mimeType) {
      const buf = ownedBuffer(bytes)
      return call('insertImage', [buf, mimeType], [buf])
    },
    insertHyperlink: (text, url) => call('insertHyperlink', [text, url]),
    dispatch: (command: string, args?: UnoArg[]) => call('dispatch', args ? [command, args] : [command]),
    onState: (command, cb) => subscribe<UnoCommandState>('onState', [command], cb),
    onModifiedChange: (cb) => subscribe<boolean>('onModifiedChange', [], cb),
    onSelectionChange: (cb) => subscribe<{ text: string }>('onSelectionChange', [], cb),
    getOutline: () => call('getOutline'),
    getDocStats: () => call('getDocStats'),
    goToHeading: (index) => call('goToHeading', [index]),
    setZoom: (percent) => call('setZoom', [percent]),
    getSlideSize: () => call('getSlideSize'),
    newDocument(docKind, templateBytes) {
      if (!templateBytes) return call('newDocument', [docKind])
      const buf = ownedBuffer(templateBytes)
      return call('newDocument', [docKind, buf], [buf])
    },
    setTheme: (theme) => call('setTheme', [theme]),
    setWorkspaceColor: (rgb) => call('setWorkspaceColor', [rgb]),
    close() {
      closed = true
      window.removeEventListener('message', onMessage)
      for (const p of pending.values()) p.reject(new Error('boot: the editor closed'))
      pending.clear()
      listeners.clear()
    },
  }

  // The engine answers `ready` with false until its port is up; poll it, one question at a time.
  const started = Date.now()
  return new Promise<RelayBridge>((resolve, reject) => {
    const tick = () => {
      if (closed) return reject(new Error('boot: the editor closed'))
      if (Date.now() - started > timeoutMs) {
        bridge.close()
        return reject(new Error('boot: timed out'))
      }
      // A question sent before the frame's relay loaded is never answered: drop it after a second and ask again.
      let answered = false
      let askedId = 0
      const retry = setTimeout(() => {
        if (answered) return
        pending.delete(askedId)
        tick()
      }, 1000)
      call<boolean>('ready', [], [], (id) => { askedId = id }).then(
        (ready) => {
          answered = true
          clearTimeout(retry)
          if (ready) resolve(bridge)
          else setTimeout(tick, 250)
        },
        () => { answered = true; clearTimeout(retry); setTimeout(tick, 250) },
      )
    }
    tick()
  })
}
