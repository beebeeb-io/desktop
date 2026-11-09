// Drive › Bandwidth (DW-0785 family) and the speed test (DW-0786 family): what Beebeeb is sending and receiving
// now and over the last 20 hours, this PC's sync, the speed limits and the connection test. Every figure is read
// (P1c): the speeds from `app_activity_snapshot`, the 20 hours from `get_bandwidth_history`, this PC's queue from the
// engine (`drive_transfers`, the sync summary). No command reports another device's sync to this PC yet, so the
// page says that instead of the board's devices; the board's page stays in its frames.
import { useEffect, useRef, useState } from 'react'
import { useDriveSource, type SyncSummary } from '@beebeeb/shared/drive-ui'
import { command, type AppActivitySnapshot, type BandwidthSample } from '../../../../desktopApi'
import { Btn, Choice, Group, Meter, Page, Row, Switch, fmtBytes, mono } from '../shell/kit'
import { useConfig } from '../shell/data'
import { useSettings } from '../shell/context'
import { useDriveExtras, type TransferRow } from '../../../source/extras'
import { uploadingLine } from '../../../chrome/StatusLine'
import { board } from '../../../board'
import { ErrorBanner, Pill, fmtSpeed } from './ui'
import { runSpeedTest, useSpeedTest, type TestState } from './speedtest'

export type BwVariant = 'normal' | 'idle' | 'limited' | 'empty' | 'metered'

export interface BwDevice {
  id: string; name: string; platform: string
  status: 'Syncing' | 'Up to date' | 'Paused' | "Couldn't sync"
  files?: number; doneBytes?: number; totalBytes?: number; speedBps?: number | null; reported: string
}

/** One chart bucket: megabytes moved in its 20 minutes. */
export interface Bucket { up: number; down: number }

/** The page's figures, from the reads (the running app) or from the board (a frame's variant). */
export interface BwView {
  upBps: number | null
  downBps: number | null
  upLine: string
  downLine: string
  footer: string
  devices: BwDevice[]
  /** Other devices' sync isn't reported to this PC: the running app says so under This PC. */
  othersUnknown: boolean
  chart: Bucket[] | null
  /** The chart's first bucket's start, in ms (bucket i covers 20 minutes from start + i × 20 min). */
  chartStart: number
}

/** A frame's variant draws the board's page (registered by the dev server, ../../../board.ts). */
export type BoardBandwidth = (variant: BwVariant, o: { upLimit: number; meteredOn: boolean }) => BwView

const BUCKETS = 60
const BUCKET_MS = 20 * 60 * 1000

/** The last 20 hours in 60 buckets of 20 minutes, oldest first, from the state DB's samples (MB = 10^6 bytes). */
export function bucketsFrom(samples: BandwidthSample[], now: number): Bucket[] {
  const start = now - BUCKETS * BUCKET_MS
  const out = Array.from({ length: BUCKETS }, () => ({ up: 0, down: 0 }))
  for (const s of samples) {
    const i = Math.floor((s.sampled_at * 1000 - start) / BUCKET_MS)
    if (i < 0 || i >= BUCKETS) continue
    out[i].up += s.up_bytes / 1_000_000
    out[i].down += s.down_bytes / 1_000_000
  }
  return out.map((b) => ({ up: Math.round(b.up), down: Math.round(b.down) }))
}

const hhmm = (ms: number) => new Date(ms).toTimeString().slice(0, 5)
/** "Yesterday 13:40 – 14:00": a bucket's span, said against the PC's own day. */
export function bucketSpan(start: number, i: number, now: number): string {
  const from = start + i * BUCKET_MS
  const day = new Date(from).toDateString() === new Date(now).toDateString() ? 'Today' : 'Yesterday'
  return `${day} ${hhmm(from)} – ${hhmm(from + BUCKET_MS)}`
}
/** The smallest round top the chart's tallest bar fits under (the board's is 400 MB). */
export function chartTop(data: Bucket[]): number {
  const peak = Math.max(1, ...data.map((d) => Math.max(d.up, d.down)))
  return [10, 20, 50, 100, 200, 400, 1000, 2000, 5000, 10_000, 20_000, 50_000].find((t) => t >= peak) ?? Math.ceil(peak / 10_000) * 10_000
}

const MBPS = [0, 1, 2, 5, 10, 20, 50, 100]
const kbpsToMbps = (kbps: number) => kbps / 1000
function limitOptions(kbps: number) {
  const opts = MBPS.map((m) => ({ value: String(m), label: m === 0 ? 'No limit' : `${m} Mbps` }))
  const cur = kbpsToMbps(kbps)
  // An older version's saved limit that isn't a preset is kept and shown as its own choice.
  if (!MBPS.includes(cur)) opts.push({ value: String(cur), label: `${cur} Mbps` })
  return opts
}

function Chart({ data, start, now, hover: hoverProp }: { data: Bucket[] | null; start: number; now: number; hover?: number }) {
  const [hover, setHover] = useState<number | null>(hoverProp ?? null)
  if (data === null) return <div className="grid h-[140px] place-items-center px-6 text-center text-[13px] text-ink-3">Beebeeb couldn’t read the traffic recorded on this PC.</div>
  if (!data.length || data.every((d) => d.up === 0 && d.down === 0)) return <div className="grid h-[140px] place-items-center px-6 text-center text-[13px] text-ink-3">Nothing recorded yet. Beebeeb records traffic from when it starts syncing on this PC.</div>
  const W = 600, H = 140, max = chartTop(data), bw = W / data.length
  const up = data.reduce((s, d) => s + d.up, 0), down = data.reduce((s, d) => s + d.down, 0)
  const total = (mb: number) => (mb >= 1000 ? `${(mb / 1000).toFixed(1)} GB` : `${mb} MB`)
  const busiest = data.reduce((b, d, i) => (d.up + d.down > data[b].up + data[b].down ? i : b), 0)
  const y = (v: number) => H - (Math.min(v, max) / max) * H
  const axis = [0, 10, 20, 30, 40, 50].map((i) => hhmm(start + i * BUCKET_MS)).concat('Now')
  return (
    <div>
      <div className="mb-2 flex gap-4 text-[12px] text-ink-2">
        <span className="flex items-center gap-1.5"><span className="h-2 w-2 rounded-sm bg-amber" />Upload <span className={mono}>{total(up)}</span></span>
        <span className="flex items-center gap-1.5"><span className="h-2 w-2 rounded-sm bg-ink-3" />Download <span className={mono}>{total(down)}</span></span>
      </div>
      <div className="relative">
        <svg viewBox={`0 0 ${W} ${H}`} className="h-[140px] w-full" role="img" aria-label={`Data moved in the last 20 hours: upload ${total(up)}, download ${total(down)}. Busiest: ${bucketSpan(start, busiest, now)}.`}
          onMouseLeave={() => setHover(null)} onMouseMove={(e) => { const r = e.currentTarget.getBoundingClientRect(); setHover(Math.max(0, Math.min(data.length - 1, Math.floor(((e.clientX - r.left) / r.width) * data.length)))) }}>
          {[0, 0.25, 0.5, 0.75, 1].map((g) => <line key={g} x1={0} x2={W} y1={y(g * max)} y2={y(g * max)} className="stroke-line" strokeWidth={1} vectorEffect="non-scaling-stroke" />)}
          {data.map((d, i) => (
            <g key={i}>
              <rect data-testid="bar-down" x={i * bw + 0.5} width={bw - 1} y={y(d.down)} height={H - y(d.down)} className="fill-ink-3" opacity={0.55} />
              <rect data-testid="bar-up" x={i * bw + 0.5} width={bw - 1} y={y(d.up)} height={H - y(d.up)} className="fill-amber" opacity={0.85} />
            </g>
          ))}
          {hover != null ? <line data-testid="crosshair" x1={hover * bw + bw / 2} x2={hover * bw + bw / 2} y1={0} y2={H} className="stroke-ink" strokeWidth={1} vectorEffect="non-scaling-stroke" /> : null}
        </svg>
        <div className={`pointer-events-none absolute right-0 top-0 text-[11px] text-ink-3 ${mono}`}>{total(max)}</div>
        {hover != null ? (
          <div className="absolute top-1 rounded-md border border-line bg-paper px-2 py-1 text-[12px] text-ink shadow" style={{ left: `${Math.min(62, (hover / data.length) * 100)}%` }} data-testid="chart-tip">
            {bucketSpan(start, hover, now)} · Upload <span className={mono}>{data[hover].up} MB</span> · Download <span className={mono}>{data[hover].down} MB</span>
          </div>
        ) : null}
      </div>
      <div className={`mt-1 flex justify-between text-[11px] text-ink-3 ${mono}`}>{axis.map((a, i) => <span key={i}>{a}</span>)}</div>
    </div>
  )
}

function Live({ label, figure, line }: { label: string; figure: string; line: string }) {
  return (
    <div className="flex-1 px-4 py-3" data-testid="live">
      <div className="text-[12px] text-ink-3">{label}</div>
      <div className={`text-[24px] leading-8 text-ink ${mono}`} aria-label={`${label}, ${figure === '—' ? 'nothing moving' : figure}`}>{figure}</div>
      <div className="text-[12px] leading-4 text-ink-3">{line}</div>
    </div>
  )
}

function DeviceRow({ d }: { d: BwDevice }) {
  const { go } = useSettings()
  const frac = d.totalBytes ? (d.doneBytes ?? 0) / d.totalBytes : 0
  return (
    <button type="button" onClick={() => go('devices')} className="flex w-full items-center gap-3 px-4 py-2.5 text-left hover:bg-paper-2" data-testid="device-row">
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2 text-[13px] text-ink">{d.name}<span className="text-[12px] text-ink-3">{d.platform}</span></div>
        {d.status === 'Syncing' ? (
          <>
            <div className="my-1.5"><Meter label={`${d.name} sync`} fraction={frac} amber /></div>
            <div className={`text-[12px] text-ink-3 ${mono}`}>{d.files} files · {fmtBytes(d.doneBytes ?? 0)} of {fmtBytes(d.totalBytes ?? 0)} · {fmtSpeed(d.speedBps)}</div>
          </>
        ) : null}
        <div className="text-[12px] text-ink-3">Reported {d.reported}</div>
      </div>
      <Pill tone={d.status === "Couldn't sync" ? 'red' : 'neutral'}>{d.status}</Pill>
    </button>
  )
}

export function SpeedTest({ forced, scroll }: { forced?: TestState; scroll?: boolean }) {
  const live = useSpeedTest()
  const st = forced ?? live
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => { if (scroll) ref.current?.scrollIntoView({ block: 'start' }) }, [scroll])
  const running = st.phase === 'running'
  const step = (name: string, state: 'done' | 'now' | 'wait', detail: string, bar?: { frac: number; amber?: boolean }) => (
    <div className="px-4 py-2.5" data-testid="step">
      <div className="flex items-center justify-between text-[13px]"><span className="text-ink">{name}</span><span className={`text-[12px] text-ink-3 ${state === 'done' ? mono : ''}`}>{detail}</span></div>
      {bar ? <div className="mt-1.5"><Meter label={name} fraction={bar.frac} amber={bar.amber} /></div> : null}
    </div>
  )
  return (
    <div ref={ref} data-testid="speedtest">
      <Group title="Speed test">
        <Row label="Test connection speed"
          sub={running ? 'Testing… this takes 30 to 90 seconds.' : 'Measures latency, upload and download between this PC and Beebeeb. It takes 30 to 90 seconds.'}
          right={<Btn onClick={runSpeedTest} disabled={running}>{running ? 'Running…' : st.phase === 'done' ? 'Run test again' : 'Run test'}</Btn>} />
        {st.phase === 'running' ? (
          <>
            {step('Latency', st.step === 'latency' ? 'now' : 'done', st.step === 'latency' ? 'Pinging…' : st.latency != null ? `${st.latency} ms${st.latencyRange ? ` (${st.latencyRange[0]} to ${st.latencyRange[1]} ms)` : ''}` : 'Done')}
            {step('Upload', st.step === 'upload' ? 'now' : st.step === 'download' ? 'done' : 'wait', st.step === 'latency' ? 'Waits for the pings' : st.step === 'upload' ? `${st.done} of 3` : st.up != null ? `${st.up} Mbps` : 'Done',
              st.step === 'latency' ? undefined : { frac: st.step === 'upload' ? st.done / 3 : 1, amber: true })}
            {step('Download', st.step === 'download' ? 'now' : 'wait', st.step === 'download' ? `${st.done} of 3` : 'Waits for the uploads', st.step === 'download' ? { frac: st.done / 3 } : undefined)}
          </>
        ) : null}
        {st.phase === 'done' ? (
          <div className="grid grid-cols-3 gap-2 px-4 py-3" data-testid="speed-result">
            {[['Latency', `${st.result.latencyAvg} ms`, `${st.result.latencyMin} to ${st.result.latencyMax} ms`], ['Upload', `${st.result.up.toFixed(1)} Mbps`, ''], ['Download', `${st.result.down.toFixed(1)} Mbps`, '']].map(([k, v, s]) => (
              <div key={k} data-testid="speed-figure">
                <div className="text-[12px] text-ink-3">{k}</div>
                <div className={`text-[18px] leading-6 text-ink ${mono}`}>{v}</div>
                <div className={`text-[12px] text-ink-3 ${mono}`}>{s || '✓'}</div>
              </div>
            ))}
            <p className="col-span-3 text-[12px] text-ink-3">Measured just now, from this PC to Beebeeb's servers in Europe. The test moved 60 MB.</p>
          </div>
        ) : null}
      </Group>
      {st.phase === 'error' ? (
        <div className="mt-3"><ErrorBanner title="Couldn't run the speed test." action={<Btn onClick={runSpeedTest}>Run test</Btn>}>{st.message} Check your internet connection and try again.</ErrorBanner></div>
      ) : null}
    </div>
  )
}

export interface BandwidthProps { variant?: BwVariant; hover?: number; limitPopup?: 'upload' | 'download'; test?: TestState; scrollTo?: 'limits' | 'test' }

const PHASE_STATUS: Record<SyncSummary['phase'], BwDevice['status']> = {
  checking: 'Up to date', 'up-to-date': 'Up to date', syncing: 'Syncing', paused: 'Paused', offline: 'Paused', 'needs-decision': 'Syncing', stopped: "Couldn't sync",
}

/** The page's figures in the running app, from the reads. */
export function liveView(o: { activity: AppActivitySnapshot | null; transfers: TransferRow[]; sync: SyncSummary | null; history: Bucket[] | null; start: number }): BwView {
  const { activity, transfers, sync } = o
  const downloads = transfers.filter((t) => t.direction === 'down' && t.state !== 'stopped')
  const moving = transfers.filter((t) => t.state === 'moving')
  const upBps = activity?.upload_bps ?? null
  const downBps = activity?.download_bps ?? null
  const status = sync ? PHASE_STATUS[sync.phase] : 'Up to date'
  const live = transfers.filter((t) => t.state !== 'stopped')
  const known = live.every((t) => (t.total ?? 0) > 0)
  const thisPc: BwDevice = {
    id: 'pc', name: 'This PC', platform: 'Windows', status, reported: 'just now', speedBps: upBps,
    ...(status === 'Syncing' && live.length && known ? { files: live.length, doneBytes: live.reduce((n, t) => n + t.bytes, 0), totalBytes: live.reduce((n, t) => n + (t.total ?? 0), 0) } : {}),
  }
  return {
    upBps, downBps,
    upLine: uploadingLine(transfers) ?? (sync?.phase === 'paused' ? 'Paused' : sync?.phase === 'syncing' ? 'Syncing' : 'Up to date'),
    downLine: downloads.length ? `${downloads.length.toLocaleString('en')} ${downloads.length === 1 ? 'file' : 'files'} downloading` : 'Files download when you open them',
    footer: moving.length ? 'This PC is syncing' : 'Nothing is moving',
    devices: [thisPc],
    othersUnknown: true,
    chart: o.history,
    chartStart: o.start,
  }
}

/** The reads behind the page: the speeds once a second, the engine's queue and summary, and the 20 hours once. */
function useBandwidth(): BwView {
  const source = useDriveSource()
  const extras = useDriveExtras()
  const [activity, setActivity] = useState<AppActivitySnapshot | null>(null)
  const [transfers, setTransfers] = useState<TransferRow[]>([])
  const [sync, setSync] = useState<SyncSummary | null>(null)
  const [history, setHistory] = useState<Bucket[] | null>([])
  const [now] = useState(() => Date.now())
  useEffect(() => {
    let live = true
    const tick = () => { void command<AppActivitySnapshot>('app_activity_snapshot').then((r) => { if (live && r.ok) setActivity(r.value) }) }
    tick()
    const t = setInterval(tick, 1000)
    void command<BandwidthSample[]>('get_bandwidth_history', { hours: 20 }).then((r) => { if (live) setHistory(r.ok ? bucketsFrom(r.value, now) : null) })
    const readQueue = () => extras.transfers().then((rows) => { if (live) setTransfers(rows) }, () => { if (live) setTransfers([]) })
    void readQueue()
    source.sync().then((s) => { if (live) setSync(s) }, () => undefined)
    const off = source.subscribe((e) => { if (e.kind === 'sync' && live) { setSync(e.sync); void readQueue() } })
    return () => { live = false; clearInterval(t); off() }
  }, [source, extras, now])
  return liveView({ activity, transfers, sync, history, start: now - BUCKETS * BUCKET_MS })
}

export function Bandwidth({ variant = 'normal', hover, limitPopup, test, scrollTo }: BandwidthProps = {}) {
  const { config, update } = useConfig(variant === 'limited' ? { upload_kbps_limit: 1000 } : {})
  const limitsRef = useRef<HTMLDivElement>(null)
  useEffect(() => { if (scrollTo === 'limits') limitsRef.current?.scrollIntoView({ block: 'start' }) }, [scrollTo])
  const metered = variant === 'metered'
  const syncOnMetered = config.metered !== false && !metered
  const [meteredOn, setMeteredOn] = useState(syncOnMetered)
  const upLimit = kbpsToMbps(config.upload_kbps_limit || 0)
  const live = useBandwidth()
  const drawn = board<BoardBandwidth>('bandwidth')?.(variant, { upLimit, meteredOn })
  const v = drawn ?? live
  const pick = (dir: 'upload' | 'download') => (val: string) => update(dir === 'upload' ? { upload_kbps_limit: Number(val) * 1000 } : { download_kbps_limit: Number(val) * 1000 })
  return (
    <Page title="Bandwidth" sub="What Beebeeb is sending and receiving, and how fast it may.">
      <Group title="Right now">
        <div className="flex divide-x divide-line">
          <Live label="Upload" figure={fmtSpeed(v.upBps)} line={v.upLine} />
          <Live label="Download" figure={fmtSpeed(v.downBps)} line={v.downLine} />
        </div>
        <div className="px-4 py-2 text-[12px] text-ink-3" data-testid="live-footer">{v.footer}</div>
      </Group>
      <Group title="Last 20 hours"><div className="px-4 py-3"><Chart data={v.chart} start={v.chartStart} now={v.chartStart + BUCKETS * BUCKET_MS} hover={hover} /></div></Group>
      <Group title="Your devices right now">
        {v.devices.length ? v.devices.map((d) => <DeviceRow key={d.id} d={d} />) : <div className="px-4 py-3 text-[13px] text-ink-3">Devices show here once they have synced.</div>}
        {v.othersUnknown ? <div className="px-4 py-3 text-[12px] text-ink-3" data-testid="others-unknown">Your other devices don’t report their sync to this PC in this build yet.</div> : null}
      </Group>
      <div ref={limitsRef}>
        <Group title="Limits" note={upLimit ? 'Sync takes longer while this is lower than your connection.' : metered && !meteredOn ? 'Off: sync waits on networks Windows marks as metered.' : undefined}>
          <Row label="Upload" sub={upLimit ? 'Sync takes longer while this is lower than your connection.' : undefined}
            right={<Choice key={limitPopup} label="Upload limit" value={String(upLimit)} options={limitOptions(config.upload_kbps_limit || 0)} onPick={pick('upload')} open={limitPopup === 'upload'} />} />
          <Row label="Download"
            right={<Choice key={limitPopup} label="Download limit" value={String(kbpsToMbps(config.download_kbps_limit || 0))} options={limitOptions(config.download_kbps_limit || 0)} onPick={pick('download')} open={limitPopup === 'download'} />} />
          {(
            <Row label="Sync on metered connections" sub={metered && !meteredOn ? 'Off: sync waits on networks Windows marks as metered' : 'When off, Beebeeb waits for Wi-Fi or Ethernet.'}
              right={<Switch label="Sync on metered connections" on={meteredOn} onChange={(on) => { setMeteredOn(on); update({ metered: on }) }} />} />
          )}
        </Group>
      </div>
      <SpeedTest forced={test} scroll={scrollTo === 'test'} />
    </Page>
  )
}
