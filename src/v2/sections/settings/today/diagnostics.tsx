// General › Help › Diagnostics (DW-0790, DW-0790b): what Beebeeb is doing on this PC, and a support bundle you can
// read before you send it. Nothing is sent by Beebeeb: the person attaches the file to the draft. App activity is this
// process's own (`app_activity_snapshot`, once a second while the page is open), never the board's example (P1c).
import { useEffect, useState } from 'react'
import { command, type AppActivitySnapshot } from '../../../../desktopApi'
import { actionFailure } from '../../files/model'
import type { ProblemReportResult } from '../../../../diagnosticsCopy'
import { Btn, Group, Page, Row, mono } from '../shell/kit'
import { useSettings } from '../shell/context'
import { fmtBytes } from '../shell/kit'
import { board } from '../../../board'
import { BackLink, ErrorBanner, InfoBanner, fmtSpeed } from './ui'

function Spark({ values, label }: { values: number[]; label: string }) {
  const max = Math.max(...values, 1)
  const pts = values.map((v, i) => `${(i / (values.length - 1)) * 100},${28 - (v / max) * 26}`).join(' ')
  return <svg viewBox="0 0 100 30" className="h-7 w-24" role="img" aria-label={`${label}, the last 20 seconds`}><polyline points={pts} fill="none" className="stroke-ink-3" strokeWidth={1.5} vectorEffect="non-scaling-stroke" /></svg>
}
/** The last 20 samples, oldest first: one a second while the page is open. */
const KEEP = 20

/** Samples app activity once a second while mounted. `failed`: the read was refused (its rows say so). */
function useActivity(): { samples: AppActivitySnapshot[]; failed: boolean } {
  const [samples, setSamples] = useState<AppActivitySnapshot[]>([])
  const [failed, setFailed] = useState(false)
  useEffect(() => {
    let live = true
    const read = () => {
      void command<AppActivitySnapshot>('app_activity_snapshot').then((r) => {
        if (!live) return
        if (r.ok) { setFailed(false); setSamples((s) => [...s, r.value].slice(-KEEP)) } else setFailed(true)
      })
    }
    read()
    const t = setInterval(read, 1000)
    return () => { live = false; clearInterval(t) }
  }, [])
  return { samples, failed }
}

/** The four rows from the samples: the newest value and its last 20 seconds. A figure not read yet is a dash. */
export function activityRows(samples: AppActivitySnapshot[]): { k: string; v: string; s: number[] }[] {
  const last = samples[samples.length - 1]
  const series = (f: (a: AppActivitySnapshot) => number | null | undefined) => samples.map((a) => f(a) ?? 0)
  return [
    { k: 'CPU', v: last ? `${last.cpu_percent.toFixed(1)} %` : '—', s: series((a) => a.cpu_percent) },
    { k: 'Memory', v: last ? fmtBytes(last.memory_rss_bytes) : '—', s: series((a) => a.memory_rss_bytes) },
    { k: 'Upload', v: last ? fmtSpeed(last.upload_bps ?? null) : '—', s: series((a) => a.upload_bps) },
    { k: 'Download', v: last ? fmtSpeed(last.download_bps ?? null) : '—', s: series((a) => a.download_bps) },
  ]
}

export function Diagnostics({ saved, failed, report: given }: { saved?: boolean; failed?: boolean; report?: ProblemReportResult } = {}) {
  const { go } = useSettings()
  // A frame's saved variant draws the board's bundle; a real save names its own file.
  const [report, setReport] = useState<ProblemReportResult | null>(given ?? board<ProblemReportResult>('report') ?? null)
  const [result, setResult] = useState<'none' | 'saved' | 'failed'>(saved && report ? 'saved' : failed ? 'failed' : 'none')
  const [refusal, setRefusal] = useState<string | null>(null)
  // App activity samples only while this page is open.
  const activity = useActivity()
  const rows = activityRows(activity.samples)
  // Export… is the desktop's own flow (DW-0790#0): `report_problem` writes the bundle, shows it in File Explorer and
  // opens the draft; the banner then says where it is. A build without it says so; nothing is claimed saved.
  const exportBundle = async () => {
    setRefusal(null)
    const r = await command<ProblemReportResult>('report_problem')
    if (r.ok) { setReport(r.value); setResult('saved'); return }
    if (r.unsupported) { setResult('none'); setRefusal(actionFailure('save the support bundle', 'unsupported').text); return }
    setResult('failed')
  }
  const reveal = () => {
    // tauri-plugin-opener 2.5's `reveal_item_in_dir(paths: Vec<PathBuf>)`: a list, even of one.
    if (!report) return
    void command<void>('plugin:opener|reveal_item_in_dir', { paths: [report.path] })
      .then((r) => setRefusal(r.ok ? null : actionFailure('show it in File Explorer', r.unsupported ? 'unsupported' : 'failed').text))
  }
  return (
    <Page title="Diagnostics" back={<BackLink to="general">General</BackLink>}>
      {result === 'saved' && report ? (
        <InfoBanner title="Support bundle saved." action={<Btn onClick={reveal}>Show in File Explorer</Btn>}>
          <span>Saved to <span className={`text-[12px] break-all ${mono}`}>{report.path}</span>. {report.email_opened ? 'A draft to support is opening in your email app; attach the file yourself.' : "Your email app didn't open; send the file to support@beebeeb.io yourself."}</span>
        </InfoBanner>
      ) : null}
      {refusal ? <p role="alert" className="text-[12px] text-red-text">{refusal}</p> : null}
      {result === 'failed' ? <ErrorBanner title="Couldn't save the support bundle" action={<Btn onClick={exportBundle}>Try again</Btn>}>The folder for support bundles could not be written to.</ErrorBanner> : null}
      <Group title="App activity" note={activity.failed ? 'Beebeeb couldn’t read its activity on this PC.' : 'Sampled only while this page is open.'}>
        {rows.map((r) => (
          <div key={r.k} className="flex items-center gap-3 px-4 py-2.5" data-testid="activity-row">
            <span className="flex-1 text-[13px] text-ink">{r.k}</span>
            {r.s.length > 1 ? <Spark values={r.s} label={r.k} /> : <span className="h-7 w-24" />}
            <span className={`w-24 text-right text-[13px] text-ink-2 ${mono}`}>{r.v}</span>
          </div>
        ))}
      </Group>
      <Group title="Support bundle" note="Nothing is sent by Beebeeb: you attach the file to the draft yourself.">
        <Row label="Export a support bundle" sub="Saves a file on this PC, then opens a draft to support."
          right={<Btn onClick={exportBundle}>Export…</Btn>} />
        <div className="px-4 py-3 text-[12px] leading-4 text-ink-3">
          <p><span className="text-ink-2">In the bundle:</span> queue counts, an error code and the last error message.</p>
          <p className="mt-1"><span className="text-ink-2">Removed first:</span> paths, file and folder names and sign-in tokens.</p>
          <p className="mt-1"><span className="text-ink-2">Kept:</span> standard error words, numbers, opaque ids and the server address.</p>
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
