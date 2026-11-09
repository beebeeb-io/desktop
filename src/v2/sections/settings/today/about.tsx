// About › Updates (DW-0789 family): release channel with what each does, the version a channel is at, and the
// notes of a version before downloading it. Beebeeb never installs an update by itself.
import { useEffect, useRef, useState, useSyncExternalStore } from 'react'
import { command, type ManualUpdateCheckResult } from '../../../../desktopApi'
import { useCapabilities } from '../../../../capabilities'
import { runUpdate, updateCopyFor } from '../../../update'
import { actionFailure } from '../../files/model'
import { hasTauri } from '../../../source'
import { noteUpdate } from '../shell/updates'
import { Btn, Choice, Dialog, Group, Page, Row } from '../shell/kit'
import { useConfig } from '../shell/data'
import { useSettings } from '../shell/context'
import type { ReleaseChannel } from '../../../../desktopApi'
import { BackLink, InfoBanner } from './ui'
import { openExternal } from '../../../external'

/** The website's release notes: the OS opens them, never this window (#274 review), and the fallback when a check
 *  answered without a notes address. */
const RELEASES_URL = 'https://beebeeb.io/releases'

/** A new release channel: saved first, then checked, because Rust's check reads the channel from disk (#274 review);
 *  a write that failed checks nothing. */
export async function pickThenCheck(save: () => Promise<boolean>, check: () => Promise<void>, refuse: () => void): Promise<void> {
  if (await save()) await check()
  else refuse()
}

/** What the last check offered in notes: only the answer's own version, channel, body and address (never a guess). */
interface NotesOffer { version: string; channel: ReleaseChannel; body: string; url: string }
let offer: NotesOffer | null = null
const offerSubs = new Set<() => void>()
const setOffer = (next: NotesOffer | null) => { offer = next; offerSubs.forEach((f) => f()) }
/** The dialog is opened by key, away from the page that ran the check, so the offer lives here. */
const useOffer = () => useSyncExternalStore((f) => { offerSubs.add(f); return () => { offerSubs.delete(f) } }, () => offer)

const CHANNELS: { value: ReleaseChannel; label: string; sub: string }[] = [
  { value: 'stable', label: 'Stable', sub: 'Regular releases after validation.' },
  { value: 'beta', label: 'Beta', sub: 'Earlier builds for release testing.' },
  { value: 'alpha', label: 'Alpha', sub: 'Earliest builds for rapid feedback.' },
]
const label = (c: ReleaseChannel) => c[0].toUpperCase() + c.slice(1)

export function AboutToday({ runningFrom }: { runningFrom?: ReleaseChannel } = {}) {
  const { open, go } = useSettings()
  const { config, update } = useConfig()
  const channel = config.release_channel ?? 'stable'
  const [check, setCheck] = useState<'idle' | 'checking'>('idle')
  const [checkedAt, setCheckedAt] = useState<string | null>(null)
  const [refusal, setRefusal] = useState<string | null>(null)
  const [copied, setCopied] = useState(false)
  // The running version (`app_version`) only: null until it answers, and when the read fails the line says unknown.
  const [live, setLive] = useState<string | null>(null)
  useEffect(() => { void command<string>('app_version').then((r) => { if (r.ok) setLive(r.value) }) }, [])
  // An older version on the channel picked: the updater's `downgrade_available` answer, never a guess.
  const [down, setDown] = useState<{ version: string; from: ReleaseChannel } | null>(null)
  const versionLine = live ?? 'unknown'
  const newest = down?.version ?? versionLine
  const behind = down !== null
  const from = down?.from ?? runningFrom ?? channel
  const pickChannel = (c: ReleaseChannel) => {
    void pickThenCheck(() => update({ release_channel: c }), doCheck, () => setRefusal(actionFailure('change the release channel', 'failed').text))
  }
  // Check now asks the updater (DW-0751#1): a newer version shows on General with its update button (DW-0781) and in
  // the flyout; an older one on the channel picked shows here; none says when it checked; a check that fails says so.
  const doCheck = async () => {
    setCheck('checking')
    setRefusal(null)
    const r = await command<ManualUpdateCheckResult>('check_for_updates_now')
    setCheck('idle')
    if (!r.ok) { setRefusal(actionFailure('check for updates', r.unsupported ? 'unsupported' : 'failed').text); return }
    if (r.value.status === 'update_available') {
      setOffer({ version: r.value.version, channel: r.value.channel, body: r.value.body, url: r.value.release_notes_url })
      noteUpdate(r.value.version); go('general'); return
    }
    // A later answer of none (or an older one) ends an offer an earlier check made (another channel's).
    noteUpdate(null)
    setDown(r.value.status === 'downgrade_available' ? { version: r.value.version, from: r.value.current_channel } : null)
    setOffer(r.value.status === 'downgrade_available' ? { version: r.value.version, channel: r.value.channel, body: r.value.body, url: r.value.release_notes_url } : null)
    setCheckedAt(new Date().toTimeString().slice(0, 5))
  }
  // The menu's Check for Updates opens this page and asks for a check (native_menu.rs): drained once here, and again
  // while the page is open.
  const checkRef = useRef(doCheck)
  checkRef.current = doCheck
  useEffect(() => {
    let alive = true
    const drain = () => { void command<boolean>('consume_menu_update_check').then((r) => { if (alive && r.ok && r.value) void checkRef.current() }) }
    drain()
    let off: (() => void) | undefined
    if (hasTauri()) {
      void import('@tauri-apps/api/event').then(({ listen }) => listen('menu:check-for-updates', drain)).then((stop) => { if (alive) off = stop; else stop() })
    }
    return () => { alive = false; off?.() }
  }, [])
  return (
    <Page title="About Beebeeb" back={<BackLink to="general">Back</BackLink>}>
      <Group>
        <Row label="Beebeeb Drive" sub="End-to-end encrypted storage"
          right={<Btn onClick={() => { void navigator.clipboard?.writeText(`Beebeeb Drive${live ? ` ${live}` : ''}`); setCopied(true) }}>{copied ? 'Copied' : 'Copy'}</Btn>} />
        <Row label="Version" value={versionLine} mono />
      </Group>
      <Group title="Updates" note="Beebeeb checks for updates at launch and every 4 hours.">
        <Row label="Check for updates" sub={refusal ?? (behind ? `The ${label(channel)} channel is at version ${newest}` : live ? `${live} is the newest version${checkedAt ? ` · checked today, ${checkedAt}` : ''}` : 'Check for a newer version now.')}
          right={<Btn onClick={doCheck} disabled={check === 'checking'}>Check now</Btn>} />
        <Row label="Release channel" sub={CHANNELS.find((c) => c.value === channel)?.sub}
          right={<Choice label="Release channel" value={channel} options={CHANNELS} onPick={pickChannel} />} />
        <Row label="Release notes" onClick={() => open('release-notes')} />
      </Group>
      {behind ? (
        <InfoBanner title={`The ${label(channel)} channel is at version ${newest}`}>
          This PC runs {versionLine}, from the {label(from)} channel. Read the <a href={RELEASES_URL} onClick={(e) => { e.preventDefault(); void openExternal(RELEASES_URL).then((r) => { if (!r.ok) setRefusal(actionFailure('open the release notes', r.unsupported ? 'unsupported' : 'failed').text) }) }} className="underline">release notes</a> before you go back to an older version. Nothing is installed.
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
  const offers = useOffer()
  const caps = useCapabilities()
  const copy = updateCopyFor(caps)
  const [refusal, setRefusal] = useState<string | null>(null)
  const ch = offers?.channel ?? config.release_channel ?? 'stable'
  // Only what the check answered: its version, its channel, its body split into lines, and its notes address.
  const lines = offers?.body.split('\n').map((l) => l.trim()).filter(Boolean) ?? []
  const url = offers?.url || RELEASES_URL
  // The platform's update button (lead ruling M3e item 1): Windows restarts into it, a Mac downloads it (DW-0789b#0).
  const run = () => { void runUpdate(copy, caps).then((r) => setRefusal(r.ok ? null : actionFailure('update Beebeeb', r.unsupported ? 'unsupported' : 'failed').text)) }
  return (
    <Dialog labelId="notes-h" title={offers ? `Beebeeb ${offers.version}` : 'Release notes'} wide onCancel={close}
      actions={<><Btn onClick={close}>Close</Btn><Btn variant="amber" onClick={run}>{copy.label}</Btn></>}>
      {refusal ? <p role="alert" className="mb-2 text-[12px] text-red-text">{refusal}</p> : null}
      {offers ? <p className="text-[12px] text-ink-3">{label(ch)} channel</p> : null}
      {empty || (offers && lines.length === 0) ? <p className="mt-2">No release notes were provided for this version.</p>
        : lines.length ? <ul className="mt-2 list-disc pl-5">{lines.map((n) => <li key={n}>{n}</li>)}</ul>
        : <p className="mt-2">Check for updates to read the release notes for a version.</p>}
      <p className="mt-3 text-[12px] text-ink-3">
        <a href={url} onClick={(e) => { e.preventDefault(); void openExternal(url).then((r) => { if (!r.ok) setRefusal(actionFailure('open the release notes', r.unsupported ? 'unsupported' : 'failed').text) }) }} className="underline">Release notes on beebeeb.io</a>
      </p>
      <p className="mt-3 text-[12px] text-ink-3">{copy.line}</p>
    </Dialog>
  )
}
