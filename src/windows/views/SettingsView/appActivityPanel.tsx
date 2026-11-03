/**
 * App activity: CPU, memory and network samples for the running engine, polled only while
 * the panel is visible.
 */
import { useEffect, useState } from 'react'
import { appActivitySnapshot, formatBytes, type AppActivitySnapshot } from '../../../desktopApi'
import { T, Card, Chip, NavIcon, Skeleton } from '../../ui'
import { pollWhileVisible } from '../../visiblePoll'
import { ErrorBlock } from './primitives'

export const ACTIVITY_POLL_INTERVAL_MS = 1_500

export function formatCpuPercent(value: number | null | undefined): string {
  if (value == null || !Number.isFinite(value) || value < 0) return '—'
  if (value < 10) return `${value.toFixed(1)}%`
  return `${Math.round(value)}%`
}

export function formatRate(bytesPerSecond: number | null | undefined): string {
  if (bytesPerSecond == null || !Number.isFinite(bytesPerSecond) || bytesPerSecond < 0) return '—'
  return `${formatBytes(bytesPerSecond)}/s`
}

export function formatSampleAge(ms: number | null | undefined): string {
  if (ms == null || !Number.isFinite(ms) || ms < 0) return 'No local sample'
  if (ms < 1_000) return 'just now'
  const seconds = Math.round(ms / 1_000)
  if (seconds < 60) return `${seconds}s ago`
  const minutes = Math.round(seconds / 60)
  if (minutes < 60) return `${minutes}m ago`
  const hours = Math.round(minutes / 60)
  return `${hours}h ago`
}

export type AppActivityLoadState =
  | { phase: 'loading' }
  | { phase: 'error'; reason: string; unsupported: boolean }
  | { phase: 'ready'; snapshot: AppActivitySnapshot; updatedAt: number }

export function ActivityMetric({
  label,
  value,
  detail,
  icon,
  accent = T.ink3,
}: {
  label: string
  value: string
  detail?: string
  icon: string
  accent?: string
}) {
  return (
    <div style={{ background: T.paper2, border: `1px solid ${T.line}`, borderRadius: 8, padding: '13px 14px', minWidth: 0 }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 7, marginBottom: 8 }}>
        <NavIcon name={icon} size={12} color={accent} />
        <div style={{ fontSize: 9.5, fontFamily: T.fontMono, textTransform: 'uppercase' as const, letterSpacing: '0.06em', color: T.ink4 }}>
          {label}
        </div>
      </div>
      <div style={{ fontSize: 22, lineHeight: 1.1, fontWeight: 700, fontFamily: T.fontMono, color: T.ink, overflowWrap: 'anywhere' as const }}>
        {value}
      </div>
      {detail && (
        <div style={{ marginTop: 5, fontSize: 10.5, lineHeight: 1.4, fontFamily: T.fontMono, color: T.ink4, overflowWrap: 'anywhere' as const }}>
          {detail}
        </div>
      )}
    </div>
  )
}

export function AppActivityPanel() {
  const [state, setState] = useState<AppActivityLoadState>({ phase: 'loading' })
  const [reloadKey, setReloadKey] = useState(0)

  useEffect(() => {
    let cancelled = false

    const load = async () => {
      const result = await appActivitySnapshot()
      if (cancelled) return
      if (result.ok) {
        setState({ phase: 'ready', snapshot: result.value, updatedAt: Date.now() })
      } else {
        setState((prev) => (
          prev.phase === 'ready'
            ? prev
            : { phase: 'error', reason: result.reason, unsupported: result.unsupported }
        ))
      }
    }

    // Task 2243: each read opens the state DB; only while the window is on screen.
    const stop = pollWhileVisible(load, ACTIVITY_POLL_INTERVAL_MS)
    return () => {
      cancelled = true
      stop()
    }
  }, [reloadKey])

  if (state.phase === 'loading') {
    return (
      <Card style={{ padding: 18 }}>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(145px, 1fr))', gap: 12 }}>
          {Array.from({ length: 4 }).map((_, index) => (
            <Skeleton key={index} height={84} radius={8} />
          ))}
        </div>
      </Card>
    )
  }

  if (state.phase === 'error') {
    return (
      <ErrorBlock
        reason={state.reason}
        unsupported={state.unsupported}
        onRetry={() => {
          setState({ phase: 'loading' })
          setReloadKey((key) => key + 1)
        }}
      />
    )
  }

  const { snapshot } = state
  const sampleAge = formatSampleAge(snapshot.throughput_sample_age_ms)
  const period = snapshot.throughput_period_secs ?? null
  const networkDetail = period != null && snapshot.throughput_sample_age_ms != null
    ? `heartbeat ${sampleAge}, ${period}s window`
    : sampleAge

  return (
    <Card style={{ padding: 18 }}>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 14, marginBottom: 14 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 10, minWidth: 0 }}>
          <div style={{ width: 32, height: 32, borderRadius: 8, background: T.amberBg, border: '1px solid oklch(0.86 0.07 90)', display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
            <NavIcon name="chart" size={15} color={T.amberDeep} />
          </div>
          <div style={{ minWidth: 0 }}>
            <div style={{ fontSize: 13, fontWeight: 600, color: T.ink }}>App Activity</div>
            <div style={{ fontSize: 11, color: T.ink3, fontFamily: T.fontMono, marginTop: 2, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' as const }}>
              {snapshot.process_name || 'Beebeeb'} · PID {snapshot.pid}
            </div>
          </div>
        </div>
        <Chip tone="neutral">Updated {formatSampleAge(Date.now() - state.updatedAt)}</Chip>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(145px, 1fr))', gap: 12 }}>
        <ActivityMetric
          label="CPU"
          value={formatCpuPercent(snapshot.cpu_percent)}
          detail="current process"
          icon="bolt"
          accent={T.amberDeep}
        />
        <ActivityMetric
          label="Memory RSS"
          value={formatBytes(snapshot.memory_rss_bytes)}
          detail="resident set"
          icon="device"
        />
        <ActivityMetric
          label="Upload"
          value={formatRate(snapshot.upload_bps)}
          detail={networkDetail}
          icon="cloud"
          accent={T.amberDeep}
        />
        <ActivityMetric
          label="Download"
          value={formatRate(snapshot.download_bps)}
          detail={networkDetail}
          icon="download"
        />
      </div>
    </Card>
  )
}
