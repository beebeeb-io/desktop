// Drive › Storage (DW-0712) and its dialogs: preview limit (DW-0713), downloaded-files limit (DW-0714), free up (DW-0780).
import { useState } from 'react'
import { command } from '../../../../desktopApi'
import { navigateTo } from '../../../routes'
import { actionFailure } from '../../files/model'
import { Btn, Dialog, Group, Meter, Page, Row, Slider, fmtBytes, GB, mono } from '../shell/kit'
import { FIXTURE_BREAKDOWN, FIXTURE_LOCAL, useConfig } from '../shell/data'
import { useSettings } from '../shell/context'

export function Storage() {
  const { open } = useSettings()
  const [refused, setRefused] = useState<string | null>(null)
  const { config } = useConfig()
  const b = FIXTURE_BREAKDOWN
  const limit = config.local_cache_limit_bytes || 50 * GB
  return (
    <Page title="Storage">
      <Group title="In your Beebeeb account">
        <div className="px-4 py-3">
          <div className="mb-1.5 flex justify-between text-[13px]"><span className="text-ink">{fmtBytes(b.usedBytes)} used of {fmtBytes(b.quotaBytes)}</span></div>
          <Meter label="Account storage" fraction={b.usedBytes / b.quotaBytes} amber />
        </div>
        {b.rows.map((r) => (
          <Row key={r.label} label={r.label} value={fmtBytes(r.bytes)} mono onClick={r.label === 'Trash' ? () => navigateTo({ section: 'trash' }) : undefined} />
        ))}
        <Row label="Storage insights" sub="By type, and your largest files" onClick={() => open('insights')} />
      </Group>
      <Group title="On this PC" note="Limits are per PC. Thumbnails and previews count toward your storage.">
        <Row label="Downloaded files" value={`${fmtBytes(FIXTURE_LOCAL.downloaded)} of ${fmtBytes(limit)}`} mono
          right={<><Btn onClick={() => open('free-up')}>Clear…</Btn><Btn onClick={() => open('limit-downloads')}>Change limit</Btn></>} />
        <Row label="Thumbnails and previews" value={`${fmtBytes(FIXTURE_LOCAL.previews)} of ${fmtBytes(FIXTURE_LOCAL.previewLimit)}`} mono
          right={<><Btn onClick={() => setRefused(actionFailure('clear the previews', 'unsupported').text)}>Clear</Btn><Btn onClick={() => open('limit-previews')}>Change limit</Btn></>} />
        {/* No command empties the preview cache yet: Clear says so instead of opening the limit (M3e). */}
        {refused ? <p role="alert" className="px-4 pb-2 text-[12px] text-red-text">{refused}</p> : null}
        <Row label="Available offline" value={fmtBytes(FIXTURE_LOCAL.offline)} mono href="?section=available-offline" />
      </Group>
    </Page>
  )
}

function LimitDialog({ title, id, initial, max, inUse, note }: { title: string; id: string; initial: number; max: number; inUse: string; note: string }) {
  const { close } = useSettings()
  const { update } = useConfig()
  const [gb, setGb] = useState(initial)
  return (
    <Dialog labelId={id} title={title} onCancel={close}
      actions={<><Btn onClick={close}>Cancel</Btn><Btn variant="amber" onClick={() => { if (id === 'dl-h') update({ local_cache_limit_bytes: gb * GB }); close() }}>Save</Btn></>}>
      <div className={`mb-2 text-[22px] text-ink ${mono}`}>{gb} GB</div>
      <Slider label={title} min={1} max={max} value={gb} onChange={setGb} />
      <div className={`mt-1 flex justify-between text-[12px] text-ink-3 ${mono}`}><span>1 GB</span><span>{max} GB</span></div>
      <p className="mt-3">{inUse}</p>
      <p className="mt-1 text-[12px] text-ink-3">{note}</p>
    </Dialog>
  )
}

export const PreviewLimitDialog = () => (
  <LimitDialog id="pv-h" title="Limit for thumbnails and previews" initial={20} max={93}
    inUse="3.1 GB in use. 20 GB fits about 38,000 previews." note="Runs from 1 GB to half of the free space on this PC." />
)
export const DownloadLimitDialog = () => (
  <LimitDialog id="dl-h" title="Limit for downloaded files" initial={50} max={186}
    inUse="18.4 GB in use. Lowering the limit below that removes the oldest copies straight away."
    note="A file you have open is never removed. Available offline files don't count." />
)

export function FreeUpDialog() {
  const { close } = useSettings()
  const [state, setState] = useState<'idle' | 'working' | string>('idle')
  // Free up space removes the downloaded copies (`free_up_space`, DW-0780#0); a refusal stays in the alert.
  const free = () => {
    setState('working')
    void command('free_up_space').then((r) => { if (r.ok) close(); else setState(actionFailure('free up space', r.unsupported ? 'unsupported' : 'failed').text) })
  }
  const failed = state !== 'idle' && state !== 'working' ? state : null
  return (
    <Dialog labelId="free-h" title="Free up 18.4 GB?" onCancel={close}
      actions={<><Btn onClick={close}>Cancel</Btn><Btn variant="amber" onClick={free} disabled={state === 'working'}>Free up space</Btn></>}>
      {failed ? <p role="alert" className="mb-2 text-[12px] text-red-text">{failed}</p> : null}
      <p>This removes the copies of your files stored on this PC. File Explorer shows them as online only, and they download again when you open them.</p>
      <p className="mt-2">Never removed: Available offline files and any file that is open. Nothing leaves your Beebeeb account.</p>
    </Dialog>
  )
}

export function InsightsDialog() {
  const { close } = useSettings()
  return (
    <Dialog labelId="ins-h" title="Storage insights" onCancel={close} actions={<Btn variant="amber" onClick={close}>Done</Btn>}>
      <p>By type: Photos and videos 216 GB, Files 19.5 GB.</p>
      <p className="mt-1">Largest: Render final.mov 4.1 GB, Wedding 2025.zip 3.8 GB, Archive.tar 2.2 GB.</p>
    </Dialog>
  )
}
