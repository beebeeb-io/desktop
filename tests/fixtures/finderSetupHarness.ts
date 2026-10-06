/**
 * What a test needs to mount a macOS Finder surface that takes the reconciler from the shared
 * `useFinderSetup` hook (lead ruling 7b): a scripted stand-in for the Tauri event bus, and the REAL
 * hook declaration bound into the component under test, so the surface and the hook share one
 * state store as they do in React. Mount with `expand: true`: the harness keeps a callback stable
 * only then, and the hook's effect depends on one.
 *
 * What it proves: the surface's real decisions for a scripted order of answers and events.
 * What it does not: React scheduling, or a real Tauri event loop (tests/finderSetup.test.ts pins
 * the subscription's own wiring).
 */
import { commandUnavailableLabel } from '../../src/desktopApi'
import * as finderSetup from '../../src/finderSetup'
import * as copy from '../../src/finderSetupCopy'
import type { FinderSetupView } from '../../src/finderSetup'
import type { HookModule } from './componentHarness'

export const finderView = (over: Partial<FinderSetupView> = {}): FinderSetupView => ({
  setup: 'adding',
  reason: null,
  launch_location: 'applications',
  attempt: 1,
  max_attempts: 0,
  last_failure: null,
  ...over,
})

/**
 * A stand-in for `subscribeFinderSetup`: the registration lands on the next microtask, as the real
 * one does. `registered` counts every subscription ever made, `live` those still open, and `emit`
 * is a reconciler transition arriving as a `finder-setup-changed` event.
 */
export function finderBus() {
  const listeners: Array<(view: FinderSetupView) => void> = []
  let registered = 0
  return {
    get live() { return listeners.length },
    get registered() { return registered },
    subscribeFinderSetup(onView: (view: FinderSetupView) => void, options: { onSubscribed?: () => void } = {}) {
      registered += 1
      listeners.push(onView)
      void Promise.resolve().then(() => options.onSubscribed?.())
      return () => {
        const at = listeners.indexOf(onView)
        if (at >= 0) listeners.splice(at, 1)
      }
    },
    emit(view: FinderSetupView) { for (const listener of [...listeners]) listener(view) },
  }
}

/** The REAL `useFinderSetup`, for the harness's `hookModules`. */
export function useFinderSetupModule(bus: ReturnType<typeof finderBus>, copied: string[] = []): HookModule {
  return {
    file: 'finderSetup.ts',
    name: 'useFinderSetup',
    bindings: {
      loadFinderSetup: finderSetup.loadFinderSetup,
      runFinderSetupAction: (action: copy.FinderSetupAction) =>
        finderSetup.runFinderSetupAction(action, { writeClipboard: async (text) => { copied.push(text) } }),
      finderSetupLoadPresentation: copy.finderSetupLoadPresentation,
      FINDER_ACTION_FAILED: copy.FINDER_ACTION_FAILED,
      FINDER_ACTION_COMMAND: finderSetup.FINDER_ACTION_COMMAND,
      commandUnavailableLabel,
      subscribeFinderSetup: bus.subscribeFinderSetup,
    },
  }
}

export const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 0))
