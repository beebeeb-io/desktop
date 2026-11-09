// General (DW-0701, DW-0781), the problem report (DW-0782) and the plan dialog (DW-0708). About is today's
// (../today/about.tsx). The version and the plan are read (`app_version`, `account_subscription`), never the board's (P1c).
import { useState } from 'react'
import { BILLING_URL, command, type Subscription } from '../../../../desktopApi'
import type { ProblemReportResult } from '../../../../diagnosticsCopy'
import { useCapabilities } from '../../../../capabilities'
import { openExternal } from '../../../external'
import { runUpdate, updateCopyFor } from '../../../update'
import { actionFailure } from '../../files/model'
import { useUpdateOffer } from './updates'
import { Btn, Dialog, Group, Meter, Page, Row, Choice, fmtBytes, mono } from './kit'
import { useConfig, useRead } from './data'
import { useSettings } from './context'
import { startGuide } from '../../../guide/Guide'
import { board } from '../../../board'
import type { DesktopTheme } from '../../../../desktopApi'

/** The running version (`app_version`); in a frame, the board's. Null until it answers, and when it can't. */
export function useAppVersion(): string | null {
  const live = useRead<string>('app_version')
  return live.state === 'ready' ? live.value : board<{ version: string }>('about')?.version ?? null
}

/** DW-0781: a newer version, with the platform's update button (lead ruling M3e item 1). A refusal is said in the banner. */
export function UpdateBanner({ version }: { version: string }) {
  const caps = useCapabilities()
  const copy = updateCopyFor(caps)
  const [state, setState] = useState<'idle' | 'working' | string>('idle')
  const go = () => {
    setState('working')
    void runUpdate(copy, caps).then((r) => setState(r.ok ? 'idle' : actionFailure('update Beebeeb', r.unsupported ? 'unsupported' : 'failed').text))
  }
  const failed = state !== 'idle' && state !== 'working' ? state : null
  return (
    <div role="status" className="flex items-center gap-4 rounded-[12px] border border-line bg-paper-2 px-4 py-3" data-testid="update-banner">
      <div className="flex-1">
        <div className="text-[13px] font-medium text-ink">Version {version} is available</div>
        <div className="text-[12px] text-ink-3">{copy.line}</div>
        {failed ? <div role="alert" className="mt-1 text-[12px] text-red-text">{failed}</div> : null}
      </div>
      <Btn variant="amber" onClick={go} disabled={state === 'working'}>{copy.label}</Btn>
    </div>
  )
}

const THEMES: { value: DesktopTheme; label: string }[] = [
  { value: 'system', label: 'System' }, { value: 'light', label: 'Light' }, { value: 'dark', label: 'Dark' },
]

export function General({ updateAvailable }: { updateAvailable?: string } = {}) {
  const { go, open } = useSettings()
  const { config, update } = useConfig()
  const offer = useUpdateOffer()
  const running = useAppVersion()
  // The version the updater noted; a frame's variant (DW-0781) names the board's.
  const version = offer ?? updateAvailable ?? null
  const newest = version !== null
  return (
    <Page title="General">
      {version ? <UpdateBanner version={version} /> : null}
      <Group title="Appearance">
        <Row label="Theme" sub={config.theme === 'system' || !config.theme ? 'Follows Windows.' : undefined}
          right={<Choice label="Theme" value={config.theme ?? 'system'} options={THEMES} onPick={(theme) => update({ theme })} />} />
      </Group>
      <Group title="Help">
        <Row label="Show the guide again" onClick={startGuide} />
        <Row label="Help and report a problem" onClick={() => open('report')} />
        <Row label="Diagnostics" href="?section=settings&page=diagnostics" />
      </Group>
      <Group title="Privacy and data">
        <Row label="Privacy" href="https://beebeeb.io/privacy" />
        <Row label="Export my data" href="https://app.beebeeb.io/settings/data" />
        <Row label="Storage location" value="Europe" sub="Where new files are stored: the account's region." onClick={() => open('region')} />
      </Group>
      <Group title="About">
        <Row label="Terms and privacy policy" href="https://beebeeb.io/terms" />
        <Row label="About Beebeeb" value={running ? (newest ? `${running} · update available` : `Version ${running}`) : newest ? 'Update available' : undefined} mono onClick={() => go('about')} />
      </Group>
    </Page>
  )
}

/**
 * DW-0782: Save and write email saves the support bundle, shows it in File Explorer and opens the draft
 * (`report_problem`), then Diagnostics says where it is (DW-0790b). A refusal stays in the dialog; nothing is claimed.
 */
export function ReportDialog() {
  const { close, go } = useSettings()
  const [state, setState] = useState<'idle' | 'working' | string>('idle')
  const save = () => {
    setState('working')
    void command<ProblemReportResult>('report_problem').then((r) => {
      if (r.ok) { close(); go('diagnostics', { props: { saved: true, report: r.value } }); return }
      setState(actionFailure('save the support bundle', r.unsupported ? 'unsupported' : 'failed').text)
    })
  }
  const failed = state !== 'idle' && state !== 'working' ? state : null
  return (
    <Dialog labelId="report-h" title="Help and report a problem" wide onCancel={close}
      actions={<><Btn onClick={close}>Cancel</Btn><Btn variant="amber" onClick={save} disabled={state === 'working'}>Save and write email</Btn></>}>
      {failed ? <p role="alert" className="mb-2 text-[12px] text-red-text">{failed}</p> : null}
      <p>Beebeeb saves a file on this PC and opens an email to support@beebeeb.io in your own email app. You read the file first, attach it and send the email yourself.</p>
      <h3 className="mt-3 text-[13px] font-medium text-ink">In the bundle</h3>
      <p>Queue counts, an error code and the last error message.</p>
      <h3 className="mt-3 text-[13px] font-medium text-ink">Removed first</h3>
      <p>Paths, file and folder names, sign-in tokens. Kept: standard error words, numbers, opaque ids and the server address.</p>
      <h3 className="mt-3 text-[13px] font-medium text-ink">Your email</h3>
      <p>The email is sent from your own address, so support sees it.</p>
    </Dialog>
  )
}

/** "Renews 14 Mar 2027": a date in the board's day-month-year form. */
const day = (iso: string) => new Date(iso).toLocaleDateString('en-GB', { day: 'numeric', month: 'short', year: 'numeric' })

/** DW-0708: the account's plan (`account_subscription`): its storage, what is used, when it renews and how it bills. */
export function PlanDialog() {
  const { close } = useSettings()
  const [refused, setRefused] = useState<string | null>(null)
  const plan = useRead<Subscription>('account_subscription')
  // Paying happens on the website: the plan page opens in the browser (DW-0708#0), never in the window.
  const manage = () => { void openExternal(BILLING_URL).then((r) => setRefused(r.ok ? null : actionFailure('open beebeeb.io', r.unsupported ? 'unsupported' : 'failed').text)) }
  const p = plan.state === 'ready' ? plan.value : null
  const cycle = p?.billing_cycle ? p.billing_cycle[0].toUpperCase() + p.billing_cycle.slice(1) : null
  return (
    <Dialog labelId="plan-h" title="Your plan" onCancel={close}
      actions={<><Btn onClick={manage}>Manage on beebeeb.io</Btn><Btn variant="amber" onClick={close}>Done</Btn></>}>
      {refused ? <p role="alert" className="mb-2 text-[12px] text-red-text">{refused}</p> : null}
      {p ? (
        <>
          <div className="flex items-baseline justify-between"><span className="text-[15px] font-medium text-ink">{p.quota_bytes ? fmtBytes(p.quota_bytes) : 'No storage yet'}</span><span className={mono}>{fmtBytes(p.used_bytes)} used</span></div>
          {p.quota_bytes ? <div className="my-2"><Meter label="Storage used" fraction={p.used_bytes / p.quota_bytes} amber /></div> : null}
          {p.current_period_end ? <div className="flex justify-between"><span>Renews</span><span className={mono}>{day(p.current_period_end)}</span></div> : null}
          {cycle ? <div className="flex justify-between"><span>Billing</span><span>{cycle}</span></div> : null}
        </>
      ) : plan.state === 'failed' ? (
        <p role="alert" className="text-[12px] text-red-text">{actionFailure('read your plan', 'failed').text}</p>
      ) : <p className="text-ink-3" aria-busy="true">Reading your plan…</p>}
      <p className="mt-3 text-[12px] text-ink-3">Everything about paying happens on the website.</p>
    </Dialog>
  )
}

export function RegionDialog() {
  const { close } = useSettings()
  return (
    <Dialog labelId="region-h" title="Storage location" onCancel={close} actions={<Btn variant="amber" onClick={close}>Done</Btn>}>
      <p>New files are stored in Europe: data centers in Germany and Finland. This is your account's region.</p>
    </Dialog>
  )
}
