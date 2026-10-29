import { checkForDesktopUpdatesNow, commandUnavailableLabel, type CommandResult, type ManualUpdateCheckResult } from '../desktopApi'
import { stateFromManualUpdateResult, type ManualUpdateCheckState } from './updateCheckViewModel'

/** One manual-check state per app webview, surviving Settings navigation. */
export function createManualUpdateCheck(check: () => Promise<CommandResult<ManualUpdateCheckResult>>) {
  let state: ManualUpdateCheckState = { kind: 'idle' }
  let generation = 0
  const listeners = new Set<() => void>()
  const publish = (next: ManualUpdateCheckState) => {
    state = next
    listeners.forEach((listener) => listener())
  }
  return {
    getSnapshot: () => state,
    subscribe: (listener: () => void) => {
      listeners.add(listener)
      return () => { listeners.delete(listener) }
    },
    reset: () => {
      generation += 1
      publish({ kind: 'idle' })
    },
    check: async () => {
      if (state.kind === 'checking') return
      const request = ++generation
      publish({ kind: 'checking' })
      try {
        const result = await check()
        if (request !== generation) return
        publish(result.ok ? stateFromManualUpdateResult(result.value) : {
          kind: 'error',
          reason: result.unsupported ? commandUnavailableLabel('check_for_updates_now') : result.reason,
        })
      } catch (error) {
        if (request !== generation) return
        publish({ kind: 'error', reason: error instanceof Error ? error.message : String(error) })
      }
    },
  }
}

export const desktopUpdateCheck = createManualUpdateCheck(checkForDesktopUpdatesNow)

/** Listen before draining the pending request: handles both existing and cold windows. */
export function connectNativeUpdateMenu(
  listen: (callback: () => void) => Promise<() => void>,
  consume: () => Promise<CommandResult<boolean>>,
  check: () => Promise<void>,
) {
  let cancelled = false
  let unlisten: (() => void) | undefined
  const drain = async () => {
    if (cancelled) return
    const result = await consume()
    // A consumed request must run even if StrictMode cleans up this listener
    // during the IPC round trip. The shared controller coalesces duplicate checks.
    if (result.ok && result.value) await check()
  }
  void listen(() => { void drain() }).then((cleanup) => {
    if (cancelled) cleanup()
    else {
      unlisten = cleanup
      void drain()
    }
  }).catch(() => {
    // Browser previews have no native event transport. Settings still works.
  })
  return () => {
    cancelled = true
    unlisten?.()
  }
}
