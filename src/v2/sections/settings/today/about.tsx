// About › Updates (DW-0789 family): release channel with what each does, the version a channel is at, and the
// notes of a version before downloading it. Beebeeb never installs an update by itself.
import { useState } from 'react'
import { command } from '../../../../desktopApi'
import { hasTauri } from '../../../source'
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
  const { open } = useSettings()
  const { config, update } = useConfig()
  const channel = config.release_channel ?? 'stable'
  const [check, setCheck] = useState<'idle' | 'checking'>('idle')
  const [copied, setCopied] = useState(false)
  const newest = CHANNEL_NEWEST[channel]
  const behind = older(newest, VERSION)
  const pickChannel = (c: ReleaseChannel) => { update({ release_channel: c }); void doCheck() }
  const doCheck = async () => {
    setCheck('checking')
    if (hasTauri()) await command('check_for_updates_now')
    else await new Promise((r) => setTimeout(r, 800))
    setCheck('idle')
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
        <Row label="Check for updates" sub={check === 'checking' ? 'Checking…' : behind ? `The ${label(channel)} channel is at version ${newest}` : `${VERSION} is the newest version · checked today, 09:12`}
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
  const ch = config.release_channel ?? 'stable'
  return (
    <Dialog labelId="notes-h" title="Beebeeb 0.9.2" wide onCancel={close}
      actions={<><Btn onClick={close}>Close</Btn><Btn variant="amber" onClick={close}>Download update</Btn></>}>
      <p className="text-[12px] text-ink-3">{label(ch)} channel · released 4 Nov 2026</p>
      {empty ? <p className="mt-2">No release notes were provided for this version.</p> : (
        <ul className="mt-2 list-disc pl-5">{RELEASE_NOTES.map((n) => <li key={n}>{n}</li>)}</ul>
      )}
      <p className="mt-3 text-[12px] text-ink-3">Download it, then open it to install. Beebeeb never installs an update by itself.</p>
    </Dialog>
  )
}
