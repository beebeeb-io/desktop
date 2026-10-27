/**
 * Task 2243: a poll that reads only while its window is on screen.
 *
 * The tray flyout stays loaded while it is hidden, which is almost always, so its pollers kept reading the state DB
 * every few seconds for nobody to see. On Guus's PC each read opened the DB cold: about 1,100 page reads for one
 * `sync_status`, about 5,500 for the file overview. A tick is skipped while the window is hidden, and showing the
 * window again reads at once, so a surface is never a tick stale when it appears.
 *
 * Tauri's `hide()` hides the native window and leaves the webview's own visibility alone, so `document.hidden` is not
 * enough on its own: the window is asked too. Outside Tauri (a plain browser, a test) there is no window to ask, and
 * the poll reads on every tick, as it did before.
 */
import { getCurrentWindow } from '@tauri-apps/api/window'

/** What a poll needs from its window: whether it is on screen, and a call when it comes back. */
export interface PollWindow {
  isVisible(): Promise<boolean>
  /** Calls `listener` when the window is shown or focused again; resolves to the unsubscribe. */
  onShown(listener: () => void): Promise<() => void>
}

/** The current Tauri window as a {@link PollWindow}, or `null` outside Tauri. */
export function currentPollWindow(): PollWindow | null {
  let win: ReturnType<typeof getCurrentWindow>
  try {
    win = getCurrentWindow()
  } catch {
    return null
  }
  return {
    isVisible: () => win.isVisible(),
    async onShown(listener) {
      const onVisibility = () => {
        if (document.visibilityState === 'visible') listener()
      }
      const unlistenFocus = await win.onFocusChanged(({ payload: focused }) => {
        if (focused) listener()
      })
      if (typeof document !== 'undefined') document.addEventListener('visibilitychange', onVisibility)
      return () => {
        unlistenFocus()
        if (typeof document !== 'undefined') document.removeEventListener('visibilitychange', onVisibility)
      }
    },
  }
}

/**
 * Whether `win` is on screen. A window that cannot answer counts as on screen: a surface that reads too often is
 * better than one that freezes.
 */
export async function windowVisible(win: PollWindow | null): Promise<boolean> {
  if (typeof document !== 'undefined' && document.visibilityState === 'hidden') return false
  if (!win) return true
  try {
    return await win.isVisible()
  } catch {
    return true
  }
}

/**
 * Run `tick` now, then every `intervalMs` while `win` is on screen, and once each time it is shown again. Returns the
 * stop function for an effect's cleanup. The first read is unconditional: a surface needs one answer to render.
 */
export function pollWhileVisible(
  tick: () => void | Promise<void>,
  intervalMs: number,
  win: PollWindow | null = currentPollWindow(),
): () => void {
  let stopped = false
  let unlisten: (() => void) | undefined
  const tickIfVisible = async () => {
    if (stopped || !(await windowVisible(win)) || stopped) return
    await tick()
  }
  void tick()
  const id = window.setInterval(tickIfVisible, intervalMs)
  void win
    ?.onShown(() => {
      if (!stopped) void tick()
    })
    .then((fn) => {
      if (stopped) fn()
      else unlisten = fn
    })
    .catch(() => undefined)
  return () => {
    stopped = true
    window.clearInterval(id)
    unlisten?.()
  }
}
