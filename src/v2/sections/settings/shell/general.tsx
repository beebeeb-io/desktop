// General (DW-0701, DW-0781), About (DW-0751), the problem report (DW-0782) and the plan dialog (DW-0708).
import { useState } from 'react'
import { BILLING_URL } from '../../../../desktopApi'
import { Btn, Dialog, Group, Meter, Page, Row, Choice, mono, GB } from './kit'
import { useConfig } from './data'
import { useSettings } from './context'
import { startGuide } from '../../../guide/Guide'
import type { DesktopTheme, ReleaseChannel } from '../../../../desktopApi'

const VERSION = '0.9.1'
const BUILD = '311'
const CORE = '2084927'

function UpdateBanner() {
  return (
    <div role="status" className="flex items-center gap-4 rounded-[12px] border border-line bg-paper-2 px-4 py-3" data-testid="update-banner">
      <div className="flex-1">
        <div className="text-[13px] font-medium text-ink">Version 0.9.2 is available</div>
        <div className="text-[12px] text-ink-3">Download it, then open it to install. Beebeeb never installs an update by itself.</div>
      </div>
      <Btn variant="amber">Download update</Btn>
    </div>
  )
}

const THEMES: { value: DesktopTheme; label: string }[] = [
  { value: 'system', label: 'System' }, { value: 'light', label: 'Light' }, { value: 'dark', label: 'Dark' },
]

export function General({ updateAvailable }: { updateAvailable?: boolean } = {}) {
  const { go, open } = useSettings()
  const { config, update } = useConfig()
  const newest = updateAvailable ?? false
  return (
    <Page title="General">
      {newest ? <UpdateBanner /> : null}
      <Group title="Appearance">
        <Row label="Theme" sub={config.theme === 'system' || !config.theme ? 'Follows Windows.' : undefined}
          right={<Choice label="Theme" value={config.theme ?? 'system'} options={THEMES} onPick={(theme) => update({ theme })} />} />
      </Group>
      <Group title="Help">
        <Row label="Show the guide again" onClick={startGuide} />
        <Row label="Help and report a problem" onClick={() => open('report')} />
        <Row label="Diagnostics" href="?section=settings&sub=diagnostics" />
      </Group>
      <Group title="Privacy and data">
        <Row label="Privacy" href="https://beebeeb.io/privacy" />
        <Row label="Export my data" href="https://app.beebeeb.io/settings/data" />
        <Row label="Storage location" value="Europe" sub="Where new files are stored: the account's region." onClick={() => open('region')} />
      </Group>
      <Group title="About">
        <Row label="Terms and privacy policy" href="https://beebeeb.io/terms" />
        <Row label="About Beebeeb" value={newest ? `${VERSION} · update available` : `Version ${VERSION}`} mono onClick={() => go('about')} />
      </Group>
    </Page>
  )
}

const CHANNELS: { value: ReleaseChannel; label: string }[] = [
  { value: 'stable', label: 'Stable' }, { value: 'beta', label: 'Beta' }, { value: 'alpha', label: 'Alpha' },
]

export function About() {
  const { go } = useSettings()
  const { config, update } = useConfig()
  const [state, setState] = useState<'idle' | 'checking'>('idle')
  const [copied, setCopied] = useState(false)
  const lines = `Beebeeb Drive ${VERSION} (${BUILD})\nEncryption core ${CORE}`
  return (
    <Page title="About Beebeeb"
      back={<a href="?section=settings&sub=general" onClick={(e) => { e.preventDefault(); go('general') }} className="mb-2 inline-block text-[13px] text-ink-2 hover:underline">Back</a>}>
      <Group>
        <Row label="Beebeeb Drive" sub="End-to-end encrypted storage"
          right={<Btn onClick={() => { void navigator.clipboard?.writeText(lines); setCopied(true) }}>{copied ? 'Copied' : 'Copy'}</Btn>} />
        <Row label="Version" value={`${VERSION} (${BUILD})`} mono />
        <Row label="Encryption core" value={CORE} mono />
      </Group>
      <Group title="Updates" note="Beebeeb checks for updates at launch and every 4 hours.">
        <Row label="Check for updates" sub={state === 'checking' ? 'Checking…' : `${VERSION} is the newest version · checked today, 09:12`}
          right={<Btn onClick={() => { setState('checking'); setTimeout(() => setState('idle'), 800) }}>Check now</Btn>} />
        <Row label="Release channel" sub="Regular releases after validation."
          right={<Choice label="Release channel" value={config.release_channel ?? 'stable'} options={CHANNELS} onPick={(release_channel) => update({ release_channel })} />} />
        <Row label="Release notes" onClick={() => undefined} />
      </Group>
      <Group title="Open source">
        <Row label="Encryption core on GitHub" href="https://github.com/beebeeb-io/core" />
        <Row label="Open-source licences" href="https://beebeeb.io/licences" />
      </Group>
      <Group title="More">
        <Row label="About audits" href="https://beebeeb.io/security" />
        <Row label="beebeeb.io" href="https://beebeeb.io" />
        <Row label="Initlabs B.V." />
      </Group>
    </Page>
  )
}

export function ReportDialog() {
  const { close } = useSettings()
  return (
    <Dialog labelId="report-h" title="Help and report a problem" wide onCancel={close}
      actions={<><Btn onClick={close}>Cancel</Btn><Btn variant="amber" onClick={close}>Save and write email</Btn></>}>
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

export function PlanDialog() {
  const { close } = useSettings()
  return (
    <Dialog labelId="plan-h" title="Your plan" onCancel={close}
      actions={<><Btn onClick={() => window.open(BILLING_URL, '_blank', 'noopener')}>Manage on beebeeb.io</Btn><Btn variant="amber" onClick={close}>Done</Btn></>}>
      <div className="flex items-baseline justify-between"><span className="text-[15px] font-medium text-ink">1 TB</span><span className={mono}>240 GB used</span></div>
      <div className="my-2"><Meter label="Storage used" fraction={240 / 1024} amber /></div>
      <div className="flex justify-between"><span>Renews</span><span className={mono}>14 Mar 2027</span></div>
      <div className="flex justify-between"><span>Billing</span><span>Yearly</span></div>
      <p className="mt-3 text-[12px] text-ink-3">Everything about paying happens on the website.</p>
      <span hidden>{GB}</span>
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
