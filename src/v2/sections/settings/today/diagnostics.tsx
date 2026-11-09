// General › Help › Diagnostics (DW-0790, DW-0790b): what Beebeeb is doing on this PC, and a support bundle you can
// read before you send it. Nothing is sent by Beebeeb: the person attaches the file to the draft. App activity is this
// process's own (`app_activity_snapshot`, once a second while the page is open), never the board's example (P1c).
// A row names the command it came from and the time that command answered. A failed read stays, with Try again.
import { useEffect, useState } from 'react'
import { command, commandUnavailableLabel, type AppActivitySnapshot } from '../../../../desktopApi'
import { actionFailure } from '../../files/model'
import type { ProblemReportResult } from '../../../../diagnosticsCopy'
import { Btn, Group, Page, Row, fmtBytes, mono } from '../shell/kit'
import { useSettings } from '../shell/context'
import { board } from '../../../board'
import { BackLink, ErrorBanner, InfoBanner, fmtSpeed } from './ui'

function Spark({ values, label }: { values: number[]; label: string }) {
  const max = Math.max(...values, 1)
  const pts = values.map((v, i) => `${(i / (values.length - 1)) * 100},${28 - (v / max) * 26}`).join(' ')
  return <svg viewBox="0 0 100 30" className="h-7 w-24" role="img" aria-label={`${label}, the last 20 seconds`}><polyline points={pts} fill="none" className="stroke-ink-3" strokeWidth={1.5} vectorEffect="non-scaling-stroke" /></svg>
}
/** The last 20 samples, oldest first: one a second while the page is open. */
const KEEP = 20
const ACTIVITY_SOURCE = 'app_activity_snapshot'
const REPORT_SOURCE = 'report_problem'

/** Clock time of a read, on this PC. `iso` is when the command answered. */
export function formatReadAt(iso: string): string {
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return 'Not read yet'
  const parts = new Intl.DateTimeFormat('en-GB', {
    day: 'numeric', month: 'short', year: 'numeric', hour: '2-digit', minute: '2-digit', hourCycle: 'h23',
  }).formatToParts(d)
  const get = (type: string) => parts.find((p) => p.type === type)?.value ?? ''
  const day = get('day')
  const month = get('month')
  const year = get('year')
  const hour = get('hour')
  const minute = get('minute')
  if (!day || !month || !year || !hour || !minute) return 'Not read yet'
  return `${day} ${month} ${year}, ${hour}:${minute}`
}

function readLabel(source: string, readAt: string | null): string {
  return `${source} · ${readAt ? formatReadAt(readAt) : 'Not read yet'}`
}

/** One `app_activity_snapshot` answer. `readAt` is when it answered, not a clock this page invented. */
export type ActivityRead =
  | { ok: true; value: AppActivitySnapshot; readAt: string }
  | { ok: false; unsupported: boolean; readAt: string | null }

export interface DiagnosticsProps {
  saved?: boolean
  failed?: boolean
  report?: ProblemReportResult
  /** An answer already in hand (the General dialog, or a test). Omit it and the page reads the command. */
  activity?: ActivityRead
  /** When `report_problem` answered. Omit it and a save on this page records that time. */
  reportReadAt?: string | null
}

/** The four rows from the samples: the newest value and its last 20 seconds. A figure not read yet is a dash. */
export function activityRows(samples: AppActivitySnapshot[], readAt: string | null = null): { k: string; v: string; s: number[]; source: string; readAt: string | null }[] {
  const last = samples[samples.length - 1]
  const series = (f: (a: AppActivitySnapshot) => number | null | undefined) => samples.map((a) => f(a) ?? 0)
  const row = (k: string, v: string, s: number[]) => ({ k, v, s, source: ACTIVITY_SOURCE, readAt })
  return [
    row('CPU', last ? `${last.cpu_percent.toFixed(1)} %` : '—', series((a) => a.cpu_percent)),
    row('Memory', last ? fmtBytes(last.memory_rss_bytes) : '—', series((a) => a.memory_rss_bytes)),
    row('Upload', last ? fmtSpeed(last.upload_bps ?? null) : '—', series((a) => a.upload_bps)),
    row('Download', last ? fmtSpeed(last.download_bps ?? null) : '—', series((a) => a.download_bps)),
  ]
}

/** Samples app activity once a second while mounted, unless `scripted` is the answer already. */
function useActivity(scripted: ActivityRead | undefined): { answer: ActivityRead | null; samples: AppActivitySnapshot[]; retry: () => void } {
  const [samples, setSamples] = useState<AppActivitySnapshot[]>([])
  const [answer, setAnswer] = useState<ActivityRead | null>(null)
  const [attempt, setAttempt] = useState(0)
  useEffect(() => {
    if (scripted) return
    let live = true
    const read = () => {
      void command<AppActivitySnapshot>(ACTIVITY_SOURCE).then((r) => {
        if (!live) return
        const readAt = new Date().toISOString()
        if (r.ok) {
          setAnswer({ ok: true, value: r.value, readAt })
          setSamples((s) => [...s, r.value].slice(-KEEP))
        } else setAnswer({ ok: false, unsupported: r.unsupported, readAt })
      })
    }
    read()
    const t = setInterval(read, 1000)
    return () => { live = false; clearInterval(t) }
  }, [scripted, attempt])
  const retry = () => setAttempt((n) => n + 1)
  if (scripted) return { answer: scripted, samples: scripted.ok ? [scripted.value] : [], retry }
  return { answer, samples, retry }
}

export function Diagnostics({ saved, failed, report: given, activity: activityProp, reportReadAt: givenReportAt }: DiagnosticsProps = {}) {
  const { go } = useSettings()
  // A frame's saved variant draws the board's bundle; a real save names its own file. No folder is guessed.
  const initialReport = given ?? board<ProblemReportResult>('report') ?? null
  const [report, setReport] = useState<ProblemReportResult | null>(initialReport)
  const [result, setResult] = useState<'none' | 'saved' | 'failed'>(saved && initialReport ? 'saved' : failed ? 'failed' : 'none')
  const [reportAt, setReportAt] = useState<string | null>(givenReportAt ?? null)
  const [refusal, setRefusal] = useState<string | null>(null)
  const activity = useActivity(activityProp)
  const rows = activity.answer?.ok ? activityRows(activity.samples, activity.answer.readAt) : activity.answer ? [] : activityRows([], null)
  // Export… is the desktop's own flow (DW-0790#0): `report_problem` writes the bundle, shows it in File Explorer and
  // opens the draft; the banner then says where it is. A build without it says so; nothing is claimed saved.
  const exportBundle = async () => {
    setRefusal(null)
    const r = await command<ProblemReportResult>(REPORT_SOURCE)
    if (r.ok) { setReport(r.value); setReportAt(new Date().toISOString()); setResult('saved'); return }
    setReportAt(new Date().toISOString())
    if (r.unsupported) { setResult('none'); setRefusal(actionFailure('save the support bundle', 'unsupported').text); return }
    setResult('failed')
  }
  const reveal = () => {
    // tauri-plugin-opener 2.5's `reveal_item_in_dir(paths: Vec<PathBuf>)`: a list, even of one.
    if (!report) return
    void command<void>('plugin:opener|reveal_item_in_dir', { paths: [report.path] })
      .then((r) => setRefusal(r.ok ? null : actionFailure('show it in File Explorer', r.unsupported ? 'unsupported' : 'failed').text))
  }
  const activityFailed = activity.answer != null && activity.answer.ok === false
  return (
    <Page title="Diagnostics" back={<BackLink to="general">General</BackLink>}>
      {result === 'saved' && report ? (
        <InfoBanner title="Support bundle saved." action={<Btn onClick={reveal}>Show in File Explorer</Btn>}>
          <span>{readLabel(REPORT_SOURCE, reportAt)}. Saved to <span className={`text-[12px] break-all ${mono}`}>{report.path}</span>. {report.email_opened ? 'A draft to support is opening in your email app; attach the file yourself.' : "Your email app didn't open; send the file to support@beebeeb.io yourself."}</span>
        </InfoBanner>
      ) : null}
      {refusal ? <p role="alert" className="text-[12px] text-red-text">{refusal}</p> : null}
      {result === 'failed' ? <ErrorBanner title="Couldn't save the support bundle" action={<Btn onClick={exportBundle}>Try again</Btn>}>{readLabel(REPORT_SOURCE, reportAt)}. The folder for support bundles could not be written to.</ErrorBanner> : null}
      <Group title="App activity" note={activityFailed ? undefined : activity.answer ? 'Sampled only while this page is open.' : 'Not read yet.'}>
        {activityFailed && activity.answer && !activity.answer.ok ? (
          <div className="flex items-center gap-3 px-4 py-2.5" data-testid="read-row" data-source={ACTIVITY_SOURCE} data-state="failed">
            <div className="min-w-0 flex-1">
              <div className="text-[13px] text-ink">App activity</div>
              <div className="text-[12px] leading-4 text-ink-3">{readLabel(ACTIVITY_SOURCE, activity.answer.readAt)}</div>
              <div role="alert" className="text-[12px] leading-4 text-red-text">{activity.answer.unsupported ? commandUnavailableLabel(ACTIVITY_SOURCE) : 'Beebeeb couldn’t read its activity on this PC.'}</div>
            </div>
            <Btn onClick={activity.retry}>Try again</Btn>
          </div>
        ) : rows.map((r) => (
          <div key={r.k} className="flex items-center gap-3 px-4 py-2.5" data-testid="activity-row" data-source={r.source}>
            <span className="w-24 text-[13px] text-ink">{r.k}</span>
            <span className="min-w-0 flex-1 text-[12px] leading-4 text-ink-3">{readLabel(r.source, r.readAt)}</span>
            {r.s.length > 1 ? <Spark values={r.s} label={r.k} /> : <span className="h-7 w-24" />}
            <span className={`w-24 text-right text-[13px] text-ink-2 ${mono}`}>{r.v}</span>
          </div>
        ))}
      </Group>
      <Group title="Support bundle" note="Nothing is sent by Beebeeb: you attach the file to the draft yourself.">
        <Row label="Export a support bundle" sub="Saves a file on this PC, then opens a draft to support."
          right={<Btn onClick={exportBundle}>Export…</Btn>} />
        <div className="px-4 py-3 text-[12px] leading-4 text-ink-3">
          <p>Beebeeb saves a file on this PC and opens an email to support@beebeeb.io in your own email app. You read the file first, attach it and send the email yourself.</p>
          <p className="mt-2"><span className="text-ink-2">In the bundle.</span> Queue counts, an error code and the last error message.</p>
          <p className="mt-1"><span className="text-ink-2">Removed first.</span> Paths, file and folder names, sign-in tokens. Kept: standard error words, numbers, opaque ids and the server address.</p>
          <p className="mt-1"><span className="text-ink-2">Your email.</span> The email is sent from your own address, so support sees it.</p>
        </div>
      </Group>
      <Group title="Help">
        <Row label="Test connection speed" onClick={() => go('bandwidth')} />
        <Row label="Get help" href="https://beebeeb.io/support" />
        {/* The report dialog over General, where the board draws it (DW-0790#2 → DW-0782; its Cancel returns to General). */}
        <Row label="Report a problem" onClick={() => go('general', { dialog: 'report' })} />
      </Group>
    </Page>
  )
}
