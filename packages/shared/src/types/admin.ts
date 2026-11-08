// Shared admin types: tokens, webhooks, audit events, and role changes.
import type { components } from './openapi'
export interface PersonalAccessToken {
  id: string
  name: string
  /** Always [] from the server since task 1789: no scope is enforced, so none is claimed. */
  scopes?: string[]
  created_at: string
  last_used_at: string | null
  expires_at: string | null
}

/** Raw `token` is shown exactly once; the server never returns it again. */
export type CreateTokenResponse = components['schemas']['CreateTokenResponse']

export interface CreateTokenParams {
  name: string
  /** Whole days, 1..=366, or null for a non-expiring token. Scopes are not sent (task 1789). */
  expires_in_days: number | null
}

export interface Webhook {
  id: string
  url: string
  events: string[]
  is_active: boolean
  last_triggered_at: string | null
  failure_count: number
  created_at: string
}

export interface CreateWebhookResponse {
  id: string
  url: string
  /** Signing secret — returned once on creation, never again. */
  secret: string
  events: string[]
  is_active: boolean
}

export interface AuditEvent {
  id: string
  workspace_id: string | null
  actor: string
  event: string
  target: string | null
  ip_address: string | null
  device: string | null
  created_at: string
}

export interface WorkspaceMember {
  id: string
  email: string
  email_verified: boolean
  joined_at: string
}

export interface AdminUser {
  id: string
  email: string
  created_at: string
  email_verified: boolean
  plan: string
  storage_used_bytes: number
  // task 0750 — always emitted by GET /admin/users (server beebeeb-api/src/routes/admin.rs).
  /** Non-folder, non-trashed file count — server admin.rs:4233. */
  file_count: number
  /** Total stored chunk count — server admin.rs:4234. */
  chunk_count: number
  /** Admin kill-switch: `disabled_at IS NOT NULL` — server admin.rs:4230. */
  is_disabled: boolean
}

export interface AdminUsersResponse {
  users: AdminUser[]
  total: number
  limit: number
  offset: number
}

export interface AdminUserDetail {
  id: string
  email: string
  email_verified: boolean
  /** Top-level account role. Server adds this in `7be299a` for task 0018. */
  role: 'user' | 'admin' | 'superadmin'
  created_at: string
  plan: string
  billing_cycle: string | null
  subscription_status: string | null
  current_period_end: string | null
  storage_bytes: number
  file_count: number
  share_count: number
  session_count: number
  last_login: string | null
  workspaces: { id: string; name: string; role: string; joined_at: string }[]
  plan_limit_bytes?: number
  /** Whether the account uses OPAQUE (true) or legacy Argon2id (false/absent). */
  opaque_enrolled?: boolean
  /**
   * Backward-compat alias for `is_disabled`. Both fields reflect the same
   * `users.disabled_at` column after the task 0160 reconciliation.
   */
  is_suspended?: boolean
  /** Admin "kill switch" — when true, login returns 403 account_disabled. */
  is_disabled?: boolean
  /** ISO timestamp the kill-switch was set, or null. */
  disabled_at?: string | null
  /**
   * Optional human-readable reason captured at disable time. Currently the
   * server returns null until reasons are wired through the UI.
   */
  disabled_reason?: string | null
  /** Referral + admin-granted bonus storage on top of the plan quota. */
  bonus_storage_bytes?: number
  /** Provenance of the active subscription row (e.g. 'admin_override'); historical rows may carry any string. */
  subscription_source?: string | null
  /** Free-text justification when source = 'admin_override'. */
  subscription_admin_note?: string | null
  /** Extra TB purchased as add-ons on Pro/Business plans (0 when no add-ons). */
  extra_storage_tb?: number
  /** Billing lifecycle state: active, grace, read_only, suspended, terminated. */
  billing_state?: string | null
  /** ISO timestamp when the subscription became past-due. */
  past_due_since?: string | null
  // ── task 0750 — emitted by GET /admin/users/:id (server admin.rs json! @ 4674-4724) ──
  /** ISO timestamp the recovery PDF was downloaded, or null — server admin.rs:4711. */
  recovery_pdf_downloaded_at?: string | null
  /** ISO timestamp the recovery phrase was verified, or null — server admin.rs:4712. */
  phrase_verified_at?: string | null
  /** ISO timestamp the recovery phrase was last tested, or null — server admin.rs:4713. */
  phrase_last_tested_at?: string | null
  /** User disabled transactional email — server admin.rs:4714 (`email_notifications_disabled`). */
  email_notifications_disabled?: boolean
  /** User opted in to product analytics — server admin.rs:4715. */
  tracking_opted_in?: boolean
  /** ISO timestamp of the analytics opt-in, or null — server admin.rs:4716. */
  tracking_opted_in_at?: string | null
  /** ISO timestamp of the analytics opt-out, or null — server admin.rs:4717. */
  tracking_opted_out_at?: string | null
  /** Admin-facing free-text notes, or null — server admin.rs:4722. */
  admin_notes?: string | null
}

export interface AdminUserFile {
  id: string
  name_encrypted: string | null
  size_bytes: number
  is_folder: boolean
  is_trashed: boolean
  created_at: string
  storage_pool_id: string | null
}

export interface AdminUserFilesResponse {
  user_id: string
  files: AdminUserFile[]
  total_count: number
  total_bytes: number
}

export interface AdminUserSession {
  id: string
  created_at: string
  last_active_at: string | null
  ip_address: string | null
  user_agent: string | null
  device_hint: string | null
}

export interface WaitlistEntry {
  email: string
  source: string | null
  plan: string | null
  storage_tb: number | null
  users: number | null
  details: string | null
  created_at: string
}

export interface WaitlistResponse {
  count: number
  entries: WaitlistEntry[]
}
