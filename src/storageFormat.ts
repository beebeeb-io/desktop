/**
 * Locale-aware size formatting for the menu-bar popover's header and progress
 * lines (task 1683 slice 2; spec section 9, row "Storage used and quota", and
 * section 3: "Numbers and dates follow the Mac's locale, `84,3 GB` on a Dutch Mac").
 *
 * Why not `formatBytes` in `desktopApi.ts`: it always prints a dot, drops the
 * decimal from 1 GB up to 1 TB (`84.3 GB` becomes `84 GB`) and never reaches a
 * decimal in the gigabytes. Windows still uses it, unchanged.
 *
 * Rules: decimal units (1 GB = 1,000,000,000 bytes, like Finder), at most one
 * decimal, a trailing `,0` trimmed (`200 GB`, not `200,0 GB`), the decimal mark
 * and grouping from the locale, and a value that rounds up to 1,000 of a unit
 * is shown in the next unit (999.96 MB is `1 GB`, never `1000 MB`).
 */

const UNITS = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'] as const

/** Shown for a size that is not a number or is negative, like `formatBytes`. */
export const UNKNOWN_SIZE = '—'

/** The Mac's language, from the webview. Falls back to English. */
export function systemLocale(): string {
  try {
    const nav = typeof navigator === 'undefined' ? undefined : navigator
    return nav?.languages?.[0] || nav?.language || 'en'
  } catch {
    return 'en'
  }
}

function numberFormat(locale: string, maximumFractionDigits: number): Intl.NumberFormat {
  try {
    return new Intl.NumberFormat(locale, { minimumFractionDigits: 0, maximumFractionDigits })
  } catch {
    // An invalid locale tag throws RangeError: use English rather than no number.
    return new Intl.NumberFormat('en', { minimumFractionDigits: 0, maximumFractionDigits })
  }
}

export function formatStorageSize(bytes: number | null | undefined, locale: string = systemLocale()): string {
  if (bytes == null || !Number.isFinite(bytes) || bytes < 0) return UNKNOWN_SIZE
  if (bytes < 1000) return `${numberFormat(locale, 0).format(Math.round(bytes))} B`

  let unit = 1
  let value = bytes / 1000
  // Move up while the ROUNDED value would print as 1,000 of this unit.
  while (unit < UNITS.length - 1 && Math.round(value * 10) / 10 >= 1000) {
    value /= 1000
    unit += 1
  }
  return `${numberFormat(locale, 1).format(value)} ${UNITS[unit]}`
}

/** `Using 84,3 GB of 200 GB` without the word (the UI owns the words). */
export function formatStorageUsage(
  used: number | null | undefined,
  quota: number | null | undefined,
  locale: string = systemLocale(),
): { used: string; quota: string } {
  return { used: formatStorageSize(used, locale), quota: formatStorageSize(quota, locale) }
}
