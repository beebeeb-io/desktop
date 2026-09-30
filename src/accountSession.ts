// One revision per WebView, observed from native status. It changes even if a
// complete sign-out/sign-in happens between polls (including the same account).
let revision: number | null = null
const listeners = new Set<() => void>()
export const accountSessionRevision = () => revision
export function subscribeAccountSession(listener: () => void): () => void {
  listeners.add(listener)
  return () => { listeners.delete(listener) }
}
export function observeAccountSession(next: number): void {
  if (next === revision) return
  revision = next
  for (const listener of listeners) listener()
}
