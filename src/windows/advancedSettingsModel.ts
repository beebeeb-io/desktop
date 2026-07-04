export type ThemePreference = 'light' | 'dark' | 'system'
export type ResolvedTheme = 'light' | 'dark'

export const DEFAULT_LOCAL_CACHE_LIMIT_BYTES = 0
export const LOCAL_CACHE_LIMIT_100_GB_BYTES = 100_000_000_000

export const CACHE_LIMIT_OPTIONS: Array<{ value: number; label: string }> = [
  { value: 5_000_000_000, label: '5 GB' },
  { value: 25_000_000_000, label: '25 GB' },
  { value: LOCAL_CACHE_LIMIT_100_GB_BYTES, label: '100 GB' },
  { value: 0, label: 'Unlimited' },
]

export function normalizeThemePreference(value: unknown): ThemePreference {
  return value === 'light' || value === 'dark' || value === 'system' ? value : 'system'
}

export function resolveThemePreference(preference: ThemePreference, systemPrefersDark: boolean): ResolvedTheme {
  if (preference === 'system') return systemPrefersDark ? 'dark' : 'light'
  return preference
}

export interface CacheLimitInput {
  cacheBytes: number
  pinnedBytes: number
  limitBytes: number | null | undefined
}

export interface CacheLimitView {
  totalBytes: number
  capBytes: number | null
  percentUsed: number | null
  pinnedExceedsCap: boolean
  usageLabel: string
  capLabel: string
  warning: string | null
}

export function normalizeCacheLimitBytes(value: number | null | undefined): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) return DEFAULT_LOCAL_CACHE_LIMIT_BYTES
  return Math.max(0, Math.round(value))
}

function formatBytes(bytes: number): string {
  if (bytes == null || !Number.isFinite(bytes) || bytes < 0) return '-'
  if (bytes < 1_000) return `${bytes} B`
  if (bytes < 1_000_000) return `${(bytes / 1_000).toFixed(0)} KB`
  if (bytes < 1_000_000_000) return `${(bytes / 1_000_000).toFixed(0)} MB`
  if (bytes < 1_000_000_000_000) return `${(bytes / 1_000_000_000).toFixed(0)} GB`
  return `${(bytes / 1_000_000_000_000).toFixed(1)} TB`
}

export function buildCacheLimitView(input: CacheLimitInput): CacheLimitView {
  const cacheBytes = Math.max(0, input.cacheBytes)
  const pinnedBytes = Math.max(0, input.pinnedBytes)
  const totalBytes = cacheBytes + pinnedBytes
  const limitBytes = normalizeCacheLimitBytes(input.limitBytes)
  const capBytes = limitBytes === 0 ? null : limitBytes
  const percentUsed = capBytes == null || capBytes === 0 ? null : Math.min(100, Math.round((totalBytes / capBytes) * 100))
  const pinnedExceedsCap = capBytes != null && pinnedBytes > capBytes
  const capLabel = capBytes == null ? 'Unlimited' : formatBytes(capBytes)

  return {
    totalBytes,
    capBytes,
    percentUsed,
    pinnedExceedsCap,
    usageLabel: `${formatBytes(totalBytes)} used`,
    capLabel,
    warning: pinnedExceedsCap
      ? `Pinned files use ${formatBytes(pinnedBytes)}, which is above the ${capLabel} cap. Unpin files to bring local usage back under the limit.`
      : null,
  }
}
