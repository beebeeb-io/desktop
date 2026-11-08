// General › Help › Diagnostics (DW-0790, DW-0790b): what Beebeeb is doing on this PC, and a support bundle you can
// read before you send it. Nothing is sent by Beebeeb: the person attaches the file to the draft.
import { useEffect, useState } from 'react'
import { command } from '../../../../desktopApi'
import { hasTauri } from '../../../source'
import { Btn, Group, Page, Row, mono } from '../shell/kit'
import { useSettings } from '../shell/context'
import { BackLink, ErrorBanner, InfoBanner, fmtSpeed } from './ui'

const FILE = '%LOCALAPPDATA%\\io.beebeeb.app\\diagnostics\\beebeeb-diagnostics-1793956320.json'

function Spark({ values, label }: { values: number[]; label: string }) {
  const max = Math.max(...values, 1)
  const pts = values.map((v, i) => `${(i / (values.length - 1)) * 100},${28 - (v / max) * 26}`).join(' ')
  return <svg viewBox="0 0 100 30" className="h-7 w-24" role="img" aria-label={`${label}, the last 20 seconds`}><polyline points={pts} fill="none" className="stroke-ink-3" strokeWidth={1.5} vectorEffect="non-scaling-stroke" /></svg>
}
const wave = (base: number, amp: number, k: number) => Array.from({ length: 20 }, (_, i) => base + Math.abs(Math.sin(i * k)) * amp)

export function Diagnostics({ saved, failed }: { saved?: boolean; failed?: boolean } = {}) {
  const { open, go } = useSettings()
  const [sampleN, setSampleN] = useState(0)
  const [result, setResult] = useState<'none' | 'saved' | 'failed'>(saved ? 'saved' : failed ? 'failed' : 'none')
  const [mail, setMail] = useState(true)
  // App activity samples only while this page is open.
  useEffect(() => { const t = setInterval(() => setSampleN((n) => n + 1), 1000); return () => clearInterval(t) }, [])
  const rows = [
    { k: 'CPU', v: `${(3.1 + (sampleN % 3) * 0.1).toFixed(1)} %`, s: wave(1.5, 3, 0.7) },
    { k: 'Memory', v: '212 MB', s: wave(208, 6, 0.4) },
    { k: 'Upload', v: fmtSpeed(3.2 * 1024 * 1024), s: wave(2, 2, 0.9) },
    { k: 'Download', v: fmtSpeed(410 * 1024), s: wave(100, 400, 1.3) },
  ]
  const exportBundle = async () => {
    if (hasTauri()) { const r = await command('export_diagnostics'); if (!r.ok) return setResult('failed') }
    setResult('saved'); setMail(true)
  }
  return (
    <Page title="Diagnostics" back={<BackLink to="general">General</BackLink>}>
      {result === 'saved' ? (
        <InfoBanner title="Support bundle saved." action={<Btn onClick={() => undefined}>Show in File Explorer</Btn>}>
          <span>Saved to <span className={`text-[12px] break-all ${mono}`}>{FILE}</span>. {mail ? 'A draft to support is opening in your email app; attach the file yourself.' : "Your email app didn't open; send the file to support@beebeeb.io yourself."}</span>
        </InfoBanner>
      ) : null}
      {result === 'failed' ? <ErrorBanner title="Couldn't save the support bundle" action={<Btn onClick={exportBundle}>Try again</Btn>}>The folder for support bundles could not be written to.</ErrorBanner> : null}
      <Group title="App activity" note="Sampled only while this page is open.">
        {rows.map((r) => (
          <div key={r.k} className="flex items-center gap-3 px-4 py-2.5" data-testid="activity-row">
            <span className="flex-1 text-[13px] text-ink">{r.k}</span>
            <Spark values={r.s} label={r.k} />
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
        <Row label="Report a problem" onClick={() => open('report')} />
      </Group>
    </Page>
  )
}
