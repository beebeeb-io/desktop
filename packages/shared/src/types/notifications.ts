// Shared notification and sign-in-history types.
import type { ActivityEncryptedNameSnapshot } from './account'

export interface OnboardingState {
  /** User has verified their recovery phrase. */
  phrase_verified: boolean
  /** Server has a welcome file record (onboarding step 6 complete). */
  welcome_file_exists: boolean
  /** User has uploaded at least one non-welcome file. */
  first_upload_done: boolean
  /** User has created at least one active share link. */
  first_share_done: boolean
}

export interface TrackingPreference {
  tracking_opted_in: boolean
  opted_in_at: string | null
  opted_out_at: string | null
}

export interface MySignIn {
  at: string
  ip_anonymized: string | null
  user_agent: string | null
  country_code: string | null
  success: boolean
}

export interface MySignInsResponse {
  opted_in: boolean
  sign_ins: MySignIn[]
}

export interface AdminSignIn {
  id: string
  at: string
  /** Anonymized per data minimization — last octet zeroed for IPv4. */
  ip_anonymized: string | null
  /** Parsed UA string, e.g. "Chrome 124 on macOS". */
  device: string | null
  country_code: string | null
  success: boolean
}

export interface AdminSignInsResponse {
  opted_in: boolean
  sign_ins: AdminSignIn[]
}

export interface AdminActivityEvent {
  id: string
  at: string
  event_type: 'file_uploaded' | 'file_deleted' | 'file_downloaded' | 'share_created' | 'share_revoked' | 'folder_created' | string
  description: string
}

export interface AdminActivityResponse {
  opted_in: boolean
  events: AdminActivityEvent[]
}

/** One event from `GET /api/v1/me/activity` (tracking.rs `get_my_activity`).
 *  Shape: { id, type, description, category, outcome, device, country_code,
 *  created_at } — NOT the subject/details/where shape of `ActivityEvent`
 *  (which belongs to `/api/v1/activity`). */
export interface MyActivityEvent {
  id: string
  type: string
  description: string
  category: string
  outcome: string
  device: string | null
  country_code: string | null
  encrypted_name_snapshot?: ActivityEncryptedNameSnapshot | null
  created_at: string
}

export interface MyActivityResponse {
  opted_in: boolean
  events: MyActivityEvent[]
}

/** Per-channel toggle for a single notification type. */
export interface NotificationChannel {
  in_app: boolean
  email: boolean
}

/**
 * Expanded notification preferences — each type has per-channel toggles.
 *
 * Security types (new_device_login, password_changed, two_fa_changes,
 * recovery_phrase_used) always have email=true enforced client-side.
 */
export interface NotificationPreferences {
  // Security
  new_device_login: NotificationChannel
  password_changed: NotificationChannel
  two_fa_changes: NotificationChannel
  recovery_phrase_used: NotificationChannel
  // Sharing
  share_received: NotificationChannel
  share_link_opened: NotificationChannel
  share_access_revoked: NotificationChannel
  // Storage
  storage_warning: NotificationChannel
  storage_critical: NotificationChannel
  storage_full: NotificationChannel
  // Backup
  backup_complete: NotificationChannel
  backup_failed: NotificationChannel
  // Account
  payment_failed: NotificationChannel
  plan_expiring: NotificationChannel
  data_export_ready: NotificationChannel
}

/** Keys that belong to the security section — email is always on. */
export type SecurityNotificationType =
  | 'new_device_login'
  | 'password_changed'
  | 'two_fa_changes'
  | 'recovery_phrase_used'

/** All valid notification type keys. */
export type NotificationType = keyof NotificationPreferences

/**
 * Legacy flat boolean preferences (pre-expansion).
 * Used only for migration on first load.
 */
export interface LegacyNotificationPreferences {
  share_received: boolean
  storage_warning: boolean
  new_device_login: boolean
  backup_complete: boolean
}

export interface DataExportRequest {
  export_id: string
  status: 'pending' | 'processing' | 'ready' | 'failed' | string
  estimated_seconds?: number
  /** Server returns `resumed: true` when an existing export job is reused
   * instead of creating a new one. When resumed, the response also carries
   * any already-populated status fields below. */
  resumed?: boolean
  file_count?: number
  total_bytes?: number
  download_url?: string
  expires_at?: string
}

export interface DataExportStatus {
  export_id: string
  status: 'pending' | 'processing' | 'ready' | 'failed' | string
  file_count?: number
  total_bytes?: number
  download_url?: string
  expires_at?: string
}

export interface AvailableRegion {
  continent: string
  display_name: string
  /** Representative COUNTRY, e.g. "Germany" (task 2050). Absent on a server
   *  older than 2050. The server still emits a deprecated `example_city` for
   *  one release; it is deliberately not typed here, so nothing can render it. */
  example_country?: string
  /** No `provider` field — the brand rule (docs/canon/data-residency.md) is
   *  name the city, never the hosting provider, in user-facing copy. The
   *  server's `RegionInfo.provider` is `#[serde(skip_serializing)]` (task
   *  1367) so `/api/v1/regions` never sends one. */
  is_default: boolean
}


export interface UserRegionResponse {
  preferred_region: string | null
  /** "Only store in <country>" (task 1952). Absent on a server older than 1952. */
  region_strict?: boolean
  /** Server (regions.rs `get_preferred_region`) returns the array under
   *  `available_regions`, not `regions`. */
  available_regions: AvailableRegion[]
}
