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

export interface DesktopConfig {
  upload_kbps_limit: number
  download_kbps_limit: number
  pause_sync: boolean
  notify_conflicts: boolean
  notify_sync_complete: boolean
  notify_quota_warnings: boolean
}

export interface VaultItem {
  id: string
  name: string
  is_folder: boolean
  path?: string
  children?: VaultItem[]
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
