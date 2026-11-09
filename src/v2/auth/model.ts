// B10 sign-in: the pure rules the screens share. No command, no React: tested and screenshot-driven.
// Error codes come from Rust (`auth_v2::flow_code`, core `AuthFlowError::code`); the screens show the board's words.

/** The sign-in form's error state (DW-A19 to DW-A22, DW-A63). */
export type SignInProblem =
  | { kind: 'invalid' }
  | { kind: 'locked-hint' }
  | { kind: 'offline' }
  | { kind: 'rate-limited'; seconds: number }
  | { kind: 'deleted' }
  | { kind: 'other' }

/** Sign in stays disabled until the email looks like an address and the password is not empty (DW-A02). */
export function canSubmit(email: string, password: string): boolean {
  const e = email.trim()
  if (!password || /\s/.test(e)) return false
  const at = e.indexOf('@')
  if (at <= 0 || at !== e.lastIndexOf('@')) return false
  const domain = e.slice(at + 1)
  return domain.includes('.') && !domain.startsWith('.') && !domain.endsWith('.')
}

/** The client's own count of consecutive failures on this screen; at five the lock note appears (DW-A22). */
export const LOCK_HINT_AFTER = 5

/** Map a command's error code to the form's problem. `failures` is the count including this one. */
export function problemFrom(code: string, failures: number): SignInProblem {
  if (code === 'offline') return { kind: 'offline' }
  if (code.startsWith('rate_limited')) {
    const seconds = Number(code.split(':')[1])
    return { kind: 'rate-limited', seconds: Number.isFinite(seconds) && seconds > 0 ? seconds : 30 }
  }
  if (code === 'account_deleted') return { kind: 'deleted' }
  if (code === 'invalid_credentials' || /invalid email or password/i.test(code)) {
    return failures >= LOCK_HINT_AFTER ? { kind: 'locked-hint' } : { kind: 'invalid' }
  }
  if (/network error|could not reach/i.test(code)) return { kind: 'offline' }
  return { kind: 'other' }
}

/** The board's words for each problem. */
export function problemText(p: SignInProblem): string {
  switch (p.kind) {
    case 'invalid':
    case 'locked-hint':
      return 'That email and password don’t match. Nothing was unlocked.'
    case 'offline':
      return 'You’re offline. Nothing was sent.'
    case 'rate-limited':
      return `Too many tries from this network. Wait ${p.seconds} seconds, then try again.`
    case 'deleted':
      return 'This account has been deleted. We can’t recover it.'
    case 'other':
      return 'Sign-in didn’t finish. Nothing was unlocked. Try again.'
  }
}

/** DW-A22's note: a hint, not a diagnosis (the lock is invisible on the wire). */
export const LOCK_NOTE =
  'After five wrong tries in a row, your account locks for a short while. It unlocks by itself, and we’ve emailed the account owner a link to unlock it now.'

/** DW-A07: the code as XXXX-XXXX, mono. */
export function formatDeviceCode(raw: string): string {
  const s = raw.replace(/[^a-z0-9]/gi, '').toUpperCase().slice(0, 8)
  return s.length > 4 ? `${s.slice(0, 4)}-${s.slice(4)}` : s
}

/** True for a whole XXXX-XXXX code as typed. */
export function isDeviceCode(raw: string): boolean {
  return /^[A-Z0-9]{4}-[A-Z0-9]{4}$/.test(formatDeviceCode(raw))
}

/** "4:12" for the countdown ring. */
export function countdown(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds))
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`
}

/** The requesting screen's status (Rust `DeviceRequestView.status`). */
export type DeviceStatus = 'waiting' | 'reconnecting' | 'approved' | 'signed_in' | 'denied' | 'expired' | 'keys_did_not_arrive' | 'failed'

/** The approving side's states (DW-A12 to DW-A17). */
export type ApproveStage = 'code' | 'review' | 'hello' | 'approved' | 'denied' | 'expired' | 'already-handled'

export function approveErrorText(code: string): string {
  if (code === 'request_not_found') return 'We could not find a waiting request with that code. Check it on the other device.'
  if (code === 'invalid_credentials') return 'That password isn’t right. Nothing was shared.'
  if (code === 'hello_unavailable') return 'Windows Hello isn’t set up on this PC. Set it up in Windows Settings to approve sign-ins.'
  if (code === 'hello_failed') return 'Windows Hello didn’t recognise you. Nothing was shared.'
  if (code === 'offline') return 'You’re offline. Nothing was shared.'
  return 'The approval didn’t finish. Nothing was shared.'
}

/** Two-factor: the six boxes show a single field's digits (DW-A26); eight for a backup code (DW-A28). */
export function digitsOnly(raw: string, max: number): string {
  return raw.replace(/\D/g, '').slice(0, max)
}

/** What the two-factor field got: up to six digits are a code; more (an 8-digit backup code pasted into it) go to the
 *  backup field, so they never use up a try as the first six digits of a code (#271 review). */
export function totpEntry(raw: string): { kind: 'code' | 'backup'; code: string } {
  const digits = raw.replace(/\D/g, '')
  return digits.length > 6 ? { kind: 'backup', code: digits.slice(0, 8) } : { kind: 'code', code: digits }
}

/** A recovery phrase is 12 words. */
export const PHRASE_WORDS = 12
/** Every one of the 12 boxes holds a word: an empty list is not a filled phrase (`[].every` is true; #271 review). */
export const phraseFilled = (words: readonly string[]) => words.length === PHRASE_WORDS && words.every(Boolean)

/** The 12 words: lowercase letters only, trimmed (DW-A23). Pasting 12 words into any box fills them all. */
export function normalizeWord(raw: string): string {
  return raw.toLowerCase().replace(/[^a-z]/g, '')
}

export function spreadPaste(words: string[], index: number, pasted: string): string[] {
  const parts = pasted.trim().split(/\s+/).map(normalizeWord).filter(Boolean)
  if (parts.length === 12) return parts
  const next = [...words]
  parts.forEach((w, i) => { if (index + i < 12) next[index + i] = w })
  return next
}

/** Initials for the Continue-as avatar. */
export function initials(name: string): string {
  return name.split(/\s+/).filter(Boolean).slice(0, 2).map((p) => p[0]!.toUpperCase()).join('')
}

/** A country as people read it: Rust's two-letter code ("DE") becomes its English name ("Germany", as the board says),
 *  through the platform's own list (Intl.DisplayNames). A storage or sign-in place is a country, never a city (LD-0021);
 *  a code the list does not know reads "Unknown", and anything else is shown as it came. */
export function countryName(code: string | null | undefined): string {
  const c = (code ?? '').trim()
  if (!c) return 'Unknown'
  if (!/^[A-Za-z]{2}$/.test(c)) return c
  try {
    const name = new Intl.DisplayNames(['en'], { type: 'region', fallback: 'none' }).of(c.toUpperCase())
    return name && name !== 'Unknown Region' ? name : 'Unknown'
  } catch { return 'Unknown' }
}
