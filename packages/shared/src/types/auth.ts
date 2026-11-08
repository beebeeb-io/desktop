/**
 * Shared TypeScript interfaces copied from repos/web/src/lib/api.ts.
 *
 * These describe server JSON contracts consumed by web, admin, and future
 * Beebeeb clients. Keep this file dependency-free so type-only imports never
 * pull in React or app-local runtime code.
 */
import type { components } from './openapi'

export type ErrorNotifier = (message: string) => void

export type SessionExpiredHandler = () => void

export type ConnectionStatusHandler = (status: 'ok' | 'flaky') => void

export interface AuthSessionResponse {
  user_id: string
  session_token: string
  salt: string
}

export interface AuthUser {
  user_id: string
  email: string
  email_verified: boolean
  created_at: string
  frozen_at?: string | null
  /** Account role: `user`, `admin`, or `superadmin`. Surfaced so the web app
   *  can show an Admin sidebar link without probing /admin/stats (which
   *  CORS-rejects from the user-app origin and triggers spurious 401s). */
  role?: 'user' | 'admin' | 'superadmin' | string
  totp_enabled: boolean
  two_factor_enabled?: boolean
  twoFactorEnabled?: boolean
  /** True when the current session was created via admin impersonation. */
  is_impersonation?: boolean
  /** UUID of the admin who issued the impersonation, when `is_impersonation`. */
  admin_user_id?: string | null
}

export interface SignupResult {
  session_token: string
  salt: string
  user_id: string
}

export interface LoginResult {
  requires_2fa?: boolean
  partial_token?: string
  user_id?: string
  session_token?: string
  salt?: string
}

export interface TotpSetupResponse {
  secret: string
  qr_uri: string
  backup_codes: string[]
}

/** One stored chunk of a file's download version (server `version_download.rs`). */
export type DownloadVersionChunk = components['schemas']['DownloadChunk']

/**
 * The exact object version a file's content currently lives in (LD-0001).
 * Optional on the row because older server responses do not carry it.
 */
export type DownloadVersion = components['schemas']['DownloadVersion']
