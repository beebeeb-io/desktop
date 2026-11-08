/**
 * Pure formatting helpers for the office loading skeleton (task 1567).
 * Kept separate from the component so the percentage/label math is
 * unit-testable without a DOM.
 */

/** Clamped 0–100 integer percent. A zero/unknown total never divides by zero or shows NaN/Infinity. */
export function officeLoadPercent(loadedBytes: number, totalBytes: number): number {
  if (!Number.isFinite(loadedBytes) || !Number.isFinite(totalBytes) || totalBytes <= 0) return 0
  const pct = (loadedBytes / totalBytes) * 100
  return Math.max(0, Math.min(100, Math.round(pct)))
}

/** "42 MB / 257 MB" style label, or null when there is nothing meaningful to show yet. */
export function officeLoadByteLabel(loadedBytes: number, totalBytes: number): string | null {
  if (!Number.isFinite(totalBytes) || totalBytes <= 0) return null
  return `${formatMegabytes(loadedBytes)} / ${formatMegabytes(totalBytes)}`
}

function formatMegabytes(bytes: number): string {
  const mb = Math.max(0, bytes) / (1024 * 1024)
  return `${mb < 10 ? mb.toFixed(1) : Math.round(mb)} MB`
}
