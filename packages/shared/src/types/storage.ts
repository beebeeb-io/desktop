// Shared storage-pool and migration types.
import type { LifecyclePhase } from './admin-ops'

export interface StoragePool {
  id: string
  name: string
  provider: string
  endpoint: string
  bucket: string
  region: string
  /** Human-readable location name shown in region badges. */
  city?: string | null
  /** Continent slug for data-residency display (e.g. "europe") */
  continent?: string | null
  display_name: string
  is_default: boolean
  is_active: boolean
  capacity_bytes: number | null
  used_bytes: number
  usage_pct: number | null
  created_at: string
  lifecycle_phase?: LifecyclePhase
}

export interface MigrationSummary {
  pending: number
  copying: number
  verifying: number
  done: number
  failed: number
  total: number
}

export interface MigrationEntry {
  id: string
  file_id: string
  from_pool: string | null
  to_pool: string | null
  status: string
  chunks_copied: number
  error: string | null
  started_at: string | null
  completed_at: string | null
  created_at: string
}

export interface PoolUsageEntry {
  user_id: string
  email: string
  used_bytes: number
  file_count: number
}

export interface UserStoragePoolEntry {
  pool_id: string
  pool_name: string
  file_count: number
  total_bytes: number
}

export interface UserStorageBreakdown {
  user_id: string
  /** Pools where the user has at least one non-trashed, non-folder file. */
  pools: UserStoragePoolEntry[]
  total_files: number
  total_bytes: number
  plan_name: string
  plan_limit_bytes: number
}
