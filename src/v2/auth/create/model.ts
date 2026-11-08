// Pure helpers for B11's create-account and recovery screens: the board's words for each error code.

export type CreateStep = 'email' | 'verify' | 'terms' | 'password' | 'phrase' | 'confirm' | 'device' | 'creating' | 'done'

/** The progress line's six steps (DW-A31): email, verify, terms, password, phrase, this device. */
export const STEP_NUMBER: Partial<Record<CreateStep, number>> = { email: 1, verify: 2, terms: 3, password: 4, phrase: 5, confirm: 5, device: 6 }

export const EMAIL_SENT_LINE = 'If this address can be used, we sent an email.'
export const NEW_CODE_LINE = 'If this address can be used, a new email is on its way.'
export const WRONG_CODE = 'That code is not right, or it has expired.'
export const TOO_MANY_CODES = 'Too many wrong codes. Ask for a new one.'
export const BREACHED = 'This password appears in known data breaches. Choose a different one.'
export const BREACH_BLOCKED = 'We could not check this password against known breaches, and we do not let you continue without that check. Try again.'
export const WORDS_MISMATCH = 'Those words do not match your recovery phrase.'
export const CREATE_FAILED = 'We could not create your account. Nothing was stored. You can try again.'
export const FINISH_FAILED = 'Your account was created. We could not finish setting it up on this device. Nothing is lost.'
export const TICKET_EXPIRED = 'Your email code expired before the account was created. Request a new one. Your password and recovery phrase are kept.'

/** DW-A31's errors. */
export function emailErrorText(reason: string): string {
  if (reason === 'invalid_email') return 'Enter a valid email address.'
  if (reason.startsWith('rate_limited')) return 'Too many requests for this address. Try again later.'
  return 'We could not send the email. Check your connection and try again.'
}

export function isEmail(raw: string): boolean {
  const e = raw.trim()
  const at = e.indexOf('@')
  if (at < 1 || /\s/.test(e)) return false
  const domain = e.slice(at + 1)
  return domain.includes('.') && !domain.startsWith('.') && !domain.endsWith('.')
}

/** "0:42" */
export function clock(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds))
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`
}

export const STRENGTH_LABEL: Record<string, string> = { too_short: 'Weak', fair: 'Weak', good: 'Good', strong: 'Strong' }
export const STRENGTH_SEGMENTS: Record<string, number> = { too_short: 1, fair: 2, good: 3, strong: 4 }

export function hintText(hint: string): string | null {
  if (hint === 'mix_case_and_number') return 'Mix upper and lowercase, and add a number or symbol.'
  if (hint === 'mix_case') return 'Mix upper and lowercase.'
  if (hint === 'number') return 'Add a number or symbol.'
  return null
}

/** "Needs at least 12 characters, 2 more." */
export function needsMoreText(min: number, missing: number): string {
  return `Needs at least ${min} characters, ${missing} more.`
}

/** DW-A47/A48's errors. */
export function recoverErrorText(reason: string): string {
  if (reason.startsWith('rate_limited')) return 'Too many attempts. For security, wait an hour and try again.'
  if (reason === 'offline') return 'We could not reach Beebeeb. Check your connection and try again.'
  return 'That phrase did not verify. Nothing was changed: your password is as it was.'
}
