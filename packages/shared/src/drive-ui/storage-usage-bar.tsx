/**
 * StorageUsageBar — reusable storage quota progress bar.
 *
 * Colour thresholds:
 *   < 70 %  → green
 *   70–90 % → amber
 *   > 90 %  → red + optional warning banner
 *
 * `compact` mode renders the slim sidebar bar: thin bar + "X GB / Y GB" +
 * Manage link, plus at most ONE warning line — "Running low. Upgrade →"
 * from ≥80%, escalating to "Almost full. Upgrade →" at ≥95% (task 1705:
 * exactly one bar and at most one warning line in the sidebar; the old
 * duplicate second bar was removed).
 * Default (full) mode is for settings pages — taller bar + upgrade link.
 */

import { useEffect, useState } from 'react'
import { DefaultDriveLink, type DriveLinkProps } from './link'
import type { ReactNode } from 'react'
import { formatStorageSI } from './lib/format'

interface StorageUsageBarProps {
  usedBytes: number
  quotaBytes: number
  planName?: string
  /** Sidebar compact view — thin bar, no upgrade callout. */
  compact?: boolean
  /** Draws the Manage / Upgrade links; default is a plain anchor (web passes its router Link). */
  renderLink?: (props: DriveLinkProps) => ReactNode
}

function barColor(pct: number): string {
  if (pct > 90) return 'var(--color-red)'
  if (pct > 70) return 'var(--color-amber)'
  return 'oklch(0.55 0.12 155)'  // green matching the rest of the design
}

function textColorClass(pct: number): string {
  if (pct > 90) return 'text-red'
  if (pct > 70) return 'text-amber-deep'
  return 'text-ink-2'
}

export function StorageUsageBar({
  usedBytes,
  quotaBytes,
  planName,
  compact = false,
  renderLink,
}: StorageUsageBarProps) {
  const L = renderLink ?? DefaultDriveLink
  const [mounted, setMounted] = useState(false)
  useEffect(() => {
    const t = setTimeout(() => setMounted(true), 80)
    return () => clearTimeout(t)
  }, [])

  if (quotaBytes <= 0) return null

  const pct = Math.min(100, (usedBytes / quotaBytes) * 100)
  const isWarning = pct > 90
  const isAmber   = pct > 70 && pct <= 90
  const color     = barColor(pct)
  // Compact warning line (sidebar, task 1705): ONE line at ≥80%, wording
  // escalates at ≥95%. Independent of the bar colour thresholds above, which
  // stay as they were (amber >70%, red >90%).
  const showWarning = pct >= 80
  const isCritical  = pct >= 95

  if (compact) {
    // ── Compact (sidebar) ──────────────────────────────────────────────
    return (
      <div>
        <div className="h-[3px] w-full rounded-full bg-paper-3 overflow-hidden mb-1.5">
          <div
            className="h-full rounded-full transition-all duration-700 ease-out"
            style={{ width: mounted ? `${pct}%` : '0%', background: color }}
          />
        </div>
        <div className="flex justify-between items-center text-[11px]">
          <span className={`font-mono tabular-nums ${isWarning ? 'text-red' : isAmber ? 'text-amber-deep' : ''}`}>
            {formatStorageSI(usedBytes)} / {formatStorageSI(quotaBytes)}
          </span>
          <L to="/settings/billing" className="font-medium text-amber-deep hover:underline">
            Manage
          </L>
        </div>
        {showWarning && (
          <div className={`mt-2 text-[11px] leading-snug ${isCritical ? 'text-red' : 'text-amber-deep'}`}>
            {isCritical ? 'Almost full. ' : 'Running low. '}
            <L to="/billing" className="font-semibold underline underline-offset-2">
              Upgrade →
            </L>
          </div>
        )}
      </div>
    )
  }

  // ── Full (settings) ───────────────────────────────────────────────────
  return (
    <div className="w-full">
      <div className="flex items-baseline justify-between mb-2">
        <span className={`text-sm font-semibold ${textColorClass(pct)}`}>
          {formatStorageSI(usedBytes)}
        </span>
        <span className="text-[12px] text-ink-3">
          of {formatStorageSI(quotaBytes)}
          {planName && <span className="ml-2 font-mono text-[11px] text-ink-4">({planName})</span>}
        </span>
      </div>

      {/* Bar */}
      <div className="relative h-3 w-full rounded-full bg-paper-3 overflow-hidden">
        <div
          className="h-full rounded-full transition-all duration-700 ease-out"
          style={{
            width: mounted ? `${pct}%` : '0%',
            background: color,
          }}
        />
      </div>

      {/* Percentage label */}
      <div className="mt-1.5 flex items-center justify-between text-[11px]">
        <span className={`font-mono tabular-nums ${isWarning ? 'text-red' : isAmber ? 'text-amber-deep' : 'text-ink-4'}`}>
          {Math.round(pct)}% used
        </span>
        <span className="text-ink-4 font-mono tabular-nums">
          {formatStorageSI(Math.max(0, quotaBytes - usedBytes))} free
        </span>
      </div>

      {/* Warning callout — only when >90% */}
      {isWarning && (
        <div className="mt-3 flex items-center gap-2.5 px-3 py-2 rounded-md bg-red/10 border border-red/20">
          <span className="text-[11px] text-red leading-snug flex-1">
            Running low on storage — {formatStorageSI(Math.max(0, quotaBytes - usedBytes))} remaining.
          </span>
          <L
            to="/settings/billing"
            className="shrink-0 text-[11px] font-semibold text-red hover:underline underline-offset-2 whitespace-nowrap"
          >
            Upgrade →
          </L>
        </div>
      )}
    </div>
  )
}
