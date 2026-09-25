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
 */
import { desktopLogin, desktopLogin2fa, commandUnavailableLabel } from './desktopApi'

export interface SignInApi {
  desktopLogin: typeof desktopLogin
  desktopLogin2fa: typeof desktopLogin2fa
}

export const defaultSignInApi: SignInApi = { desktopLogin, desktopLogin2fa }

export type PasswordStepResult =
  | { ok: true; requiresTotp: boolean }
  | { ok: false; message: string }

export type TotpStepResult = { ok: true } | { ok: false; message: string }

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
  return { ok: true, requiresTotp: result.value.requires_2fa }
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
  return { ok: true }
}
