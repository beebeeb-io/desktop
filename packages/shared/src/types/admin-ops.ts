// Shared admin-ops types: file versions, notifications, sessions, impersonation.
export interface FileVersion {
  id: string
  version_number: number
  size_bytes: number
  chunk_count: number
  created_by: string | null
  created_at: string
}

export interface VersionSetting {
  id: string
  scope: string
  target_id: string | null
  enabled: boolean
  max_versions: number | null
  retention_days: number | null
}

export interface Notification {
  id: string
  type: string
  title: string
  body: string | null
  data: Record<string, unknown> | null
  read: boolean
  created_at: string
}

export interface Session {
  id: string
  is_current: boolean
  created_at: string
  expires_at: string
}

export interface ImpersonateResponse {
  session_token: string
  user_id: string
  email: string
}

export interface RoleChangeResponse {
  message: string
  user_id: string
  role: 'user' | 'admin' | 'superadmin'
}

export interface AdminStatsStorageByPlan {
  plan: string
  users: number
  used_bytes: number
  /** Total extra TB purchased as add-ons across all users on this plan. */
  total_extra_storage_tb?: number
}

export interface AdminStatsRevenuePlan {
  plan: string
  count: number
  price_cents?: number
  contribution_cents?: number
}

export interface AdminStats {
  users: {
    /** People only — soft-deleted rows and `is_test_account` fixtures excluded (server `admin.rs` `human_user_count_sql`). */
    total: number
    /** Subset of `total` that signed up through the 0928 pilot-key gate. */
    pilots?: number
    /** Automation fixtures, reported rather than hidden so a growing count is visible. */
    test_accounts?: number
    active_7d: number
    active_30d?: number
    /** Humans only, same predicate as `total`. */
    signups_today: number
    /** Daily signup counts for the last 7 days, oldest → today. */
    signups_last_7d?: number[]
    /** Three-letter weekday labels paired 1:1 with `signups_last_7d`. */
    signups_last_7d_labels?: string[]
  }
  files: {
    total: number
    storage_used_bytes: number
    uploads_today: number
  }
  shares: {
    active_links: number
    active_invites: number
    created_today: number
  }
  storage?: {
    total_used_bytes?: number
    pool_count?: number
    total_files?: number
    total_folders?: number
    avg_file_size_bytes?: number
    storage_by_plan?: AdminStatsStorageByPlan[]
  }
  revenue?: {
    active_paying_users: number
    plan_breakdown: AdminStatsRevenuePlan[]
    estimated_mrr_cents: number
  }
  activity?: {
    uploads_today: number
    downloads_today: number
    shares_created_today: number
    api_errors_today: number
  }
  funnel?: {
    signups_7d: number
    completed_onboarding_7d: number
    onboarding_rate_pct: number
    first_upload_7d: number
    first_upload_rate_pct: number
    verified_email_7d: number
    plan_upgraded_7d: number
  }
}

export interface UpgradePressureUser {
  id: string
  email: string
  plan: string
  used_bytes: number
  bonus_bytes: number
  quota_bytes: number
  utilization_pct: number
}

export interface UpgradePressureResponse {
  users_near_limit: UpgradePressureUser[]
  total_at_risk: number
  summary: {
    free_at_risk: number
    basic_at_risk: number
    pro_at_risk: number
  }
}

export interface HealthWorkerItem {
  interval_secs: number
  last_run: string | null
  seconds_ago?: number | null
  status: string
}

export interface HealthResponse {
  status: string
  version: string
  uptime_seconds: number
  db: string
  blob_store?: string
  db_pool?: { active: number; idle: number; max: number; size: number }
  checks: {
    database: { status: string; latency_ms: number }
    blob_store: { status: string; pools: number }
  }
  background_workers?: {
    any_stale: boolean
    count: number
    items: Record<string, HealthWorkerItem>
  }
  websocket_connections?: number
  process_memory?: { rss_kb: number; rss_mb: number } | null
}

export interface AdminBillingStats {
  total_subscribers: number
  mrr_cents: number
  conversion_rate: number
  plan_distribution: {
    free: number
    personal: number
    pro: number
    data_hoarder: number
    // Legacy keys — may be absent on new servers
    team?: number
    business?: number
  }
  recent_invoices: Array<{
    id: string
    number: string
    // Nullable (server PR #84, task 1369): a GDPR Art. 17 purge can delete
    // the user while the invoice (a legal/bookkeeping record) is retained,
    // so the FK is now ON DELETE SET NULL rather than blocking the purge.
    // Kept in sync with repos/admin's own consumption of this same field —
    // see AdminInvoiceListItem in repos/admin/src/lib/api.ts.
    user_id: string | null
    amount_cents: number
    currency: string
    status: 'paid' | 'open' | 'void' | 'uncollectible'
    created_at: string
    user_email?: string
    buyer_full_name?: string | null
  }>
}

/**
 * Admin runtime config — GET /api/v1/admin/config (admin settings page).
 * Reconciled field-by-field against the server emission site
 * `beebeeb-api/src/routes/admin.rs` `admin_config()` json! @ 6923-6931 (task 0752,
 * runtime-probed 2026-06-10; re-verified task 1151). The server emits EXACTLY:
 * notifications_configured, notification_connector_count, email_backend,
 * blob_store, opaque_enabled. There is no
 * `email`/`slack`/`turnstile` nested object — those were a phantom removed in 1151;
 * do not reintroduce fields here without a matching server emission.
 */
export interface AdminConfig {
  // ── Actually emitted by GET /admin/config (server admin.rs:6923-6931) ──
  /** Active notification connectors > 0 — server admin.rs:6924. */
  notifications_configured?: boolean
  /** Count of active notification connectors — server admin.rs:6925. */
  notification_connector_count?: number
  /** Email backend label (e.g. "smtp" / "console") — server admin.rs:6928. */
  email_backend?: string
  /** Default storage pool provider label (e.g. "s3" / "local") — server admin.rs:6929. */
  blob_store?: string
  /** Always true — OPAQUE auth is enabled — server admin.rs:6930. */
  opaque_enabled?: boolean
}

export type AbuseReportStatus = 'pending' | 'reviewing' | 'actioned' | 'dismissed'

export interface AbuseReport {
  id: string
  share_token: string
  reporter_id: string | null
  reporter_ip: string | null
  reporter_email: string | null
  reason: string
  status: AbuseReportStatus
  admin_notes: string | null
  created_at: string
  resolved_at: string | null
}

export interface JoinTransferResponse {
  /** Sender's ephemeral X25519 public key, base64. */
  sender_pk: string
  /** One-time token the receiver presents on status / blob / ack. */
  download_token: string
}

export interface TransferStatus {
  status: 'waiting' | 'joined' | 'approved' | 'ready' | 'downloaded' | 'complete' | 'cancelled' | 'expired'
  receiver_pk?: string
  blob_size?: number
  /** Encrypted (with transfer_key) filename hint, base64. */
  file_name_hint?: string
}

export interface TransferProof {
  proof_id: string
  session_id: string
  sender_user_id?: string
  receiver_display_name: string | null
  file_name: string
  file_size_bytes: number
  sha256_hash: string
  created_at: string
}

export interface CspReport {
  id: string
  document_uri: string | null
  violated_directive: string | null
  blocked_uri: string | null
  source_file: string | null
  line_number: number | null
  created_at: string
}

export interface LoginIp {
  ip: string
  first_seen: string
  last_seen: string
}

export interface GrowthDataPoint {
  date: string
  value: number
}

export type LifecyclePhase =
  | 'active'
  | 'quiescing'
  | 'migrating'
  | 'drained'
  | 'deleted'

export type LifecycleOutcome =
  | 'in_progress'
  | 'aborted'
  | 'reverse_migrated'
  | 'archived'
  | 'completed_deleted'

export interface LifecycleRun {
  id: string
  pool_id: string
  target_pool_id: string
  // Nullable (server PR #84, task 1369): a GDPR Art. 17 purge can delete the
  // admin/system actor who started this run while the lifecycle run record
  // itself (an operational audit record) is retained.
  started_by: string | null
  started_at: string
  current_phase: LifecyclePhase
  outcome: LifecycleOutcome
  terminated_at: string | null
}

export interface PoolInsights {
  used_bytes: number
  files_count: number
  users_with_files_count: number
  currently_active_users_count: number
  in_flight_uploads_count: number
  /** Rough estimate based on recent migration throughput; falls back to a
   *  static 50 MB/s baseline when no history data is available. */
  estimated_migration_seconds: number
}

export interface RunProgress {
  run: LifecycleRun
  /** 0..1 fraction of files migrated for this run. */
  phase_progress: number
  throughput_bytes_per_sec: number
  files_total: number
  files_migrated: number
  files_failed: number
  files_pending: number
  /** Remaining seconds at current throughput; 0 when complete. */
  eta_seconds: number
}

export interface RunFileEntry {
  file_id: string
  /** Encrypted filename — UI shows truncated UUID or raw ciphertext; not decryptable in admin context. */
  name_encrypted: string
  size_bytes: number
  status: 'pending' | 'copying' | 'verifying' | 'done' | 'failed' | string
  error: string | null
  started_at: string | null
  completed_at: string | null
  /** When this file_migrations row was seeded — used to distinguish "just
   *  queued" from "stuck" for rows still pending (task 1226). */
  created_at: string
  /** Last worker heartbeat/progress timestamp; null if no worker has ever
   *  claimed the row, or if it hasn't started copying yet (task 1226). */
  last_heartbeat_at: string | null
}

export interface LifecycleRunEvent {
  /** DB sequence number — used as the since_id cursor for WS reconnect replay. */
  id: number
  type: string
  at: string
  // file event fields
  file_id?: string
  size_bytes?: number
  chunks?: number
  bytes_copied?: number
  chunks_copied?: number
  duration_ms?: number
  error?: string
  // throughput_sample fields
  bytes_per_sec?: number
  files_per_sec?: number
  // phase_changed fields
  from?: string
  to?: string
}
