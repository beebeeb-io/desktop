/**
 * The sign-in flow of the Beebeeb window, kept outside React (task 1683 slice 2; the pattern of `accountSession.ts`'s
 * held sign-out sentence).
 *
 * `AccountSessionBoundary` remounts every account view when the session revision moves, and a sign-in moves it. Flow
 * state held in a component would be thrown away at exactly the moment the next step has to be shown. A module is per
 * WebView, and the Beebeeb window is destroyed on close, so this resets with the window and never leaks into another.
 */
import { IDLE_FLOW, type SignInFlow } from './signInModel'

let flow: SignInFlow = IDLE_FLOW
const listeners = new Set<() => void>()

/** The current flow: one stable object between changes (`useSyncExternalStore` needs that). */
export const getSignInFlow = (): SignInFlow => flow

export function subscribeSignInFlow(listener: () => void): () => void {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

export function setSignInFlow(next: SignInFlow): void {
  if (next === flow) return
  flow = next
  for (const listener of [...listeners]) listener()
}

/** Back to no flow. */
export function resetSignInFlow(): void {
  setSignInFlow(IDLE_FLOW)
}
