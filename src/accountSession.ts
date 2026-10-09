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

// A sign-out that happened but could not confirm a step (FB-24 row 12) leaves Rust's sentence here, per
// WebView and outside AccountSessionBoundary on purpose: the sign-out moves the revision above, the boundary
// then remounts every account view, and a sentence held in component state would be gone within a second.
// While a sentence is held, the window remounts on the account surface that shows it. The sentence is
// cleared by the next account action on that surface, or by the next sign-in; never by the remount.
let signOutWarning: { sentence: string; signedOutAt: number | null } | null = null
const signOutWarningListeners = new Set<() => void>()
function emitSignOutWarning() {
  for (const listener of signOutWarningListeners) listener()
}
export const heldSignOutWarning = (): string | null => signOutWarning?.sentence ?? null
export function subscribeSignOutWarning(listener: () => void): () => void {
  signOutWarningListeners.add(listener)
  return () => { signOutWarningListeners.delete(listener) }
}
/** After a sign-out's `Ok`: its warning sentence, or `null` when every step was confirmed. */
export function holdSignOutWarning(sentence: string | null): void {
  signOutWarning = sentence ? { sentence, signedOutAt: null } : null
  emitSignOutWarning()
}
export function clearSignOutWarning(): void {
  if (signOutWarning === null) return
  signOutWarning = null
  emitSignOutWarning()
}
/**
 * Every status read, before its revision is observed. A read that finds the account signed out records the
 * revision it was signed out at; a later read that finds it signed in at a newer revision is the next sign-in,
 * and clears the sentence before the boundary remounts for it. A read that began before the sign-out (signed
 * in, at the old revision) clears nothing.
 */
export function observeSignOutWarning(loggedIn: boolean, revision: number): void {
  if (signOutWarning === null) return
  if (!loggedIn) {
    signOutWarning.signedOutAt = Math.max(signOutWarning.signedOutAt ?? revision, revision)
    return
  }
  if (signOutWarning.signedOutAt !== null && revision > signOutWarning.signedOutAt) clearSignOutWarning()
}
