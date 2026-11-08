// Drive › Bandwidth (DW-0785 family) and the speed test (DW-0786 family): what Beebeeb is sending and receiving
// now and over the last 20 hours, each device's sync, the speed limits and the connection test.
import { useEffect, useRef, useState } from 'react'
import { Btn, Choice, Group, Meter, Page, Row, Switch, fmtBytes, mono } from '../shell/kit'
import { useConfig } from '../shell/data'
import { useSettings } from '../shell/context'
import { BW_DEVICES, chartBuckets, type BwDevice, type BwVariant } from '../../../source/fixtures/settings-today'
import { ErrorBanner, Pill, fmtSpeed } from './ui'
import { runSpeedTest, useSpeedTest, FIXTURE_RESULT, failureSentence, type TestState } from './speedtest'

const MBPS = [0, 1, 2, 5, 10, 20, 50, 100]
const kbpsToMbps = (kbps: number) => kbps / 1000
function limitOptions(kbps: number) {
  const opts = MBPS.map((m) => ({ value: String(m), label: m === 0 ? 'No limit' : `${m} Mbps` }))
  const cur = kbpsToMbps(kbps)
  // An older version's saved limit that isn't a preset is kept and shown as its own choice.
  if (!MBPS.includes(cur)) opts.push({ value: String(cur), label: `${cur} Mbps` })
  return opts
}

const NOW = 'Now'
const AXIS = ['13:40', '17:00', '20:20', '23:40', '03:00', '06:20', NOW]
const bucketTime = (i: number) => {
  const mins = (13 * 60 + 40 + i * 20) % 1440
  const t = (m: number) => `${String(Math.floor(m / 60) % 24).padStart(2, '0')}:${String(m % 60).padStart(2, '0')}`
  return `${i >= 41 ? 'Today' : 'Yesterday'} ${t(mins)} – ${t(mins + 20)}`
}

function Chart({ variant, hover: hoverProp }: { variant: BwVariant; hover?: number }) {
  const data = chartBuckets(variant)
  const [hover, setHover] = useState<number | null>(hoverProp ?? null)
  if (!data.length) return <div className="grid h-[140px] place-items-center px-6 text-center text-[13px] text-ink-3">Nothing recorded yet. Beebeeb records traffic from when it starts syncing on this PC.</div>
  const W = 600, H = 140, max = 400, bw = W / data.length
  const up = data.reduce((s, d) => s + d.up, 0), down = data.reduce((s, d) => s + d.down, 0)
  const total = (mb: number) => (mb >= 1000 ? `${(mb / 1000).toFixed(1)} GB` : `${mb} MB`)
  const busiest = data.reduce((b, d, i) => (d.up + d.down > data[b].up + data[b].down ? i : b), 0)
  const y = (v: number) => H - (Math.min(v, max) / max) * H
  return (
    <div>
      <div className="mb-2 flex gap-4 text-[12px] text-ink-2">
        <span className="flex items-center gap-1.5"><span className="h-2 w-2 rounded-sm bg-amber" />Upload <span className={mono}>{total(up)}</span></span>
        <span className="flex items-center gap-1.5"><span className="h-2 w-2 rounded-sm bg-ink-3" />Download <span className={mono}>{total(down)}</span></span>
      </div>
      <div className="relative">
        <svg viewBox={`0 0 ${W} ${H}`} className="h-[140px] w-full" role="img" aria-label={`Data moved in the last 20 hours: upload ${total(up)}, download ${total(down)}. Busiest: ${bucketTime(busiest)}.`}
          onMouseLeave={() => setHover(null)} onMouseMove={(e) => { const r = e.currentTarget.getBoundingClientRect(); setHover(Math.max(0, Math.min(data.length - 1, Math.floor(((e.clientX - r.left) / r.width) * data.length)))) }}>
          {[0, 100, 200, 300, 400].map((g) => <line key={g} x1={0} x2={W} y1={y(g)} y2={y(g)} className="stroke-line" strokeWidth={1} vectorEffect="non-scaling-stroke" />)}
          {data.map((d, i) => (
            <g key={i}>
              <rect data-testid="bar-down" x={i * bw + 0.5} width={bw - 1} y={y(d.down)} height={H - y(d.down)} className="fill-ink-3" opacity={0.55} />
              <rect data-testid="bar-up" x={i * bw + 0.5} width={bw - 1} y={y(d.up)} height={H - y(d.up)} className="fill-amber" opacity={0.85} />
            </g>
          ))}
          {hover != null ? <line data-testid="crosshair" x1={hover * bw + bw / 2} x2={hover * bw + bw / 2} y1={0} y2={H} className="stroke-ink" strokeWidth={1} vectorEffect="non-scaling-stroke" /> : null}
        </svg>
        <div className={`pointer-events-none absolute right-0 top-0 text-[11px] text-ink-3 ${mono}`}>400 MB</div>
        {hover != null ? (
          <div className="absolute top-1 rounded-md border border-line bg-paper px-2 py-1 text-[12px] text-ink shadow" style={{ left: `${Math.min(62, (hover / data.length) * 100)}%` }} data-testid="chart-tip">
            {bucketTime(hover)} · Upload <span className={mono}>{data[hover].up} MB</span> · Download <span className={mono}>{data[hover].down} MB</span>
          </div>
        ) : null}
      </div>
      <div className={`mt-1 flex justify-between text-[11px] text-ink-3 ${mono}`}>{AXIS.map((a) => <span key={a}>{a}</span>)}</div>
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
            {step('Latency', st.step === 'latency' ? 'now' : 'done', st.step === 'latency' ? 'Pinging…' : `${st.latency ?? 24} ms (${st.latencyRange?.[0] ?? 18} to ${st.latencyRange?.[1] ?? 41} ms)`)}
            {step('Upload', st.step === 'upload' ? 'now' : st.step === 'download' ? 'done' : 'wait', st.step === 'latency' ? 'Waits for the pings' : st.step === 'upload' ? `${st.done} of 3` : `${st.up ?? 35.2} Mbps`,
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

export function Bandwidth({ variant = 'normal', hover, limitPopup, test, scrollTo }: BandwidthProps = {}) {
  const { config, update } = useConfig(variant === 'limited' ? { upload_kbps_limit: 1000 } : {})
  const limitsRef = useRef<HTMLDivElement>(null)
  useEffect(() => { if (scrollTo === 'limits') limitsRef.current?.scrollIntoView({ block: 'start' }) }, [scrollTo])
  const metered = variant === 'metered'
  const syncOnMetered = config.metered !== false && !metered
  const [meteredOn, setMeteredOn] = useState(syncOnMetered)
  const upLimit = kbpsToMbps(config.upload_kbps_limit || 0)
  const idle = variant === 'idle' || variant === 'empty'
  const upBps = idle ? null : metered ? null : upLimit ? (upLimit * 1000 * 1000) / 8 : 3.2 * 1024 * 1024
  const downBps = idle ? null : 410 * 1024
  const upLine = idle ? (variant === 'empty' ? '' : 'Up to date · last change 2 min ago')
    : metered ? (meteredOn ? 'Syncing' : 'Waiting: this PC is on a metered connection · 7 changes wait')
      : upLimit ? `Limited to ${upLimit} Mbps · 12 files, 48 MB left` : 'Encrypting and uploading 12 files · 48 MB left'
  const downLine = idle ? (variant === 'empty' ? '' : 'Files download when you open them') : metered ? 'Files you open still download' : "2 files from Anna's iPhone"
  const devices = variant === 'empty' ? [] : idle ? BW_DEVICES.map((d) => (d.status === 'Syncing' ? { ...d, status: 'Up to date' as const } : d))
    : metered ? BW_DEVICES.map((d) => (d.id === 'pc' ? { ...d, status: 'Paused' as const } : d)) : BW_DEVICES
  const syncing = devices.filter((d) => d.status === 'Syncing').length
  const pick = (dir: 'upload' | 'download') => (v: string) => update(dir === 'upload' ? { upload_kbps_limit: Number(v) * 1000 } : { download_kbps_limit: Number(v) * 1000 })
  return (
    <Page title="Bandwidth" sub="What Beebeeb is sending and receiving, and how fast it may.">
      <Group title="Right now">
        <div className="flex divide-x divide-line">
          <Live label="Upload" figure={fmtSpeed(upBps)} line={upLine} />
          <Live label="Download" figure={fmtSpeed(downBps)} line={downLine} />
        </div>
        <div className="px-4 py-2 text-[12px] text-ink-3" data-testid="live-footer">
          {variant === 'empty' ? 'Nothing is moving' : idle ? 'Nothing is moving' : metered && !meteredOn ? 'Waiting for another network' : `${syncing} ${syncing === 1 ? 'device is' : 'devices are'} syncing`}
        </div>
      </Group>
      <Group title="Last 20 hours"><div className="px-4 py-3"><Chart variant={variant} hover={hover} /></div></Group>
      <Group title="Your devices right now">
        {devices.length ? devices.map((d) => <DeviceRow key={d.id} d={d} />) : <div className="px-4 py-3 text-[13px] text-ink-3">Devices show here once they have synced.</div>}
      </Group>
      <div ref={limitsRef}>
        <Group title="Limits" note={upLimit ? 'Sync takes longer while this is lower than your connection.' : metered && !meteredOn ? 'Off: sync waits on networks Windows marks as metered.' : undefined}>
          <Row label="Upload" sub={upLimit ? 'Sync takes longer while this is lower than your connection.' : undefined}
            right={<Choice key={limitPopup} label="Upload limit" value={String(upLimit)} options={limitOptions(config.upload_kbps_limit || 0)} onPick={pick('upload')} open={limitPopup === 'upload'} />} />
          <Row label="Download"
            right={<Choice key={limitPopup} label="Download limit" value={String(kbpsToMbps(config.download_kbps_limit || 0))} options={limitOptions(config.download_kbps_limit || 0)} onPick={pick('download')} open={limitPopup === 'download'} />} />
          {(
            <Row label="Sync on metered connections" sub={metered && !meteredOn ? 'Off: sync waits on networks Windows marks as metered' : 'When off, Beebeeb waits for Wi-Fi or Ethernet.'}
              right={<Switch label="Sync on metered connections" on={meteredOn} onChange={(v) => { setMeteredOn(v); update({ metered: v }) }} />} />
          )}
        </Group>
      </div>
      <SpeedTest forced={test} scroll={scrollTo === 'test'} />
    </Page>
  )
}

export const TEST_STATES: Record<string, TestState> = {
  running: { phase: 'running', step: 'upload', done: 2, latency: 24, latencyRange: [18, 41] },
  downloads: { phase: 'running', step: 'download', done: 1, latency: 24, latencyRange: [18, 41], up: 35.2 },
  done: { phase: 'done', result: FIXTURE_RESULT },
  error: { phase: 'error', message: failureSentence('ping') },
}
