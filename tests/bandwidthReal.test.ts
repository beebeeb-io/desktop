// Bandwidth's chart is the history read, not the board's example series. An empty read names what is missing.
// A failed read names the error. Neither one draws a flat line of zeros.
import { expect, test } from 'bun:test'
import type { ReactElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import type { AppActivitySnapshot, BandwidthSample } from '../src/desktopApi'
import * as bandwidth from '../src/v2/sections/settings/today/bandwidth'
import { chartBuckets, type BwVariant } from '../src/v2/source/fixtures/settings-today'
import { elementsOf, expand, textOf } from './fixtures/componentHarness'

const { bucketsFrom, bucketSpan, chartTop, liveView } = bandwidth

const NOW = Date.parse('2026-11-09T12:00:00Z')
const BUCKET_MS = 20 * 60 * 1000
const START = NOW - 60 * BUCKET_MS
const BOARD: BwVariant[] = ['normal', 'idle', 'limited', 'metered']

const sample = (agoSec: number, up: number, down: number): BandwidthSample => ({
  sampled_at: Math.floor(NOW / 1000) - agoSec,
  up_bytes: up,
  down_bytes: down,
  period_secs: 20,
})

const activity = (ageMs: number | null): AppActivitySnapshot => ({
  pid: 1,
  process_name: 'beebeeb',
  cpu_percent: 0,
  memory_rss_bytes: 0,
  upload_bps: null,
  download_bps: null,
  throughput_sample_age_ms: ageMs,
  throughput_period_secs: ageMs == null ? null : 20,
})

const hhmm = (ms: number) => new Date(ms).toTimeString().slice(0, 5)

function view(history: bandwidth.Bucket[] | null, over: Record<string, unknown> = {}) {
  return liveView({
    activity: null,
    transfers: [],
    sync: null,
    history,
    start: START,
    now: NOW,
    ...over,
  } as Parameters<typeof liveView>[0])
}

function notBoard(series: { up: number; down: number }[] | null) {
  for (const variant of BOARD) expect(series ?? []).not.toEqual(chartBuckets(variant))
}

type ChartProps = {
  chart: bandwidth.Bucket[] | null
  note: string | null
  retry: boolean
  onRetry: () => void
  start: number
  now: number
}

function draw(props: ChartProps): ReactElement {
  const Chart = (bandwidth as { HistoryChart?: (props: ChartProps) => ReactElement }).HistoryChart
  expect(typeof Chart).toBe('function')
  return Chart!(props)
}

test('an empty history is not the board series and names what is missing', () => {
  for (const variant of BOARD) expect(chartBuckets(variant).some((b) => b.up > 0 || b.down > 0)).toBe(true)
  const history = bucketsFrom([], NOW)
  notBoard(history)
  const aged = view(history, { activity: activity(5 * 60 * 1000) })
  expect(aged.chartNote).toContain('No transfers yet')
  expect(aged.chartNote).toContain('Last write')
  expect(aged.chartNote).toContain(hhmm(NOW - 5 * 60 * 1000))
  expect(history).toEqual([])
  expect(chartTop(history)).toBe(0)
  expect(aged.chart).toEqual([])
  notBoard(aged.chart)
  expect(aged.chartRetry).toBe(false)
  expect(aged.upBps).toBeNull()
  expect(aged.downBps).toBeNull()

  const unread = view([])
  expect(unread.chartNote).toBe('No transfers yet. Last write: none yet.')
  expect(unread.chart).toEqual([])
  expect(unread.upLine).toBe('Not read yet')
  expect(unread.footer).toBe('Not read yet')
  expect(unread.devices.map((d) => d.name)).toEqual(['This PC'])
  expect(unread.devices[0]?.platform).toBe('')
  expect(unread.devices[0]?.status).toBe('Not read yet')
  expect(unread.devices[0]?.reported).toBe('')
  expect(unread.devices.some((d) => /Anna|iPhone|MacBook/.test(d.name))).toBe(false)

  const flat = view(Array.from({ length: 60 }, () => ({ up: 0, down: 0 })))
  expect(flat.chart).toEqual([])
  expect(flat.chartNote).toContain('No transfers yet')

  const html = renderToStaticMarkup(draw({ chart: aged.chart, note: aged.chartNote, retry: false, onRetry: () => {}, start: START, now: NOW }))
  expect(html).toContain('No transfers yet')
  expect(html).toContain(hhmm(NOW - 5 * 60 * 1000))
  expect(html).not.toContain('bar-up')
  expect(html).not.toContain('bar-down')
  expect(html).not.toContain('Try again')
  expect(html).not.toContain('240 MB')
})

test('one sample is that sample, on a scale that fits it, not the board series', () => {
  const history = bucketsFrom([sample(5 * 60, 3_000_000, 1_000_000)], NOW)
  expect(history.filter((b) => b.up > 0 || b.down > 0)).toEqual([{ up: 3, down: 1 }])
  expect(history).toHaveLength(60)
  expect(chartTop(history)).toBe(10)
  expect(chartTop(history)).not.toBe(400)
  notBoard(history)
  const i = history.findIndex((b) => b.up > 0)
  const from = START + i * BUCKET_MS
  const span = bucketSpan(START, i, NOW)
  const day = new Date(from).toDateString() === new Date(NOW).toDateString() ? 'Today' : 'Yesterday'
  expect(span.startsWith(day)).toBe(true)
  expect(span).toContain(hhmm(from))
  expect(span).toContain(hhmm(from + BUCKET_MS))
  const seen = view(history)
  expect(seen.chart).toEqual(history)
  expect(seen.chartNote).toBeNull()
  expect(seen.chartRetry).toBe(false)
  const html = renderToStaticMarkup(draw({ chart: seen.chart, note: seen.chartNote, retry: false, onRetry: () => {}, start: START, now: NOW }))
  expect(html).toContain('3 MB')
  expect(html).toContain('1 MB')
  expect(html.match(/data-testid="bar-up"/g)).toHaveLength(60)
  expect(html).not.toContain('240 MB')
  expect(html).not.toContain('No transfers yet')
})

test('many samples add inside one bucket and stay apart across buckets', () => {
  const history = bucketsFrom([
    sample(40 * 60, 6_000_000, 0),
    sample(15 * 60, 1_200_000, 400_000),
    sample(5 * 60, 800_000, 600_000),
    sample(21 * 60 * 60, 50_000_000, 50_000_000),
  ], NOW)
  const moved = history.filter((b) => b.up > 0 || b.down > 0)
  expect(moved).toEqual([{ up: 6, down: 0 }, { up: 2, down: 1 }])
  expect(history).toHaveLength(60)
  const peak = Math.max(...history.map((b) => Math.max(b.up, b.down)))
  expect(peak).toBe(6)
  expect(chartTop(history)).toBe([10, 20, 50, 100, 200, 400, 1000, 2000, 5000, 10_000, 20_000, 50_000].find((t) => t >= peak))
  notBoard(history)
  expect(history.some((b) => b.up >= 50 || b.down >= 50)).toBe(false)
  const seen = view(history)
  expect(seen.chart).toEqual(history)
  expect(seen.chartNote).toBeNull()
})

test('a sample older than the window is left out, and a failed read is the error not a zero line', () => {
  const history = bucketsFrom([
    sample(60 * 20 * 60 + 1, 50_000_000, 40_000_000),
    sample(60 * 20 * 60, 1_000_000, 0),
  ], NOW)
  expect(history.filter((b) => b.up > 0 || b.down > 0)).toEqual([{ up: 1, down: 0 }])
  expect(history).toHaveLength(60)
  expect(history.some((b) => b.up >= 50 || b.down >= 40)).toBe(false)
  notBoard(history)
  expect(chartTop(history)).toBe(10)

  const onlyOld = bucketsFrom([sample(21 * 60 * 60, 9_000_000, 8_000_000)], NOW)
  expect(onlyOld).toEqual([])
  expect(chartTop(onlyOld)).toBe(0)
  const quiet = view(onlyOld, { activity: activity(21 * 60 * 60 * 1000) })
  expect(quiet.chart).toEqual([])
  expect(quiet.chartNote).toContain('No transfers yet')
  expect(quiet.chartNote).toContain(hhmm(NOW - 21 * 60 * 60 * 1000))
  notBoard(quiet.chart)

  let retried = 0
  const failed = view(null, { historyError: 'connection refused' })
  expect(failed.chart).toBeNull()
  expect(failed.chartNote).toBe('connection refused')
  expect(failed.chartRetry).toBe(true)
  notBoard(failed.chart)
  const tree = draw({ chart: failed.chart, note: failed.chartNote, retry: true, onRetry: () => { retried += 1 }, start: START, now: NOW })
  const html = renderToStaticMarkup(tree)
  expect(html).toContain('connection refused')
  expect(html).toContain('Try again')
  expect(html).not.toContain('bar-up')
  expect(html).not.toContain('No transfers yet')
  const retry = elementsOf(expand(tree)).filter((n) => n.props.onClick && textOf(n).trim() === 'Try again')
  expect(retry).toHaveLength(1)
  retry[0]!.props.onClick()
  expect(retried).toBe(1)
})
