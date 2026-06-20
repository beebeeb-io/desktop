/**
 * BandwidthChart — hand-rolled inline SVG up/down area chart for the 20h traffic view.
 *
 * NO external charting library. The chart is a pure inline SVG, matching the
 * existing hand-rolled SVG style used in SelectiveSyncView.tsx and SecurityView.tsx.
 * Adding a charting dep would add ~100 kB to the bundle for a single view — not worth it.
 *
 * Design:
 *   - Upload area: amber (task 0810 brand: one amber accent for the primary series).
 *   - Download area: neutral blue-grey (T.ink3 alpha), contrasting without competing.
 *   - No emojis. JetBrains Mono for axis labels. Honest empty state.
 *   - Downsamples raw samples to a fixed number of display buckets so the chart
 *     renders cleanly regardless of how many raw beats the DB returns.
 *
 * Props:
 *   - samples: BandwidthSample[] (from getBandwidthHistory, oldest-first)
 *   - buckets: number of SVG columns to render (default 60)
 *   - heightPx: chart area height (default 80)
 */

import type { BandwidthSample } from '../../desktopApi'
import { T } from '../ui'

// Amber accent (one, per brand rules) and a paired neutral for the second series.
const AMBER_FILL = 'oklch(0.82 0.17 84 / 0.35)'
const AMBER_STROKE = 'oklch(0.82 0.17 84)'
const DOWN_FILL = 'oklch(0.55 0.04 240 / 0.22)'
const DOWN_STROKE = 'oklch(0.55 0.04 240 / 0.70)'

// ── Downsampling ──────────────────────────────────────────────────────────────

interface Bucket {
  t: number     // midpoint timestamp of the bucket
  up: number    // sum of up_bytes in this bucket
  down: number  // sum of down_bytes in this bucket
}

/**
 * Downsample raw DB samples into a fixed number of time-equal buckets.
 * Each bucket spans an equal fraction of the total window.
 * Returns `count` buckets, oldest-first.
 */
function downsample(samples: BandwidthSample[], count: number): Bucket[] {
  if (samples.length === 0 || count === 0) return []

  const tMin = samples[0].sampled_at
  const tMax = samples[samples.length - 1].sampled_at

  // If the window is a single point, put everything in one bucket.
  const span = tMax - tMin
  if (span === 0 || count === 1) {
    const up = samples.reduce((s, r) => s + r.up_bytes, 0)
    const down = samples.reduce((s, r) => s + r.down_bytes, 0)
    return [{ t: tMin, up, down }]
  }

  const bucketSpan = span / count
  const buckets: Bucket[] = Array.from({ length: count }, (_, i) => ({
    t: tMin + (i + 0.5) * bucketSpan,
    up: 0,
    down: 0,
  }))

  for (const s of samples) {
    const idx = Math.min(count - 1, Math.floor((s.sampled_at - tMin) / bucketSpan))
    buckets[idx].up += s.up_bytes
    buckets[idx].down += s.down_bytes
  }

  return buckets
}

// ── SVG polyline helper ───────────────────────────────────────────────────────

/**
 * Build the `points` attribute for an SVG polyline (or the path for an area fill).
 * `values[i]` maps to X = i/(n-1)*W, Y = (1 - values[i]/peak)*H.
 */
function buildAreaPath(
  values: number[],
  peak: number,
  width: number,
  height: number,
  padX: number,
  padY: number,
): string {
  if (values.length === 0) return ''
  const n = values.length
  const safePeak = peak > 0 ? peak : 1
  const innerW = width - 2 * padX
  const innerH = height - 2 * padY

  const pts = values.map((v, i) => {
    const x = padX + (n === 1 ? innerW / 2 : (i / (n - 1)) * innerW)
    const y = padY + innerH * (1 - v / safePeak)
    return `${x.toFixed(1)},${y.toFixed(1)}`
  })

  // Area path: go across the top, then back along the baseline.
  const baseline = (padY + innerH).toFixed(1)
  const firstX = padX + (n === 1 ? innerW / 2 : 0)
  const lastX = padX + innerW

  return [
    `M ${firstX.toFixed(1)},${baseline}`,
    ...pts.map((p) => `L ${p}`),
    `L ${lastX.toFixed(1)},${baseline}`,
    'Z',
  ].join(' ')
}

// ── Chart component ───────────────────────────────────────────────────────────

interface BandwidthChartProps {
  samples: BandwidthSample[]
  /** Number of display buckets (columns). Default 60. */
  buckets?: number
  /** Chart area height in px. Default 80. */
  heightPx?: number
  /** Whether to show the axis labels. Default true. */
  showLabels?: boolean
}

export function BandwidthChart({
  samples,
  buckets = 60,
  heightPx = 80,
  showLabels = true,
}: BandwidthChartProps) {
  const pts = downsample(samples, buckets)
  const upVals = pts.map((b) => b.up)
  const downVals = pts.map((b) => b.down)

  // Use a shared Y-axis scale so up and down are visually comparable.
  const peak = Math.max(...upVals, ...downVals, 1)

  // Width is set to 100% via viewBox so it fills the container responsively.
  const W = 400
  const H = heightPx
  const PAD_X = 0
  const PAD_Y = 4

  const upPath = buildAreaPath(upVals, peak, W, H, PAD_X, PAD_Y)
  const downPath = buildAreaPath(downVals, peak, W, H, PAD_X, PAD_Y)

  // Y-axis label for the peak speed. Convert bytes/period to bps, then to Mbps.
  // The period is approximately 20 s per beat; use actual period if available.
  const avgPeriod = pts.length > 0
    ? (samples.reduce((s, r) => s + r.period_secs, 0) / samples.length) || 20
    : 20
  const peakBps = peak / avgPeriod
  const peakMbps = (peakBps * 8) / 1_000_000

  function fmtMbps(mbps: number): string {
    if (mbps < 0.01) return '0'
    if (mbps < 1) return `${(mbps).toFixed(2)}`
    if (mbps < 10) return `${(mbps).toFixed(1)}`
    return `${Math.round(mbps)}`
  }

  const isEmpty = samples.length === 0

  if (isEmpty) {
    return (
      <div
        style={{
          height: heightPx,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          background: T.paper2,
          border: `1px solid ${T.line}`,
          borderRadius: 8,
          fontSize: 11,
          fontFamily: T.fontMono,
          color: T.ink4,
        }}
      >
        No traffic data yet — starts recording once the sync engine runs.
      </div>
    )
  }

  return (
    <div style={{ position: 'relative', userSelect: 'none' as const }}>
      <svg
        viewBox={`0 0 ${W} ${H}`}
        width="100%"
        height={heightPx}
        style={{ display: 'block', overflow: 'visible' }}
        aria-label="Bandwidth history chart"
      >
        {/* Download area (neutral) — rendered first so upload sits on top */}
        <path d={downPath} fill={DOWN_FILL} stroke={DOWN_STROKE} strokeWidth={1} />
        {/* Upload area (amber — the one accent) */}
        <path d={upPath} fill={AMBER_FILL} stroke={AMBER_STROKE} strokeWidth={1.5} />
      </svg>

      {showLabels && (
        <div
          style={{
            display: 'flex',
            justifyContent: 'space-between',
            alignItems: 'center',
            marginTop: 6,
          }}
        >
          {/* Legend */}
          <div style={{ display: 'flex', gap: 12 }}>
            <span style={{ display: 'flex', alignItems: 'center', gap: 5, fontSize: 10, fontFamily: T.fontMono, color: T.ink3 }}>
              <span style={{ width: 10, height: 2, background: AMBER_STROKE, display: 'inline-block', borderRadius: 1 }} />
              Upload
            </span>
            <span style={{ display: 'flex', alignItems: 'center', gap: 5, fontSize: 10, fontFamily: T.fontMono, color: T.ink3 }}>
              <span style={{ width: 10, height: 2, background: DOWN_STROKE, display: 'inline-block', borderRadius: 1 }} />
              Download
            </span>
          </div>

          {/* Peak label */}
          <span style={{ fontSize: 10, fontFamily: T.fontMono, color: T.ink4 }}>
            peak {fmtMbps(peakMbps)} Mbps
          </span>
        </div>
      )}
    </div>
  )
}
