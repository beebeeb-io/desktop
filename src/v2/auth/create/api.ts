// B11's commands (src-tauri/src/signup_v2): thin wrappers over core's signup_flow / recovery_flow. Secrets go one way,
// web view to Rust (crypto review rule 1); the only thing that comes back is DW-A39's 12 words, once, for that screen.
import { command, type CommandResult } from '../../../desktopApi'

export interface SignupPolicyView {
  code_length: number
  resend_after_secs: number
  min_length: number
  verify_word_count: number
  terms_url: string | null
  privacy_url: string | null
}

export interface PasswordMeter {
  length: number
  min_length: number
  missing: number
  strength: 'too_short' | 'fair' | 'good' | 'strong'
  hint: 'none' | 'more' | 'mix_case_and_number' | 'mix_case' | 'number'
}

export type BreachAnswer = 'clean' | 'breached' | 'blocked'

export const signupBegin = (): Promise<CommandResult<SignupPolicyView>> => command('auth_signup_begin')
export const sendCode = (email: string): Promise<CommandResult<void>> => command('auth_signup_send_code', { email })
export const verifyCode = (code: string): Promise<CommandResult<void>> => command('auth_signup_verify_code', { codeTyped: code })
export const passwordMeter = (password: string): Promise<CommandResult<PasswordMeter>> => command('auth_signup_password_meter', { password })
export const breachCheck = (password: string): Promise<CommandResult<BreachAnswer>> => command('auth_signup_breach_check', { password })
export const setPassword = (password: string, confirmation: string): Promise<CommandResult<void>> =>
  command('auth_signup_set_password', { password, confirmation })
export const phraseWords = (): Promise<CommandResult<string[]>> => command('auth_signup_phrase_words')
export const copyPhrase = (): Promise<CommandResult<void>> => command('auth_signup_phrase_copy')
export const savePhrase = (): Promise<CommandResult<boolean>> => command('auth_signup_phrase_save')
export const phraseChallenge = (): Promise<CommandResult<number[]>> => command('auth_signup_phrase_challenge')
export const confirmPhrase = (words: string[]): Promise<CommandResult<void>> => command('auth_signup_confirm_phrase', { words })
export const defaultDeviceName = (): Promise<CommandResult<string>> => command('auth_signup_default_device_name')
export const createAccount = (deviceName: string): Promise<CommandResult<void>> => command('auth_signup_create', { deviceName })
export const cancelSignup = (): Promise<CommandResult<void>> => command('auth_signup_cancel')
export const recoverWithPhrase = (email: string, words: string[]): Promise<CommandResult<SignupPolicyView>> =>
  command('auth_recover_with_phrase', { email, words })
export const recoverSetPassword = (password: string, confirmation: string): Promise<CommandResult<'signed_in' | 'sign_in_required'>> =>
  command('auth_recover_set_password', { password, confirmation })
