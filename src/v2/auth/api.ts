// B10's commands. Thin wrappers over `command()`: no key, token or URL comes back (crypto review rule 1); errors are
// codes the screens map to the board's words (model.ts).
import { command, type CommandResult } from '../../desktopApi'
import type { DeviceStatus } from './model'

/** `desktop_login`'s flags (src-tauri login_commands.rs `LoginOutcome`). */
export interface LoginOutcome {
  requires_2fa: boolean
  reauthenticated: boolean
  vault_unlocked: boolean
  key_replaced: boolean
  account_mismatch: { pending_changes: number } | null
}

export interface DeviceRequestView {
  status: DeviceStatus
  user_code: string | null
  seconds_left: number
  device_name: string | null
  /** SVG markup drawn in Rust; the link it encodes carries no code. */
  qr_svg?: string
}

export interface ApprovalReview {
  code: string
  country: string | null
  ip: string
  requested_at: string
  expires_at: string
  seconds_left: number
  device_name: string | null
  client: string | null
  os: string | null
}

export const signIn = (email: string, password: string): Promise<CommandResult<LoginOutcome>> =>
  command('desktop_login', { email, password })
export const verifySecondFactor = (code: string): Promise<CommandResult<LoginOutcome>> => command('desktop_login_2fa', { code })
export const lastSignedInEmail = (): Promise<CommandResult<string | null>> => command('last_signed_in_email')
export const deviceRequestStart = (): Promise<CommandResult<DeviceRequestView>> => command('auth_device_request_start')
export const deviceRequestPoll = (): Promise<CommandResult<DeviceRequestView>> => command('auth_device_request_poll')
export const deviceRequestCancel = (): Promise<CommandResult<void>> => command('auth_device_request_cancel')
export const approveLookup = (code: string): Promise<CommandResult<ApprovalReview>> => command('auth_approve_lookup', { code })
export const approveRequest = (code: string, password: string, approve: boolean): Promise<CommandResult<{ status: string }>> =>
  command('auth_approve_request', { code, password, approve })
/** B15's `passkey_commands::auth_passkey_sign_in`: `vault_unlocked` false means a session only, the password follows (DW-A30). */
export const passkeySignIn = (email: string): Promise<CommandResult<LoginOutcome>> => command('auth_passkey_sign_in', { email })
export const windowsHelloAvailable = (): Promise<CommandResult<boolean>> => command('auth_windows_hello_available')
export const unlockWithWindowsHello = (): Promise<CommandResult<void>> => command('auth_unlock_with_windows_hello')
export const unknownPhraseWords = (words: string[]): Promise<CommandResult<number[]>> => command('auth_phrase_unknown_words', { words })
export const restoreWithPhrase = (recoveryPhrase: string): Promise<CommandResult<void>> =>
  command('desktop_unlock_with_recovery_phrase', { recoveryPhrase })

// ── Task 2445: auth rules in Rust (auth_v2::gate) ──────────────────────────

export interface SignInProblemDto {
  problem: import('./model').SignInProblem
  text: string
  lock_note?: string
  wait_seconds?: number
}

/** Map a sign-in error code to the board's problem and text. The decision lives in Rust. */
export const signInProblem = (code: string, consecutiveFailures: number): Promise<CommandResult<SignInProblemDto>> =>
  command('auth_sign_in_problem', { code, consecutiveFailures })

/** What screen follows a sign-in. The decision lives in Rust (auth_v2::gate::next_after_sign_in).
 *  Keys are camelCase (Tauri's invoke convention for snake_case Rust params). */
export const nextAfterSignIn = (outcome: LoginOutcome): Promise<CommandResult<string>> =>
  command('auth_next_after_sign_in', {
    requires2fa: outcome.requires_2fa,
    vaultUnlocked: outcome.vault_unlocked,
    reauthenticated: outcome.reauthenticated,
    accountMismatch: outcome.account_mismatch !== null,
  })

/** The board's words for a 2FA verify error. */
export const twoFactorError = (code: string): Promise<CommandResult<string>> =>
  command('auth_two_factor_error', { code })

/** The board's words for a recovery-phrase error. */
export const phraseError = (code: string): Promise<CommandResult<string>> =>
  command('auth_phrase_error', { code })
