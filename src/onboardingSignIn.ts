/**
 * Pure sign-in state transitions for the macOS onboarding window
 * (`Onboarding.tsx`'s `SignInStep`), factored out of the component so the
 * password → (optional) 2FA sequence can be unit-tested without a DOM.
 *
 * Task 1521: `SignInStep` used to call `desktop_login` and advance to the
 * next onboarding step whenever the command RESOLVED (`result.ok`), without
 * ever looking at the `requires_2fa` field the Rust side returns. For a
 * 2FA-enabled account `desktop_login` resolves `Ok({ requires_2fa: true })`
 * — the password was correct, but no session was installed yet (the server
 * only handed back a short-lived partial token, held in `AppState`, not a
 * real session — see `desktop_login` / `desktop_login_2fa` in
 * `src-tauri/src/lib.rs`). The old code treated that `Ok` as "signed in" and
 * jumped straight to the recovery-phrase step, which then failed with
 * "Sign in before unlocking the vault." because `load_session_token_from_keychain`
 * found nothing — the user was NEVER asked for their TOTP code.
 *
 * `submitPassword` now surfaces `requiresTotp` explicitly so the caller MUST
 * branch on it; `submitTotpCode` completes the sign-in via `desktop_login_2fa`
 * (accepts either a 6-digit TOTP code or an 8-digit backup code — the server's
 * `/auth/2fa/verify` tries TOTP first, then backup codes, see
 * `beebeeb-api/src/routes/totp.rs::verify_totp_or_backup`).
 *
 * R8 (spec 2026-10-06): both steps also report what the sign-in BECAME (`settled`): a fresh
 * sign-in, the account already on this Mac signing in again in place, or another account
 * (`account_mismatch`, nothing changed here). The caller routes on it. A result that cannot be
 * classified is an error, never "the same account": see `settledFrom`.
 */
import { desktopLogin, desktopLogin2fa, commandUnavailableLabel, type DesktopLoginResult } from './desktopApi'

export interface SignInApi {
  desktopLogin: typeof desktopLogin
  desktopLogin2fa: typeof desktopLogin2fa
}

export const defaultSignInApi: SignInApi = { desktopLogin, desktopLogin2fa }

/** What a completed sign-in became (R8). */
export type SignInSettled =
  | { kind: 'fresh' }
  | { kind: 'reauthenticated'; vaultUnlocked: boolean }
  | { kind: 'account_mismatch'; pendingChanges: number }

/** `settledFrom`'s answer: a `SignInSettled`, or `unreadable` for a shape it cannot classify. */
export type SignInOutcome = SignInSettled | { kind: 'unreadable' }

/**
 * Shown when a sign-in finished but its result could not be read (an unknown shape from the
 * backend). The honest sentence: it does not claim anything about what did or did not change.
 */
export const SIGN_IN_OUTCOME_UNREADABLE = 'Beebeeb couldn’t read the result of that sign-in. Try signing in again.'

export type PasswordStepResult =
  | { ok: true; requiresTotp: boolean; settled: SignInSettled }
  | { ok: false; message: string }

export type TotpStepResult = { ok: true; settled: SignInSettled } | { ok: false; message: string }

const isFlag = (x: unknown) => x === undefined || x === null || typeof x === 'boolean'

/**
 * Classify what `desktop_login` / `desktop_login_2fa` returned (R8). Strict by design: this is the
 * line between "your own account signed in again" and everything else, so a value that does not
 * match the contract is `unreadable` (the caller shows an error), never the same account and never
 * an account switch with an invented pending-change count.
 *
 *  - `null`/`undefined` is a plain sign-in: `desktop_login_2fa` returned nothing before R8.
 *  - `account_mismatch` wins, and needs a whole, non-negative `pending_changes`. Claiming to be
 *    the same account at the same time is a contradiction.
 *  - `reauthenticated: true` keeps the account; the keys are claimed to be here only when
 *    `vault_unlocked` says `true`, so a missing field sends the person to the recovery phrase.
 */
export function settledFrom(value: DesktopLoginResult | null | undefined): SignInOutcome {
  const unreadable: SignInOutcome = { kind: 'unreadable' }
  if (value === null || value === undefined) return { kind: 'fresh' }
  const raw: unknown = value
  if (typeof raw !== 'object' || Array.isArray(raw)) return unreadable
  const { reauthenticated, vault_unlocked, account_mismatch } = raw as Record<string, unknown>
  if (!isFlag(reauthenticated) || !isFlag(vault_unlocked)) return unreadable
  if (account_mismatch !== undefined && account_mismatch !== null) {
    if (reauthenticated === true) return unreadable
    const pending = typeof account_mismatch === 'object' ? (account_mismatch as Record<string, unknown>).pending_changes : undefined
    if (typeof pending !== 'number' || !Number.isSafeInteger(pending) || pending < 0) return unreadable
    return { kind: 'account_mismatch', pendingChanges: pending }
  }
  if (reauthenticated === true) return { kind: 'reauthenticated', vaultUnlocked: vault_unlocked === true }
  return { kind: 'fresh' }
}

/**
 * Submit email + password. Returns `requiresTotp: true` when the server's
 * OPAQUE finish reported `requires_2fa: true` — the caller must show a
 * code-entry step and call `submitTotpCode` before the sign-in is complete.
 * Does NOT throw; network/validation/OPAQUE failures come back as
 * `{ ok: false, message }`.
 */
export async function submitPassword(
  email: string,
  password: string,
  api: Pick<SignInApi, 'desktopLogin'> = defaultSignInApi,
): Promise<PasswordStepResult> {
  const result = await api.desktopLogin(email, password)
  if (!result.ok) {
    return {
      ok: false,
      message: result.unsupported ? commandUnavailableLabel('desktop_login') : result.reason,
    }
  }
  // `requires_2fa` must be an actual boolean: a missing one used to read as "no second factor
  // needed" (task 1521's bug class), and an unreadable result is never a completed sign-in.
  const settled = settledFrom(result.value)
  if (typeof result.value?.requires_2fa !== 'boolean' || settled.kind === 'unreadable') {
    return { ok: false, message: SIGN_IN_OUTCOME_UNREADABLE }
  }
  return { ok: true, requiresTotp: result.value.requires_2fa, settled }
}

/**
 * Complete a 2FA-gated sign-in. Call only after `submitPassword` returned
 * `requiresTotp: true`. `code` is either the 6-digit TOTP code or an 8-digit
 * backup code — both are accepted by the same server field. A wrong/expired
 * code comes back as `{ ok: false, message }` and the caller should let the
 * user retry (the partial token stays valid for ~5 minutes / up to the
 * server's attempt cap).
 */
export async function submitTotpCode(
  code: string,
  api: Pick<SignInApi, 'desktopLogin2fa'> = defaultSignInApi,
): Promise<TotpStepResult> {
  const result = await api.desktopLogin2fa(code)
  if (!result.ok) {
    return {
      ok: false,
      message: result.unsupported ? commandUnavailableLabel('desktop_login_2fa') : result.reason,
    }
  }
  const settled = settledFrom(result.value)
  if (settled.kind === 'unreadable') return { ok: false, message: SIGN_IN_OUTCOME_UNREADABLE }
  return { ok: true, settled }
}
