// A scriptable stand-in for the Tauri webview bridge, for the local Playwright checks
// (task 1683 slice 3). It is installed with `page.evaluate(tauriMockInitScript())` after
// `page.setContent` (setContent does not navigate, so `addInitScript` would not run), before the
// bundle is added. It implements exactly the part of `__TAURI_INTERNALS__` that
// `@tauri-apps/api` `invoke`, `listen` and `getCurrentWindow()` use:
//   - `invoke(cmd, args)` records `{cmd, args}` in `window.__calls`, then asks the page-side
//     handler `window.__handler(cmd, args)` (set per scenario).
//   - `plugin:event|listen` / `unlisten` keep a registry; `window.__emit(event, payload)`
//     delivers an event to every listener, the way Rust's `app.emit` does.
//   - `getCurrentWindow().hide()` is `invoke('plugin:window|hide')`, recorded like any call.
//
// "App commands" are every call that is not a `plugin:` call: the checks assert on those,
// because a `listen` registration is plumbing, not the popover talking to the backend.
export function tauriMockInitScript() {
  return `(() => {
    const calls = []
    const callbacks = new Map()
    const listeners = new Map()
    let nextCallback = 1
    let nextEvent = 1
    window.__calls = calls
    window.__listeners = listeners
    window.__handler = () => undefined
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
      unregisterListener(event, eventId) {
        const list = listeners.get(event) || []
        listeners.set(event, list.filter((l) => l.eventId !== eventId))
      },
    }
    window.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: 'popover' }, currentWebview: { windowLabel: 'popover', label: 'popover' } },
      transformCallback(cb) { const id = nextCallback++; callbacks.set(id, cb); return id },
      unregisterCallback(id) { callbacks.delete(id) },
      convertFileSrc: (path) => path,
      async invoke(cmd, args) {
        calls.push({ cmd, args: args === undefined ? null : JSON.parse(JSON.stringify(args)) })
        if (cmd === 'plugin:event|listen') {
          const eventId = nextEvent++
          const list = listeners.get(args.event) || []
          list.push({ eventId, handler: args.handler })
          listeners.set(args.event, list)
          return eventId
        }
        if (cmd === 'plugin:event|unlisten') return undefined
        if (cmd.startsWith('plugin:window|')) return undefined
        return await window.__handler(cmd, args)
      },
    }
    window.__emit = (event, payload) => {
      for (const { eventId, handler } of listeners.get(event) || []) {
        const cb = callbacks.get(handler)
        if (cb) cb({ event, id: eventId, payload })
      }
    }
    window.__appCalls = () => calls.filter((c) => !c.cmd.startsWith('plugin:'))
    window.__listenerCount = (event) => (listeners.get(event) || []).length
  })()`
}
