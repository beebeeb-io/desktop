// ─── File list sort catalog ─────────────────────────
// The one place the list's sort options, their persisted key prefix and the
// type-group rank live (moved out of file-list.tsx unchanged).

import type { DriveFile } from '../types'

// ─── Sort types ─────────────────────────────────────

export type SortKey =
  | 'name-asc' | 'name-desc'
  | 'modified-desc' | 'modified-asc'
  | 'size-desc' | 'size-asc'
  | 'type-asc'
  | 'starred-first'

export const SORT_OPTIONS: { key: SortKey; label: string }[] = [
  { key: 'name-asc',      label: 'Name (A → Z)' },
  { key: 'name-desc',     label: 'Name (Z → A)' },
  { key: 'modified-desc', label: 'Modified (newest)' },
  { key: 'modified-asc',  label: 'Modified (oldest)' },
  { key: 'size-desc',     label: 'Size (largest)' },
  { key: 'size-asc',      label: 'Size (smallest)' },
  { key: 'type-asc',      label: 'Type' },
  { key: 'starred-first', label: 'Starred first' },
]

// Type-group rank: lower = earlier in the list
export function typeGroupRank(file: DriveFile, name: string): number {
  if (file.is_folder) return 0
  const ext = name.split('.').pop()?.toLowerCase() ?? ''
  if (['jpg', 'jpeg', 'png', 'gif', 'heic', 'webp', 'svg', 'ico', 'bmp', 'tiff', 'tif', 'avif'].includes(ext)) return 1
  if (['mp4', 'mov', 'avi', 'mkv', 'webm', 'flv', 'wmv', 'm4v'].includes(ext)) return 2
  if (['mp3', 'wav', 'flac', 'aac', 'ogg', 'wma', 'm4a', 'aiff', 'opus'].includes(ext)) return 3
  if (['pdf', 'doc', 'docx', 'odt', 'rtf', 'txt', 'pages', 'md', 'mdx', 'xls', 'xlsx', 'ods', 'numbers', 'ppt', 'pptx', 'odp', 'key'].includes(ext)) return 4
  if (['zip', 'tar', 'gz', 'rar', '7z', 'bz2', 'xz', 'zst', 'dmg', 'iso'].includes(ext)) return 5
  if (['js', 'jsx', 'ts', 'tsx', 'py', 'rs', 'go', 'rb', 'java', 'kt', 'swift', 'c', 'cpp', 'h', 'cs', 'php', 'sh', 'html', 'css', 'json', 'yaml', 'yml', 'toml', 'sql'].includes(ext)) return 6
  return 7
}

export const SORT_KEY_PREFIX = 'beebeeb.sort.'
