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
export type ReleaseChannel = 'stable' | 'beta' | 'alpha'
export type DesktopTheme = 'light' | 'dark' | 'system'

export type ManualUpdateCheckResult =
  | {
      status: 'up_to_date'
      current_version: string
      channel: ReleaseChannel
    }
  | {
      status: 'update_available'
      current_version: string
      channel: ReleaseChannel
      version: string
      body: string
      release_notes_url: string
    }
  | {
      status: 'downgrade_available'
      current_version: string
      current_channel: ReleaseChannel
      channel: ReleaseChannel
      version: string
      body: string
      release_notes_url: string
    }

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
  theme?: DesktopTheme
  local_cache_limit_bytes?: number
  release_channel?: ReleaseChannel
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

export interface AppActivitySnapshot {
  pid: number
  process_name: string
  cpu_percent: number
  memory_rss_bytes: number
  upload_bps?: number | null
  download_bps?: number | null
  throughput_sample_age_ms?: number | null
  throughput_period_secs?: number | null
}

export interface SharedRoot {
  id: string
  name: string
  owner_email?: string
  permission: 'read' | 'write' | 'admin'
  kind: 'file' | 'folder'
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
  version_number: number | null
  object_version_id?: string | null
  file_version_id?: string | null
  source?: string | null
  created_at?: string | number | null
  size_bytes?: number | null
  chunk_count?: number | null
  chunk_size_bytes?: number | null
  storage_pool_id?: string | null
  restorable?: boolean
  is_current: boolean
  created_by?: string | null
  uploaded_by?: string | null
}

export interface FileVersionListResponse {
  file_id: string | null
  current_version: number | null
  versions: FileVersionEntry[]
}

export interface RestoreFileVersionQueuedResponse {
  status: 'queued'
  file_id: string
  version_id: string
  operation_id: string
  operation: 'restore_version'
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
  theme: 'system',
  local_cache_limit_bytes: 0,
  release_channel: 'stable',
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

type RawObject = Record<string, unknown>

function asObject(value: unknown): RawObject | null {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as RawObject) : null
}

function asString(value: unknown): string | null {
  return typeof value === 'string' && value.length > 0 ? value : null
}

function asNumber(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

function asBoolean(value: unknown): boolean | null {
  return typeof value === 'boolean' ? value : null
}

function normalizeFileVersion(value: unknown, currentVersion: number | null): FileVersionEntry | null {
  const raw = asObject(value)
  if (!raw) return null
  const id =
    asString(raw.id) ??
    asString(raw.version_id) ??
    asString(raw.object_version_id) ??
    asString(raw.objectVersionId) ??
    asString(raw.file_version_id) ??
    asString(raw.fileVersionId)
  if (!id) return null

  const versionNumber = asNumber(raw.version_number) ?? asNumber(raw.versionNumber) ?? asNumber(raw.version)
  const createdAt = asString(raw.created_at) ?? asString(raw.createdAt) ?? asNumber(raw.created_at)
  const restorable = asBoolean(raw.restorable)
  const explicitCurrent = raw.is_current === true || raw.current === true
  const isCurrent = explicitCurrent || (currentVersion != null && versionNumber === currentVersion)

  return {
    id,
    version_number: versionNumber,
    object_version_id: asString(raw.object_version_id) ?? asString(raw.objectVersionId) ?? null,
    file_version_id: asString(raw.file_version_id) ?? asString(raw.fileVersionId) ?? null,
    source: asString(raw.source),
    created_at: createdAt,
    size_bytes: asNumber(raw.size_bytes) ?? asNumber(raw.sizeBytes),
    chunk_count: asNumber(raw.chunk_count) ?? asNumber(raw.chunkCount),
    chunk_size_bytes: asNumber(raw.chunk_size_bytes) ?? asNumber(raw.chunkSizeBytes),
    storage_pool_id: asString(raw.storage_pool_id) ?? asString(raw.storagePoolId),
    created_by: asString(raw.created_by) ?? asString(raw.createdBy),
    uploaded_by: asString(raw.uploaded_by) ?? asString(raw.uploadedBy),
    restorable: restorable ?? true,
    is_current: isCurrent,
  }
}

function timestampSortValue(value: string | number | null | undefined): number {
  if (typeof value === 'number') return value > 10_000_000_000 ? value : value * 1000
  if (typeof value === 'string') {
    const parsed = Date.parse(value)
    return Number.isNaN(parsed) ? 0 : parsed
  }
  return 0
}

export function normalizeFileVersionListResponse(value: unknown): FileVersionListResponse {
  const raw = asObject(value)
  const currentVersion = asNumber(raw?.current_version) ?? asNumber(raw?.currentVersion)
  const list = Array.isArray(value)
    ? value
    : Array.isArray(raw?.versions)
      ? raw.versions
      : Array.isArray(raw?.file_versions)
        ? raw.file_versions
        : []
  const versions = list
    .map((version) => normalizeFileVersion(version, currentVersion))
    .filter((version): version is FileVersionEntry => version !== null)
    .sort((a, b) => {
      const versionDelta = (b.version_number ?? Number.NEGATIVE_INFINITY) - (a.version_number ?? Number.NEGATIVE_INFINITY)
      if (versionDelta !== 0) return versionDelta
      return timestampSortValue(b.created_at) - timestampSortValue(a.created_at)
    })

  return {
    file_id: asString(raw?.file_id) ?? asString(raw?.fileId),
    current_version: currentVersion,
    versions,
  }
}

export function restorableFileVersions(versions: FileVersionEntry[]): FileVersionEntry[] {
  return versions.filter((version) => !version.is_current && version.restorable !== false)
}

export function restoreVersionId(version: FileVersionEntry): string {
  return version.id
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

export async function checkForDesktopUpdatesNow(): Promise<CommandResult<ManualUpdateCheckResult>> {
  return command<ManualUpdateCheckResult>('check_for_updates_now')
}

export async function appActivitySnapshot(): Promise<CommandResult<AppActivitySnapshot>> {
  return command<AppActivitySnapshot>('app_activity_snapshot')
}

export async function installDesktopChannelDowngrade(version: string): Promise<CommandResult<void>> {
  return command<void>('install_channel_downgrade', { version })
}

export function formatBytes(bytes: number): string {
  if (bytes == null || !Number.isFinite(bytes) || bytes < 0) return '—'
  if (bytes < 1_000) return `${bytes} B`
  if (bytes < 1_000_000) return `${(bytes / 1_000).toFixed(0)} KB`
  if (bytes < 1_000_000_000) return `${(bytes / 1_000_000).toFixed(0)} MB`
  if (bytes < 1_000_000_000_000) return `${(bytes / 1_000_000_000).toFixed(0)} GB`
  return `${(bytes / 1_000_000_000_000).toFixed(1)} TB`
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

// ── File overview — per-PC sync-state lens (in-app Files tab) ─────────────────
// Computed LOCALLY from the desktop SQLite mirror (no server endpoint), this is
// the device sync-state lens that complements StorageBreakdown (the by-type /
// billing lens). Mirrors src-tauri/src/account_dto.rs::FileOverview.

/** Possible sync statuses — matches `state_db::FileStatus::as_str`. */
export type FileSyncStatus =
  | 'cloud_only'
  | 'local'
  | 'downloading'
  | 'uploading'
  | 'conflict'
  | 'error'

/** One sync-status bucket. `status` is a {@link FileSyncStatus} string; an
 *  unknown value is tolerated (forward-compat) but treated as `error`-like. */
export interface StatusBucket {
  status: string
  count: number
  bytes: number
}

/** One recently-changed file. `modified_at` is seconds since the Unix epoch. */
export interface RecentFile {
  path: string
  size_bytes: number
  status: string
  pinned: boolean
  modified_at: number
  activity_type?: 'moved_to_trash'
}

/** Per-PC sync-state overview for the Files tab. */
export interface FileOverview {
  /** Buckets for statuses that have files, descending by count. Statuses with
   *  zero files are omitted; the UI renders a fixed legend and treats a missing
   *  status as zero. */
  by_status: StatusBucket[]
  /** Most-recently-changed files, descending by `modified_at`, capped. */
  recent: RecentFile[]
  total_files: number
  total_bytes: number
  /** Count of effectively-pinned files (cuts across status buckets). */
  pinned_count: number
  /** Sum of effectively-pinned file sizes. */
  pinned_bytes: number
}

export type DesktopSearchIndexState = 'ready' | 'syncing'

export interface DesktopSearchResult {
  file_id: string
  name: string
  rel_path: string
  status: string
  size_bytes: number
  modified_at: number
}

export interface DesktopSearchResponse {
  query: string
  index_state: DesktopSearchIndexState
  indexed_file_count: number
  results: DesktopSearchResult[]
}

export interface DesktopTrashItem {
  id: string
  name: string
  is_folder: boolean
  size_bytes: number
  updated_at: string
  parent_id?: string | null
}

export interface ConfirmActionResult {
  confirmation_token: string
  expires_at: string
}

// ── Data residency / region — GET /api/v1/me/region ──────────────────────────
// Mirrors the webapp `@beebeeb/shared` shapes so the desktop resolves the
// effective region's CITY the same way the webapp does. Brand rule: surface the
// CITY only, NEVER the `provider` (the field exists only to match the wire shape).

export interface RegionInfo {
  continent: string
  display_name: string
  city: string
  /** Present to match the server shape — NEVER render this (brand rule). */
  provider: string
  is_default: boolean
}

export interface UserRegionResponse {
  /** Chosen region continent, or null → fall back to the default region. */
  preferred_region: string | null
  regions: RegionInfo[]
}

export interface SetPreferredRegionResponse {
  preferred_region: string | null
}

/** Live default city — the only acceptable fallback (no provider, ever). */
const FALLBACK_CITY = 'Falkenstein'

/** city → country, mirrored from the webapp `countryFromCity`. */
function countryFromCity(city: string): string {
  const map: Record<string, string> = {
    falkenstein: 'Germany',
    helsinki: 'Finland',
    ede: 'Netherlands',
  }
  return map[city.toLowerCase()] ?? ''
}

/**
 * Resolve the effective region's CITY from a `/me/region` response.
 * Effective region = `preferred_region ?? default region's continent`; we then
 * look up that region's `city`. Falls back to "Falkenstein" (the live default)
 * when the response is missing/empty. NEVER returns a provider.
 */
export function regionCity(resp: UserRegionResponse | null | undefined): string {
  if (!resp || !resp.regions?.length) return FALLBACK_CITY
  const effective = resp.preferred_region ?? resp.regions.find(r => r.is_default)?.continent
  const region = resp.regions.find(r => r.continent === effective) ?? resp.regions.find(r => r.is_default)
  return region?.city || FALLBACK_CITY
}

/**
 * "City, Country" label for the effective region (e.g. "Falkenstein, Germany").
 * Falls back to just the city when the country is unknown. NEVER a provider.
 */
export function regionLabel(resp: UserRegionResponse | null | undefined): string {
  const city = regionCity(resp)
  const country = countryFromCity(city)
  return country ? `${city}, ${country}` : city
}

/**
 * Map a raw region code (e.g. "falkenstein") to its display CITY. Used where a
 * caller already has the code (AccountView's `Subscription.region`) and doesn't
 * need a fetch. Falls back to "Falkenstein". NEVER returns a provider.
 */
export function regionCityFromCode(code: string | null | undefined): string {
  const map: Record<string, string> = {
    falkenstein: 'Falkenstein',
    helsinki: 'Helsinki',
    ede: 'Ede',
  }
  return map[(code ?? '').toLowerCase()] ?? FALLBACK_CITY
}

// Module-singleton cached fetch: repeated callers in one window share ONE
// network round-trip. On error the cached promise resolves to null so callers
// fall back to the default city without retry-storming.
let regionPromise: Promise<UserRegionResponse | null> | null = null

export function fetchRegion(): Promise<UserRegionResponse | null> {
  if (!regionPromise) {
    regionPromise = accountRegion().then(r => (r.ok ? r.value : null)).catch(() => null)
  }
  return regionPromise
}

function updateCachedRegionPreference(preferredRegion: string | null): void {
  if (!regionPromise) return
  regionPromise = regionPromise.then(resp => (resp ? { ...resp, preferred_region: preferredRegion } : resp))
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

export function accountRegion(): Promise<CommandResult<UserRegionResponse>> {
  return command<UserRegionResponse>('account_region')
}

export async function setAccountRegion(preferredRegion: string | null): Promise<CommandResult<SetPreferredRegionResponse>> {
  const result = await command<SetPreferredRegionResponse>('account_set_region', { preferredRegion })
  if (result.ok) updateCachedRegionPreference(result.value.preferred_region)
  return result
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

/** Per-PC sync-state overview for the in-app Files tab (local vs online-only vs
 *  pinned vs conflicts + recently-changed files). Computed locally from the
 *  SQLite mirror; complements {@link accountStorageBreakdown}. */
export function desktopFileOverview(
  recentLimit?: number,
): Promise<CommandResult<FileOverview>> {
  return command<FileOverview>('desktop_file_overview', { recentLimit })
}

/** Query the local desktop file-name search index. Results are local mirror
 *  entries and are opened via the existing OS file-manager command. */
export function desktopSearchFiles(
  query: string,
  limit?: number,
): Promise<CommandResult<DesktopSearchResponse>> {
  return command<DesktopSearchResponse>('desktop_search_files', { query, limit })
}

export async function desktopListFileVersions(fileId: string): Promise<CommandResult<FileVersionListResponse>> {
  const result = await command<unknown>('list_file_versions', { fileId })
  if (!result.ok) return result
  return { ok: true, value: normalizeFileVersionListResponse(result.value) }
}

export function desktopRestoreFileVersion(
  fileId: string,
  versionId: string,
): Promise<CommandResult<RestoreFileVersionQueuedResponse>> {
  return command<RestoreFileVersionQueuedResponse>('restore_file_version', { fileId, versionId })
}

export function desktopConfirmAction(password: string): Promise<CommandResult<ConfirmActionResult>> {
  return command<ConfirmActionResult>('desktop_confirm_action', { password })
}

export function desktopTrashList(): Promise<CommandResult<DesktopTrashItem[]>> {
  return command<DesktopTrashItem[]>('desktop_trash_list')
}

export function desktopTrashRestore(fileId: string, fileName?: string): Promise<CommandResult<void>> {
  return command<void>('desktop_trash_restore', { fileId, fileName })
}

export function desktopTrashDeletePermanently(
  fileId: string,
  confirmToken: string,
): Promise<CommandResult<void>> {
  return command<void>('desktop_trash_delete_permanently', { fileId, confirmToken })
}

/** Open the main Beebeeb app window (Windows). Falls back gracefully if the
 *  command is not registered in this build. */
export function showMainAppWindow(): Promise<CommandResult<void>> {
  return command<void>('show_main_app_window')
}

// ── 2FA / TOTP login (desktop credential path) ─────────────────────────────

/** Shape returned by the `desktop_login` Tauri command. */
export interface DesktopLoginResult {
  requires_2fa: boolean
}

/**
 * Authenticate with email + password. Returns `{ requires_2fa: true }` when
 * the account has TOTP enabled — the caller must then invoke
 * `desktopLogin2fa` with the 6-digit code to complete the session.
 */
export function desktopLogin(
  email: string,
  password: string,
): Promise<CommandResult<DesktopLoginResult>> {
  return command<DesktopLoginResult>('desktop_login', { email, password })
}

/**
 * Complete a 2FA-gated login. Call only after `desktopLogin` returned
 * `{ requires_2fa: true }`. The partial token is held server-side for ~5 min.
 * Rejects with "Invalid authentication code" on a wrong code (retryable).
 */
export function desktopLogin2fa(code: string): Promise<CommandResult<void>> {
  return command<void>('desktop_login_2fa', { code })
}

/**
 * Disconnect this device from the account: clears the local session token and
 * wipes any cached credentials from the keychain. The files in the sync root
 * stay on disk; the vault stays intact in the cloud. After this call the root
 * `sync_status` poll will return `logged_in: false` and the SignedOutGate will
 * take over.
 */
export function clearSession(): Promise<CommandResult<void>> {
  return command<void>('clear_session')
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

// ── Known-folder backup ("Manage backup", task 0797) ─────────────────────────
//
// OneDrive-style one-way backup of the user's Windows known folders (Desktop /
// Documents / Pictures / Music / Videos / Downloads) into a matching subfolder
// under the sync root. Model 2: NO OS redirect — the real folders are untouched;
// we just mirror their contents into the vault and the sync engine uploads them.
// Windows-only: on macOS/Linux every row reports `source_path: null`.

/** One backupable known folder + its current state for the Manage-backup panel. */
export interface KnownFolderStatus {
  /** Stable key persisted in config + sent to `set_known_folder_backup`. */
  key: string
  /** UI label / vault subfolder name (e.g. "Documents"). */
  display_name: string
  /** Absolute source path on this machine, or null if not resolvable here
   *  (e.g. non-Windows, or a folder this user has no provisioned path for). */
  source_path: string | null
  /** True if this folder is currently being backed up. */
  enabled: boolean
  /** Rough (bounded) count of files in the source tree, or null if unknown. */
  item_count: number | null
}

/** List the backupable known folders with resolved source paths + enabled state. */
export function getKnownFolderBackup(): Promise<CommandResult<KnownFolderStatus[]>> {
  return command<KnownFolderStatus[]>('get_known_folder_backup')
}

/** Enable/disable backup for one known folder. Enabling triggers an immediate
 *  mirror pass; disabling stops FUTURE copies but does not delete what's already
 *  backed up (that would remove it from your cloud vault). */
export function setKnownFolderBackup(key: string, enabled: boolean): Promise<CommandResult<void>> {
  return command<void>('set_known_folder_backup', { key, enabled })
}

// ── Task 0804 — known-folder backup first-run onboarding prompt ──────────────
//
// The one-time "Back up these folders?" prompt (decision 0797's default-ON
// Desktop/Documents/Pictures). It surfaces AUTOMATICALLY on app start only when
// the user hasn't seen it yet AND no folders are backed up; afterwards it's
// reachable manually from the Manage-backup panel. The seen flag governs ONLY
// the automatic first-run appearance.

/** Whether the first-run known-folder backup prompt has already been shown. */
export function getKnownFolderOnboardingSeen(): Promise<CommandResult<boolean>> {
  return command<boolean>('get_known_folder_onboarding_seen')
}

/** Mark the first-run known-folder backup prompt as shown so it never appears
 *  automatically again (called on both "Back up these folders" and "Not now"). */
export function markKnownFolderOnboardingSeen(): Promise<CommandResult<void>> {
  return command<void>('mark_known_folder_onboarding_seen')
}

// ── Task 0812 — tray flyout: click file → reveal in Explorer + open ──────────

/**
 * Open the folder containing a recently-changed file in Windows Explorer
 * (with the file selected) and then open the file in its default application.
 * `relPath` is the path as returned by `desktop_file_overview` (relative to the
 * sync root). Windows-only — returns an error on other platforms.
 */
export function revealAndOpenFile(relPath: string): Promise<CommandResult<void>> {
  return command<void>('reveal_and_open_file', { relPath })
}

/** Pin (or unpin) a vault folder + its descendants so they're kept always-local
 *  (available offline). Backs the onboarding "backed-up folders stay on this PC"
 *  promise. `item_id` is a vault folder id; the recursion is server-side in the
 *  Rust `set_recursive_pin` command. Returns the pin-update outcome opaquely (the
 *  caller only needs ok/err for this flow). */
export function setRecursivePin(itemId: string, pinned: boolean): Promise<CommandResult<unknown>> {
  return command<unknown>('set_recursive_pin', { itemId, pinned })
}

// ── Bandwidth view — speedtest + 20h traffic chart (task 0810) ───────────────

export interface SpeedtestLatency {
  min_ms: number
  avg_ms: number
  max_ms: number
}

/** Result of a full speedtest run (parity with CLI + iOS). */
export interface SpeedtestResult {
  latency: SpeedtestLatency
  /** Average upload speed in bytes per second. */
  upload_bps: number
  /** Average download speed in bytes per second. */
  download_bps: number
}

/** Run a network speedtest against the Beebeeb API (5 pings + 3×10MB up/down).
 *  May take 30-90 s depending on connection speed. Requires an authenticated session. */
export function runSpeedtest(): Promise<CommandResult<SpeedtestResult>> {
  return command<SpeedtestResult>('run_speedtest')
}

/** A single bandwidth measurement point stored in the local state DB. */
export interface BandwidthSample {
  /** Unix timestamp (seconds) when the sample was recorded. */
  sampled_at: number
  /** Upload bytes transferred during `period_secs`. */
  up_bytes: number
  /** Download bytes transferred during `period_secs`. */
  down_bytes: number
  /** Duration of the measurement window in seconds (~20 s per heartbeat beat). */
  period_secs: number
}

/** Fetch bandwidth history from the local state DB for the traffic chart.
 *  `hours` controls the look-back window (default 20). Returns samples
 *  oldest-first; the caller downsamples to a fixed number of chart buckets. */
export function getBandwidthHistory(hours?: number): Promise<CommandResult<BandwidthSample[]>> {
  return command<BandwidthSample[]>('get_bandwidth_history', { hours })
}
