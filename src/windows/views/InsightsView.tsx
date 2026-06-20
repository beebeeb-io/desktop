/**
 * InsightsView — account insights: where your storage goes.
 *
 * Mounted in the WindowsApp shell (nav id `insights`). Self-fetches on mount via
 * three read-only IPCs and renders three sections:
 *   1) Storage hero  — used / quota with the amber progress bar + percentage,
 *      tagged with the plan label.
 *   2) By type       — Media / Documents / Other as proportional horizontal bars
 *      (calm tonal-amber scale, largest accented), each with bytes + file count.
 *   3) Largest files — the N largest files, middle-truncated path + size + a
 *      category chip.
 *
 * Brand: amber only for encryption/quota state + the by-type bars; Inter for
 * human text, JetBrains Mono for every machine value (bytes, counts, paths,
 * percentages, file ids); name the city only ("Falkenstein"), never the storage provider; honest voice; no emojis.
 *
 * Data wrappers (desktopApi.ts → src-tauri account commands):
 *   accountUsage()             → BillingUsage  { used_bytes, quota_bytes, percentage }
 *   accountStorageBreakdown(N) → StorageBreakdown { by_category[], largest_files[], total_* }
 *   accountSubscription()      → Subscription  (plan label only)
 *
 * All four states are handled intentionally: loading (skeleton in the real
 * layout shape), error (calm "Not available in this build" for unsupported, else
 * "Couldn't load" + reason + Retry), empty, and loaded.
 */

import { useEffect, useState } from 'react'
import {
  accountUsage,
  accountStorageBreakdown,
  accountSubscription,
  formatBytes,
  type BillingUsage,
  type StorageBreakdown,
  type StorageCategory,
  type StorageCategoryTotal,
  type LargestFile,
  type Subscription,
} from '../../desktopApi'
import { T, Card, PageHeader, Chip, Skeleton, NavIcon, PrimaryBtn } from '../ui'
import { useRegionLabel } from '../useRegion'

// ── Local constants ─────────────────────────────────────────────────────────

const LARGEST_LIMIT = 50

// Stable display order + human labels for the three coarse categories. The DTO
// guarantees all three are present (zero when empty) for a stable legend.
const CATEGORY_ORDER: StorageCategory[] = ['media', 'documents', 'other']
const CATEGORY_LABEL: Record<StorageCategory, string> = {
  media: 'Media',
  documents: 'Documents',
  other: 'Other',
}

// A neutral tonal ink scale for the by-type category bars — applied by RANK
// (largest first) so the dominant category reads slightly bolder, not amber
// (amber is reserved for encryption state and the primary storage-usage bar).
const BAR_TONES = [T.ink3, T.ink4, 'oklch(0.80 0.006 80)'] as const

const DANGER = 'oklch(0.5 0.18 25)'

// ── Helpers ──────────────────────────────────────────────────────────────────

type ErrState = { reason: string; unsupported: boolean }

// Middle-truncate a path so the filename stays visible: keep the tail (the file
// name + a little of its folder) and a short head, eliding the middle.
function truncateMiddle(path: string, head = 10, tail = 30): string {
  if (path.length <= head + tail + 1) return path
  return `${path.slice(0, head)}…${path.slice(path.length - tail)}`
}

// ── Section: Storage hero ────────────────────────────────────────────────────

function StorageHero({
  usage,
  planLabel,
}: {
  usage: BillingUsage
  planLabel: string | null
}) {
  const { used_bytes, quota_bytes, percentage } = usage
  // Prefer the server's percentage (0..1); fall back to a computed ratio so the
  // bar is never wrong if percentage is absent/zero on an unusual plan.
  const ratio =
    Number.isFinite(percentage) && percentage > 0
      ? percentage
      : quota_bytes > 0
        ? used_bytes / quota_bytes
        : 0
  const pct = Math.min(100, Math.max(0, ratio * 100))
  const pctLabel = `${pct.toFixed(pct >= 10 || pct === 0 ? 0 : 1)}%`
  const near = pct >= 90

  return (
    <Card style={{ padding: 22, marginBottom: 14 }}>
      <div style={{ display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between', gap: 16, marginBottom: 16 }}>
        <div style={{ minWidth: 0 }}>
          <div style={{ fontSize: 9.5, fontFamily: T.fontMono, textTransform: 'uppercase' as const, letterSpacing: '0.07em', color: T.ink3, marginBottom: 8 }}>
            Encrypted storage
          </div>
          <div style={{ display: 'flex', alignItems: 'baseline', gap: 8, flexWrap: 'wrap' as const }}>
            <span style={{ fontSize: 28, fontWeight: 700, fontFamily: T.fontMono, letterSpacing: '-0.02em', color: T.ink, lineHeight: 1.1 }}>
              {formatBytes(used_bytes)}
            </span>
            <span style={{ fontSize: 13, fontFamily: T.fontMono, color: T.ink3 }}>
              of {formatBytes(quota_bytes)}
            </span>
          </div>
        </div>
        {planLabel && (
          <Chip tone="neutral">
            <NavIcon name="shield" size={10} color={T.ink3} /> {planLabel}
          </Chip>
        )}
      </div>

      <div className="progress-track" style={{ height: 10 }}>
        <div
          className="progress-fill"
          style={{ width: `${pct}%`, background: near ? T.amberDeep : T.amber }}
        />
      </div>

      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginTop: 10 }}>
        <span style={{ fontSize: 11, fontFamily: T.fontMono, color: near ? T.amberDeep : T.ink3 }}>
          {pctLabel} used
        </span>
        <span style={{ fontSize: 11, fontFamily: T.fontMono, color: T.ink3 }}>
          {formatBytes(Math.max(0, quota_bytes - used_bytes))} free
        </span>
      </div>
    </Card>
  )
}

// ── Section: By-type breakdown ───────────────────────────────────────────────

function ByTypeBreakdown({ breakdown }: { breakdown: StorageBreakdown }) {
  // Index the categories by name so we render in our stable order even if the
  // server reorders them.
  const byName = new Map<StorageCategory, StorageCategoryTotal>()
  for (const c of breakdown.by_category) byName.set(c.category, c)

  const rows = CATEGORY_ORDER.map((cat) => byName.get(cat) ?? { category: cat, bytes: 0, file_count: 0 })
  const max = rows.reduce((m, r) => Math.max(m, r.bytes), 0)

  // Rank by bytes (descending) to assign the tonal scale; ties keep order.
  const rankByCategory = new Map<StorageCategory, number>()
  ;[...rows]
    .sort((a, b) => b.bytes - a.bytes)
    .forEach((r, i) => rankByCategory.set(r.category, i))

  return (
    <Card style={{ padding: 22, marginBottom: 14 }}>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 16 }}>
        <div style={{ fontSize: 13, fontWeight: 600, color: T.ink }}>By type</div>
        <span style={{ fontSize: 11, fontFamily: T.fontMono, color: T.ink3 }}>
          {breakdown.total_files.toLocaleString('en-US')} files · {formatBytes(breakdown.total_bytes)}
        </span>
      </div>

      <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
        {rows.map((row) => {
          const rank = rankByCategory.get(row.category) ?? 2
          const tone = row.bytes > 0 ? BAR_TONES[rank] ?? BAR_TONES[2] : T.line
          const widthPct = max > 0 ? (row.bytes / max) * 100 : 0
          return (
            <div key={row.category}>
              <div style={{ display: 'flex', alignItems: 'baseline', justifyContent: 'space-between', marginBottom: 7 }}>
                <span style={{ fontSize: 12.5, fontWeight: 500, fontFamily: T.fontSans, color: T.ink2 }}>
                  {CATEGORY_LABEL[row.category]}
                </span>
                <span style={{ fontSize: 11, fontFamily: T.fontMono, color: T.ink3 }}>
                  {formatBytes(row.bytes)}
                  <span style={{ color: T.ink4 }}>
                    {'  ·  '}
                    {row.file_count.toLocaleString('en-US')} {row.file_count === 1 ? 'file' : 'files'}
                  </span>
                </span>
              </div>
              <div
                style={{
                  width: '100%',
                  height: 9,
                  borderRadius: 999,
                  background: T.paper3,
                  overflow: 'hidden',
                }}
              >
                <div
                  style={{
                    width: `${Math.max(row.bytes > 0 ? 2 : 0, widthPct)}%`,
                    height: '100%',
                    borderRadius: 'inherit',
                    background: tone,
                    transition: 'width 200ms ease',
                  }}
                />
              </div>
            </div>
          )
        })}
      </div>
    </Card>
  )
}

// ── Section: Largest files ───────────────────────────────────────────────────

function LargestFiles({ files }: { files: LargestFile[] }) {
  return (
    <Card style={{ padding: 0 }}>
      <div style={{ padding: '16px 22px 12px', display: 'flex', alignItems: 'center', justifyContent: 'space-between', borderBottom: `1px solid ${T.line}` }}>
        <div style={{ fontSize: 13, fontWeight: 600, color: T.ink }}>Largest files</div>
        <span style={{ fontSize: 11, fontFamily: T.fontMono, color: T.ink3 }}>
          top {files.length}
        </span>
      </div>
      <div>
        {files.map((f, i) => (
          <div
            key={f.file_id || f.path || i}
            style={{
              display: 'grid',
              gridTemplateColumns: '1fr auto auto',
              gap: 14,
              alignItems: 'center',
              padding: '11px 22px',
              borderBottom: i === files.length - 1 ? 'none' : `1px solid ${T.line}`,
            }}
          >
            <span
              title={f.path}
              style={{
                fontSize: 12,
                fontFamily: T.fontMono,
                color: T.ink2,
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                whiteSpace: 'nowrap' as const,
                minWidth: 0,
              }}
            >
              {truncateMiddle(f.path)}
            </span>
            <Chip tone="neutral">{CATEGORY_LABEL[f.category]}</Chip>
            <span style={{ fontSize: 12, fontFamily: T.fontMono, fontWeight: 600, color: T.ink, textAlign: 'right' as const, minWidth: 64 }}>
              {formatBytes(f.size_bytes)}
            </span>
          </div>
        ))}
      </div>
    </Card>
  )
}

// ── States: loading / error / empty ──────────────────────────────────────────

function LoadingState() {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
      {/* Hero skeleton */}
      <Card style={{ padding: 22 }}>
        <Skeleton width={120} height={10} radius={4} />
        <div style={{ marginTop: 12, marginBottom: 16 }}>
          <Skeleton width={220} height={26} radius={6} />
        </div>
        <Skeleton height={10} radius={999} />
        <div style={{ display: 'flex', justifyContent: 'space-between', marginTop: 12 }}>
          <Skeleton width={70} height={10} />
          <Skeleton width={70} height={10} />
        </div>
      </Card>

      {/* By-type skeleton */}
      <Card style={{ padding: 22 }}>
        <div style={{ marginBottom: 16 }}><Skeleton width={90} height={12} /></div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
          {[0, 1, 2].map((i) => (
            <div key={i}>
              <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 7 }}>
                <Skeleton width={84} height={11} />
                <Skeleton width={120} height={11} />
              </div>
              <Skeleton height={9} radius={999} />
            </div>
          ))}
        </div>
      </Card>

      {/* Largest-files skeleton */}
      <Card style={{ padding: 0 }}>
        <div style={{ padding: '16px 22px 12px', borderBottom: `1px solid ${T.line}` }}>
          <Skeleton width={110} height={12} />
        </div>
        {[0, 1, 2, 3].map((i) => (
          <div
            key={i}
            style={{
              display: 'grid',
              gridTemplateColumns: '1fr auto auto',
              gap: 14,
              alignItems: 'center',
              padding: '11px 22px',
              borderBottom: i === 3 ? 'none' : `1px solid ${T.line}`,
            }}
          >
            <Skeleton width={`${70 - i * 9}%`} height={12} />
            <Skeleton width={60} height={16} radius={999} />
            <Skeleton width={56} height={12} />
          </div>
        ))}
      </Card>
    </div>
  )
}

function ErrorState({ err, onRetry }: { err: ErrState; onRetry: () => void }) {
  if (err.unsupported) {
    return (
      <Card style={{ padding: 22 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
          <div style={{ width: 36, height: 36, borderRadius: 9, background: T.paper2, border: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
            <NavIcon name="chart" size={16} color={T.ink3} />
          </div>
          <div style={{ minWidth: 0 }}>
            <div style={{ fontSize: 13, fontWeight: 600, color: T.ink, marginBottom: 2 }}>Not available in this build</div>
            <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.5 }}>
              Storage insights need the account data layer, which this desktop build doesn’t ship yet.
            </div>
          </div>
        </div>
      </Card>
    )
  }
  return (
    <Card style={{ padding: 22 }}>
      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 12 }}>
        <div style={{ width: 36, height: 36, borderRadius: 9, background: T.paper2, border: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
          <NavIcon name="chart" size={16} color={DANGER} />
        </div>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ fontSize: 13, fontWeight: 600, color: T.ink, marginBottom: 2 }}>Couldn’t load your insights</div>
          <div style={{ fontSize: 11.5, fontFamily: T.fontMono, color: T.ink3, lineHeight: 1.5, wordBreak: 'break-word' as const }}>
            {err.reason}
          </div>
          <div style={{ marginTop: 14 }}>
            <PrimaryBtn onClick={onRetry}>Retry</PrimaryBtn>
          </div>
        </div>
      </div>
    </Card>
  )
}

function EmptyState() {
  return (
    <Card style={{ padding: 0 }}>
      <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', padding: '48px 36px', gap: 10 }}>
        <NavIcon name="cloud" size={26} color={T.ink4} />
        <div style={{ fontSize: 14, fontWeight: 600, color: T.ink, marginTop: 4 }}>Your vault is empty</div>
        <div style={{ fontSize: 12, color: T.ink3, textAlign: 'center' as const, lineHeight: 1.6, maxWidth: 360 }}>
          Nothing to break down yet. Once you add files, you’ll see where your storage goes — by type and by your largest files.
        </div>
      </div>
    </Card>
  )
}

// ── Root component ───────────────────────────────────────────────────────────

export default function InsightsView() {
  const regionLabel = useRegionLabel()
  const [loading, setLoading] = useState(true)
  const [err, setErr] = useState<ErrState | null>(null)
  const [usage, setUsage] = useState<BillingUsage | null>(null)
  const [breakdown, setBreakdown] = useState<StorageBreakdown | null>(null)
  const [planLabel, setPlanLabel] = useState<string | null>(null)
  // Bumped on Retry to re-run the effect.
  const [reloadKey, setReloadKey] = useState(0)

  useEffect(() => {
    let cancelled = false
    setLoading(true)
    setErr(null)

    void (async () => {
      // Usage + breakdown are the load-bearing pair — fetch them together; the
      // subscription is best-effort (plan label only) and never blocks render.
      const [usageRes, breakdownRes, subRes] = await Promise.all([
        accountUsage(),
        accountStorageBreakdown(LARGEST_LIMIT),
        accountSubscription(),
      ])
      if (cancelled) return

      if (!usageRes.ok) {
        setErr({ reason: usageRes.reason, unsupported: usageRes.unsupported })
        setLoading(false)
        return
      }
      if (!breakdownRes.ok) {
        setErr({ reason: breakdownRes.reason, unsupported: breakdownRes.unsupported })
        setLoading(false)
        return
      }

      setUsage(usageRes.value)
      setBreakdown(breakdownRes.value)
      setPlanLabel(subRes.ok ? planLabelFor(subRes.value) : null)
      setLoading(false)
    })()

    return () => {
      cancelled = true
    }
  }, [reloadKey])

  const retry = () => setReloadKey((k) => k + 1)

  const isEmpty =
    breakdown != null && breakdown.total_files === 0 && breakdown.total_bytes === 0

  return (
    <div style={{ overflow: 'auto', padding: '28px 36px', flex: 1 }}>
      <PageHeader
        title="Insights"
        subtitle="Where your storage goes — by content type and your largest files. Everything here is computed from what’s already encrypted on this PC."
        aside={
          <Chip tone="amber">
            <NavIcon name="lock" size={10} color="oklch(0.4 0.08 72)" /> {regionLabel}
          </Chip>
        }
      />

      {loading ? (
        <LoadingState />
      ) : err ? (
        <ErrorState err={err} onRetry={retry} />
      ) : isEmpty ? (
        <EmptyState />
      ) : usage && breakdown ? (
        <>
          <StorageHero usage={usage} planLabel={planLabel} />
          <ByTypeBreakdown breakdown={breakdown} />
          {breakdown.largest_files.length > 0 && <LargestFiles files={breakdown.largest_files} />}
        </>
      ) : (
        // Defensive fallback — should be unreachable, but never white-screen.
        <EmptyState />
      )}
    </div>
  )
}

// Map a Subscription to a short, human plan label. The server's `plan` is a slug
// ("free" | "personal" | "team" | "business" | …); title-case it for display and
// fall back to the raw value for any plan we don't have a nicer name for.
function planLabelFor(sub: Subscription): string | null {
  const raw = (sub.plan ?? '').trim()
  if (!raw) return null
  const nice: Record<string, string> = {
    free: 'Free plan',
    personal: 'Personal plan',
    team: 'Team plan',
    business: 'Business plan',
  }
  const key = raw.toLowerCase()
  return nice[key] ?? `${raw.charAt(0).toUpperCase()}${raw.slice(1)} plan`
}
