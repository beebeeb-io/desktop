import { invoke } from '@tauri-apps/api/core'

export type CommandResult<T> =
  | { ok: true; value: T }
  | { ok: false; reason: string; unsupported: boolean }

export interface SyncStatus {
  logged_in: boolean
  engine: string
  sync_root: string | null
  engine_running?: boolean
  vault_unlocked?: boolean
  syncing: number
  cloud_only: number
  conflicts: number
}

export type DesktopPlatform = 'macos' | 'windows' | 'linux' | 'unknown'

export interface DesktopConfig {
  upload_kbps_limit: number
  download_kbps_limit: number
  pause_sync: boolean
  notify_conflicts: boolean
  notify_sync_complete: boolean
  notify_quota_warnings: boolean
  // Windows-specific optional fields (WS1)
  sync_mode?: 'everything' | 'smart' | 'custom' | 'online_only'
  metered?: boolean
  files_on_demand?: boolean
  sync_overlays?: boolean
}

/**
 * A folder node in the selective-sync tree returned by `list_vault_folders`
 * (now a NESTED tree of folders only — no files). Each node carries its
 * online-only (excluded) state plus subtree aggregates over its files.
 *
 * - `excluded`     — this folder id is currently in the user's online-only set.
 * - `size_bytes`   — SUBTREE total of all file bytes under this folder.
 * - `file_count`   — SUBTREE total file count under this folder.
 * - `on_disk_bytes`— SUBTREE bytes currently hydrated on local disk. The
 *   online-only (not-yet-downloaded) bytes are `size_bytes - on_disk_bytes`.
 *
 * Folders carry no own bytes; the aggregates roll up their descendant files.
 */
export interface VaultItem {
  id: string
  name: string
  is_folder: boolean
  excluded: boolean
  size_bytes: number
  file_count: number
  on_disk_bytes: number
  children: VaultItem[]
  // ── Legacy UI-state fields (pre-Tauri pages: src/pages/SelectiveSync.tsx,
  // src/Onboarding.tsx, backed by list_remote_tree / set_recursive_pin). The new
  // list_vault_folders backing does NOT emit these; they're optional so the
  // legacy pin-tree pages still type-check without affecting the new selective-
  // sync view, which never reads them. ──
  path?: string
  pinned?: boolean
  inheritedPinned?: boolean
}

export interface StorageSummary {
  used_bytes: number
  quota_bytes: number
  cache_bytes: number
  pinned_bytes: number
}

export interface SharedRoot {
  id: string
  name: string
  owner_email?: string
  permission: 'read' | 'write' | 'admin'
  finder_path?: string | null
}

export interface VersionConflictEntry {
  id: string
  file_id: string
  file_name: string
  kind:
    | 'conflict'
    | 'failed_upload'
    | 'quota_failure'
    | 'permission_failure'
    | 'stale_base'
    | 'restore'
    | 'metadata'
    | 'delete'
  status: string
  updated_at?: number
  detail: string
  action: 'open_conflict' | 'review_upload' | 'restore_review' | string
  op_id?: string | null
  version_id?: string | null
  base_version?: number | null
  last_error?: string | null
}

export interface FileVersionEntry {
  id: string
  version?: number | null
  object_version_id?: string | null
  created_at?: string | number | null
  size_bytes?: number | null
  is_current?: boolean
  created_by?: string | null
}

export interface FinderInstallState {
  installed: boolean
  path?: string | null
  status?: string
  last_error?: string | null
  last_attempt_at?: number | null
  reason_category?: string | null
}

export interface MacosIntegrationResetResult {
  removed_file_provider_domain: boolean
  disabled_autostart: boolean
  removed_socket: boolean
  removed_cache_files: number
  skipped_cache_files: number
  pending_operations_preserved: number
  sync_root_preserved?: string | null
  warnings: string[]
}

export const DEFAULT_CONFIG: DesktopConfig = {
  upload_kbps_limit: 0,
  download_kbps_limit: 0,
  pause_sync: false,
  notify_conflicts: true,
  notify_sync_complete: false,
  notify_quota_warnings: true,
}

function reasonFrom(error: unknown): string {
  if (error instanceof Error) return error.message
  return String(error)
}

function isUnsupported(reason: string): boolean {
  const lower = reason.toLowerCase()
  return (
    lower.includes('unknown command') ||
    lower.includes('is not a registered') ||
    lower.includes('invoke') ||
    lower.includes('command not found') ||
    lower.includes('__tauri') ||
    lower.includes('not implemented')
  )
}

export async function command<T>(name: string, args?: Record<string, unknown>): Promise<CommandResult<T>> {
  try {
    return { ok: true, value: await invoke<T>(name, args) }
  } catch (error) {
    const reason = reasonFrom(error)
    return { ok: false, reason, unsupported: isUnsupported(reason) }
  }
}

export async function loadSyncStatus(): Promise<SyncStatus | null> {
  const result = await command<SyncStatus>('sync_status')
  return result.ok ? result.value : null
}

export async function openUrl(url: string): Promise<CommandResult<void>> {
  const opened = await command<void>('plugin:opener|open_url', { url })
  if (opened.ok) return opened
  window.open(url, '_blank', 'noopener,noreferrer')
  return opened
}

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let value = bytes
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit += 1
  }
  const digits = value >= 10 || unit === 0 ? 0 : 1
  return `${value.toFixed(digits)} ${units[unit]}`
}

export function commandUnavailableLabel(commandName: string): string {
  return `${commandName} is not wired in this build yet.`
}

// ── Account data layer (PKG-DATA, commit 39f18963) ──────────────────────────
//
// Typed wrappers + interfaces for the 15 read-mostly account/billing/devices/
// activity/notifications IPC commands registered in src-tauri/src/lib.rs.
// Shapes mirror src-tauri/src/account_dto.rs exactly (which in turn mirrors the
// server's JSON). Timestamps are RFC3339 strings on the wire — the UI formats
// them. These power the WindowsApp.tsx data views; today the views are
// scaffolded placeholders and most do not yet call these, but the wrappers are
// here so the next packages can bind data without touching the IPC boundary.

/** GET /api/v1/auth/me (cached server-side). */
export interface AccountProfile {
  user_id: string
  email: string
  email_verified: boolean
  /** RFC3339. */
  created_at: string
  /** RFC3339; non-null only for frozen / pending-deletion accounts. */
  frozen_at?: string | null
  /** "user" | "admin" | "superadmin". */
  role?: string | null
  totp_enabled?: boolean | null
  is_impersonation?: boolean | null
  admin_user_id?: string | null
}

/** GET /api/v1/billing/subscription — plan + quota + lifecycle. */
export interface Subscription {
  plan: string
  billing_cycle: string
  status: string
  seats: number
  region: string
  /** RFC3339; null on the free-plan default branch. */
  created_at?: string | null
  /** RFC3339; null on the free-plan default branch. */
  current_period_end?: string | null
  /** RFC3339; when the subscription first entered `past_due`, else null. Mirrors the Rust DTO field. */
  past_due_since?: string | null
  quota_bytes: number
  used_bytes: number
  extra_storage_tb?: number
  extra_users?: number
  is_mock?: boolean | null
  stripe_configured?: boolean | null
  billing_state?: string | null
  pending_downgrade_plan?: string | null
  downgrade_cooldown_until?: string | null
  storage_grace_deadline?: string | null
  pause_until?: string | null
}

/** GET /api/v1/billing/usage — used/quota bytes + percentage (0.0..=1.0). */
export interface BillingUsage {
  used_bytes: number
  quota_bytes: number
  percentage: number
}

export interface AccountActivityEvent {
  id: string
  /** Audit event name, e.g. "auth.login.success". Serialized as `type`. */
  type: string
  description: string
  category: string
  outcome: string
  device?: string | null
  country_code?: string | null
  /** RFC3339. */
  created_at: string
}

export interface AccountActivitySummary {
  last_login_at?: string | null
  last_login_device?: string | null
  active_sessions: number
  active_shares: number
  /** Label string ("Excellent" | "Strong" | …), NOT the numeric score. */
  security_score: string
}

/** GET /api/v1/account/activity — recent events + security summary. */
export interface AccountActivity {
  events: AccountActivityEvent[]
  summary: AccountActivitySummary
}

export interface SecurityFactor {
  /** e.g. "email_verified" | "two_factor_enabled" | … */
  key: string
  satisfied: boolean
}

/** GET /api/v1/account/security-score — numeric 0..=5 score + factors. */
export interface SecurityScore {
  score: number
  max?: number | null
  /** "Excellent" | "Strong" | "Good" | "Basic" | "Vulnerable" | "Unknown". */
  label: string
  factors: SecurityFactor[]
}

export interface AccountSession {
  id: string
  device_name: string
  device_kind: string
  country_code?: string | null
  /** RFC3339 or null. */
  last_active_at?: string | null
  /** RFC3339. */
  created_at: string
  is_current: boolean
}

/** GET /api/v1/account/sessions — active account sessions. */
export interface AccountSessionList {
  sessions: AccountSession[]
}

/** POST /api/v1/account/sessions/revoke-all-others → { revoked }. */
export interface RevokeAllResult {
  revoked: number
}

export interface ClientDevice {
  id: string
  hostname: string
  platform: string
  bb_version?: string | null
  /** RFC3339. */
  last_seen: string
  /** RFC3339. */
  created_at: string
  session_count: number
}

/** GET /api/v1/clients/devices — registered client devices. */
export interface ClientDeviceList {
  devices: ClientDevice[]
}

export interface ClientSyncSession {
  id: string
  name: string
  session_type: string
  local_path?: string | null
  remote_path: string
  status: string
  heartbeat_interval_secs?: number | null
  alert_after_missed?: number | null
  /** RFC3339. */
  created_at: string
  /** RFC3339 or null. */
  last_heartbeat?: string | null
  files_synced?: number | null
  files_total?: number | null
  bytes_synced?: number | null
  bytes_total?: number | null
  heartbeat_status?: string | null
  current_file?: string | null
  device_hostname?: string | null
  device_platform?: string | null
  device_id?: string | null
  speed_bps?: number | null
}

/** GET /api/v1/clients/sessions — per-device sync sessions. */
export interface ClientSyncSessionList {
  sessions: ClientSyncSession[]
}

export interface ActivityFeedEvent {
  id: string
  /** Serialized as `type`. */
  type: string
  subject?: string | null
  details?: string | null
  /** ip_address or device. Serialized as `where`. */
  where?: string | null
  /** RFC3339. */
  created_at: string
}

/** GET /api/v1/activity — paginated audit-log feed. */
export interface ActivityFeed {
  events: ActivityFeedEvent[]
  total: number
  page?: number | null
}

export interface Notification {
  id: string
  /** Serialized as `type`. */
  type: string
  title: string
  body?: string | null
  /** Arbitrary JSON blob (JSONB column); may be null. */
  data?: unknown
  read: boolean
  /** RFC3339. */
  created_at: string
}

/** GET /api/v1/notifications — notification list + unread count. */
export interface NotificationList {
  notifications: Notification[]
  unread_count: number
}

export interface NotificationPreferenceValues {
  file_updated: boolean
  share_received: boolean
  storage_warning: boolean
  new_device_login: boolean
  backup_complete: boolean
}

/** Both GET and PUT /preferences wrap the values under a `preferences` key. */
export interface NotificationPreferences {
  preferences: NotificationPreferenceValues
}

/** Partial update body for PUT /notifications/preferences. */
export type NotificationPreferencesUpdate = Partial<NotificationPreferenceValues>

/** A coarse content category derived from a file's extension. */
export type StorageCategory = 'media' | 'documents' | 'other'

export interface StorageCategoryTotal {
  category: StorageCategory
  bytes: number
  file_count: number
}

export interface LargestFile {
  file_id: string
  path: string
  size_bytes: number
  category: StorageCategory
}

/** Local storage breakdown by content type + the N largest files. */
export interface StorageBreakdown {
  /** Always all three categories present (zero when empty) for a stable legend. */
  by_category: StorageCategoryTotal[]
  /** Largest files, descending by size, capped at the requested limit. */
  largest_files: LargestFile[]
  total_files: number
  total_bytes: number
}

// ── Wrappers ────────────────────────────────────────────────────────────────
// Each returns a CommandResult so callers can distinguish "unsupported in this
// build" from "the server / session failed", matching the existing idiom.

export function accountProfile(): Promise<CommandResult<AccountProfile>> {
  return command<AccountProfile>('account_profile')
}

export function accountSubscription(): Promise<CommandResult<Subscription>> {
  return command<Subscription>('account_subscription')
}

export function accountUsage(): Promise<CommandResult<BillingUsage>> {
  return command<BillingUsage>('account_usage')
}

export function accountActivity(): Promise<CommandResult<AccountActivity>> {
  return command<AccountActivity>('account_activity')
}

export function accountSecurityScore(): Promise<CommandResult<SecurityScore>> {
  return command<SecurityScore>('account_security_score')
}

export function accountSessionList(): Promise<CommandResult<AccountSessionList>> {
  return command<AccountSessionList>('account_session_list')
}

export function accountRevokeSession(sessionId: string): Promise<CommandResult<void>> {
  return command<void>('account_revoke_session', { sessionId })
}

export function accountRevokeOtherSessions(): Promise<CommandResult<RevokeAllResult>> {
  return command<RevokeAllResult>('account_revoke_other_sessions')
}

export function accountDevices(): Promise<CommandResult<ClientDeviceList>> {
  return command<ClientDeviceList>('account_devices')
}

export function accountClientSessions(): Promise<CommandResult<ClientSyncSessionList>> {
  return command<ClientSyncSessionList>('account_client_sessions')
}

export function accountActivityFeed(
  page?: number,
  limit?: number,
): Promise<CommandResult<ActivityFeed>> {
  return command<ActivityFeed>('account_activity_feed', { page, limit })
}

export function accountNotifications(): Promise<CommandResult<NotificationList>> {
  return command<NotificationList>('account_notifications')
}

export function accountNotificationPreferences(): Promise<CommandResult<NotificationPreferences>> {
  return command<NotificationPreferences>('account_notification_preferences')
}

export function accountUpdateNotificationPreferences(
  update: NotificationPreferencesUpdate,
): Promise<CommandResult<NotificationPreferences>> {
  return command<NotificationPreferences>('account_update_notification_preferences', { update })
}

export function accountStorageBreakdown(
  largestLimit?: number,
): Promise<CommandResult<StorageBreakdown>> {
  return command<StorageBreakdown>('account_storage_breakdown', { largestLimit })
}

/** Open the main Beebeeb app window (Windows). Falls back gracefully if the
 *  command is not registered in this build. */
export function showMainAppWindow(): Promise<CommandResult<void>> {
  return command<void>('show_main_app_window')
}

// ── Selective sync (wave-2) ──────────────────────────────────────────────────
//
// The folder tree + the user's online-only (excluded) set. `list_vault_folders`
// returns a nested tree of VaultItem folders, each carrying its `excluded` flag
// and subtree aggregates. `set_selective_sync` persists the excluded set AND
// dehydrates newly-excluded subtrees to free local disk.

/** Nested folder tree for selective sync (folders only; carries `excluded` +
 *  subtree aggregates). */
export function listVaultFolders(): Promise<CommandResult<VaultItem[]>> {
  return command<VaultItem[]>('list_vault_folders')
}

/** The user's current online-only (excluded) folder-id set. */
export function getSelectiveSync(): Promise<CommandResult<string[]>> {
  return command<string[]>('get_selective_sync')
}

/** Persist the online-only (excluded) folder-id set. Newly-excluded subtrees
 *  are dehydrated from local disk (the files stay safe in the vault). */
export function setSelectiveSync(excluded: string[]): Promise<CommandResult<void>> {
  return command<void>('set_selective_sync', { excluded })
}
