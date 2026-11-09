// About › Updates (DW-0789 family): release channel with what each does, the version a channel is at, and the
// notes of a version before downloading it. Beebeeb never installs an update by itself.
import { useState } from 'react'
import { command, type ManualUpdateCheckResult } from '../../../../desktopApi'
import { useCapabilities } from '../../../../capabilities'
import { runUpdate, updateCopyFor } from '../../../update'
import { actionFailure } from '../../files/model'
import { noteUpdate } from '../shell/updates'
import { Btn, Choice, Dialog, Group, Page, Row } from '../shell/kit'
import { useConfig } from '../shell/data'
import { useSettings } from '../shell/context'
import { CHANNEL_NEWEST, RELEASE_NOTES } from '../../../source/fixtures/settings-today'
import type { ReleaseChannel } from '../../../../desktopApi'
import { BackLink, InfoBanner } from './ui'

const VERSION = '0.9.1', BUILD = '311', CORE = '2084927', RUNNING_FROM: ReleaseChannel = 'stable'
const CHANNELS: { value: ReleaseChannel; label: string; sub: string }[] = [
  { value: 'stable', label: 'Stable', sub: 'Regular releases after validation.' },
  { value: 'beta', label: 'Beta', sub: 'Earlier builds for release testing.' },
  { value: 'alpha', label: 'Alpha', sub: 'Earliest builds for rapid feedback.' },
]
const older = (a: string, b: string) => { const x = a.split('.').map(Number), y = b.split('.').map(Number); for (let i = 0; i < 3; i++) if ((x[i] ?? 0) !== (y[i] ?? 0)) return (x[i] ?? 0) < (y[i] ?? 0); return false }
const label = (c: ReleaseChannel) => c[0].toUpperCase() + c.slice(1)

export function AboutToday({ runningFrom = RUNNING_FROM }: { runningFrom?: ReleaseChannel } = {}) {
  const { open, go } = useSettings()
  const { config, update } = useConfig()
  const channel = config.release_channel ?? 'stable'
  const [check, setCheck] = useState<'idle' | 'checking'>('idle')
  const [checkedAt, setCheckedAt] = useState<string | null>(null)
  const [refusal, setRefusal] = useState<string | null>(null)
  const [copied, setCopied] = useState(false)
  const newest = CHANNEL_NEWEST[channel]
  const behind = older(newest, VERSION)
  const pickChannel = (c: ReleaseChannel) => { update({ release_channel: c }); void doCheck() }
  // Check now asks the updater (DW-0751#1): a newer version shows on General with its update button (DW-0781) and in
  // the flyout; none says when it checked; a check that fails says so. Never a pretend check.
  const doCheck = async () => {
    setCheck('checking')
    setRefusal(null)
    const r = await command<ManualUpdateCheckResult>('check_for_updates_now')
    setCheck('idle')
    if (!r.ok) { setRefusal(actionFailure('check for updates', r.unsupported ? 'unsupported' : 'failed').text); return }
    if (r.value.status === 'update_available') { noteUpdate(r.value.version); go('general'); return }
    setCheckedAt(new Date().toTimeString().slice(0, 5))
  }
  return (
    <Page title="About Beebeeb" back={<BackLink to="general">Back</BackLink>}>
      <Group>
        <Row label="Beebeeb Drive" sub="End-to-end encrypted storage"
          right={<Btn onClick={() => { void navigator.clipboard?.writeText(`Beebeeb Drive ${VERSION} (${BUILD})\nEncryption core ${CORE}`); setCopied(true) }}>{copied ? 'Copied' : 'Copy'}</Btn>} />
        <Row label="Version" value={`${VERSION} (${BUILD})`} mono />
        <Row label="Encryption core" value={CORE} mono />
      </Group>
      <Group title="Updates" note="Beebeeb checks for updates at launch and every 4 hours.">
        <Row label="Check for updates" sub={check === 'checking' ? 'Checking…' : refusal ?? (behind ? `The ${label(channel)} channel is at version ${newest}` : `${VERSION} is the newest version${checkedAt ? ` · checked today, ${checkedAt}` : ''}`)}
          right={<Btn onClick={doCheck} disabled={check === 'checking'}>Check now</Btn>} />
        <Row label="Release channel" sub={CHANNELS.find((c) => c.value === channel)?.sub}
          right={<Choice label="Release channel" value={channel} options={CHANNELS} onPick={pickChannel} />} />
        <Row label="Release notes" onClick={() => open('release-notes')} />
      </Group>
      {behind ? (
        <InfoBanner title={`The ${label(channel)} channel is at version ${newest}`}>
          This PC runs {VERSION}, from the {label(runningFrom)} channel. Read the <a href="https://beebeeb.io/releases" className="underline">release notes</a> before you go back to an older version. Nothing is installed.
        </InfoBanner>
      ) : null}
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

export function ReleaseNotesDialog({ arg }: { arg?: string }) {
  const empty = arg === 'empty'
  const { close } = useSettings()
  const { config } = useConfig()
  const caps = useCapabilities()
  const copy = updateCopyFor(caps)
  const [refusal, setRefusal] = useState<string | null>(null)
  const ch = config.release_channel ?? 'stable'
  // The platform's update button (lead ruling M3e item 1): Windows restarts into it, a Mac downloads it (DW-0789b#0).
  const run = () => { void runUpdate(copy, caps).then((r) => setRefusal(r.ok ? null : actionFailure('update Beebeeb', r.unsupported ? 'unsupported' : 'failed').text)) }
  return (
    <Dialog labelId="notes-h" title="Beebeeb 0.9.2" wide onCancel={close}
      actions={<><Btn onClick={close}>Close</Btn><Btn variant="amber" onClick={run}>{copy.label}</Btn></>}>
      {refusal ? <p role="alert" className="mb-2 text-[12px] text-red-text">{refusal}</p> : null}
      <p className="text-[12px] text-ink-3">{label(ch)} channel · released 4 Nov 2026</p>
      {empty ? <p className="mt-2">No release notes were provided for this version.</p> : (
        <ul className="mt-2 list-disc pl-5">{RELEASE_NOTES.map((n) => <li key={n}>{n}</li>)}</ul>
      )}
      <p className="mt-3 text-[12px] text-ink-3">{copy.line}</p>
    </Dialog>
  )
}
