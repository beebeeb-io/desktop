// Drive › Storage (DW-0712) and its dialogs: preview limit (DW-0713), downloaded-files limit (DW-0714), free up (DW-0780).
// Every number is read (P1c): the account's storage from DriveSource's `storage()` (`drive_storage`), this PC's copies
// from `desktop_storage_summary`, the limit from the desktop config. What no command reads (the preview cache) says so.
import { useState } from 'react'
import { command } from '../../../../desktopApi'
import { navigateTo } from '../../../routes'
import { board } from '../../../board'
import { actionFailure } from '../../files/model'
import { Btn, Dialog, Group, Meter, Page, Row, Slider, fmtBytes, GB, mono } from '../shell/kit'
import { useAccountStorage, useConfig, useLocalSpace } from '../shell/data'
import { useSettings } from '../shell/context'

/** The board's preview cache, for a frame (no command measures it yet). */
interface BoardPreviews { bytes: number; limitGb: number; fits: string }

/** "18.4 GB of 50 GB", or "of no limit" when the limit is 0 (Rust: 0 is Unlimited). */
export function ofLimit(bytes: number, limit: number | undefined): string {
  return limit && limit > 0 ? `${fmtBytes(bytes)} of ${fmtBytes(limit)}` : `${fmtBytes(bytes)} · no limit`
}

export function Storage() {
  const { open } = useSettings()
  const [refused, setRefused] = useState<string | null>(null)
  const { config } = useConfig()
  const account = useAccountStorage()
  const local = useLocalSpace()
  const previews = board<BoardPreviews>('previews')
  const a = account.state === 'ready' ? account.value : null
  const here = local.state === 'ready' ? local.value : null
  return (
    <Page title="Storage">
      <Group title="In your Beebeeb account">
        {a ? (
          <div className="px-4 py-3" data-testid="account-meter">
            <div className="mb-1.5 flex justify-between text-[13px]"><span className="text-ink">{a.quotaBytes ? `${fmtBytes(a.usedBytes)} used of ${fmtBytes(a.quotaBytes)}` : `${fmtBytes(a.usedBytes)} used`}</span></div>
            {a.quotaBytes ? <Meter label="Account storage" fraction={a.usedBytes / a.quotaBytes} amber /> : null}
          </div>
        ) : account.state === 'failed' ? (
          <p role="alert" className="px-4 py-3 text-[12px] text-red-text">{actionFailure('read your account’s storage', 'failed').text}</p>
        ) : <div className="px-4 py-3 text-[13px] text-ink-3" aria-busy="true">Reading your account’s storage…</div>}
        {a ? (
          <>
            <Row label="Files" value={fmtBytes(a.filesBytes)} mono />
            <Row label="Photos and videos" value={fmtBytes(a.photosBytes)} mono />
            <Row label="Backups" value={fmtBytes(a.backupsBytes)} mono onClick={() => navigateTo({ section: 'backups' })} />
          </>
        ) : null}
        <Row label="Trash" onClick={() => navigateTo({ section: 'trash' })} />
        <Row label="Storage insights" sub="By type, and your largest files" onClick={() => open('insights')} />
      </Group>
      <Group title="On this PC" note="Limits are per PC. Thumbnails and previews count toward your storage.">
        <Row label="Downloaded files" value={here ? ofLimit(here.cache_bytes, config.local_cache_limit_bytes) : undefined} mono
          right={<><Btn onClick={() => open('free-up')}>Clear…</Btn><Btn onClick={() => open('limit-downloads')}>Change limit</Btn></>} />
        <Row label="Thumbnails and previews" value={previews ? `${fmtBytes(previews.bytes)} of ${previews.limitGb} GB` : undefined} mono
          sub={previews ? undefined : 'Not measured in this build yet.'}
          right={<><Btn onClick={() => setRefused(actionFailure('clear the previews', 'unsupported').text)}>Clear</Btn><Btn onClick={() => open('limit-previews')}>Change limit</Btn></>} />
        {/* No command empties the preview cache yet: Clear says so instead of opening the limit (M3e). */}
        {refused ? <p role="alert" className="px-4 pb-2 text-[12px] text-red-text">{refused}</p> : null}
        <Row label="Available offline" value={here ? fmtBytes(here.pinned_bytes) : undefined} mono href="?section=available-offline" />
        {local.state === 'failed' ? <p role="alert" className="px-4 pb-2 text-[12px] text-red-text">{actionFailure('read this PC’s storage', 'failed').text}</p> : null}
      </Group>
    </Page>
  )
}

function LimitDialog({ title, id, initial, max, inUse, note, onSave }: { title: string; id: string; initial: number; max: number; inUse: string; note: string; onSave: (gb: number) => void }) {
  const { close } = useSettings()
  const [gb, setGb] = useState(initial)
  return (
    <Dialog labelId={id} title={title} onCancel={close}
      actions={<><Btn onClick={close}>Cancel</Btn><Btn variant="amber" onClick={() => { onSave(gb); close() }}>Save</Btn></>}>
      <div className={`mb-2 text-[22px] text-ink ${mono}`}>{gb} GB</div>
      <Slider label={title} min={1} max={max} value={gb} onChange={setGb} />
      <div className={`mt-1 flex justify-between text-[12px] text-ink-3 ${mono}`}><span>1 GB</span><span>{max} GB</span></div>
      <p className="mt-3">{inUse}</p>
      <p className="mt-1 text-[12px] text-ink-3">{note}</p>
    </Dialog>
  )
}

/**
 * DW-0713. The board's limit is drawn in a frame; this build has no preview limit (no config field, no command), so the
 * running app says so instead of a slider whose Save would change nothing.
 */
export function PreviewLimitDialog() {
  const { close } = useSettings()
  const previews = board<BoardPreviews>('previews')
  if (previews) {
    return (
      <LimitDialog id="pv-h" title="Limit for thumbnails and previews" initial={previews.limitGb} max={93} onSave={() => undefined}
        inUse={`${fmtBytes(previews.bytes)} in use. ${previews.limitGb} GB fits about ${previews.fits} previews.`} note="Runs from 1 GB to half of the free space on this PC." />
    )
  }
  return (
    <Dialog labelId="pv-h" title="Limit for thumbnails and previews" onCancel={close} actions={<Btn variant="amber" onClick={close}>Close</Btn>}>
      <p>This build has no limit for thumbnails and previews yet, so there is nothing to change.</p>
    </Dialog>
  )
}

/** DW-0714: the downloaded-files limit (`local_cache_limit_bytes`; 0 is no limit) with what this PC holds now. */
export function DownloadLimitDialog() {
  const { config, update } = useConfig()
  const local = useLocalSpace()
  const limit = config.local_cache_limit_bytes ?? 0
  const initial = limit > 0 ? Math.max(1, Math.round(limit / GB)) : 50
  const max = Math.max(186, initial)
  const held = local.state === 'ready' ? `${fmtBytes(local.value.cache_bytes)} in use. ` : ''
  return (
    <LimitDialog id="dl-h" title="Limit for downloaded files" initial={initial} max={max} onSave={(gb) => update({ local_cache_limit_bytes: gb * GB })}
      inUse={`${held}${limit > 0 ? '' : 'No limit is set now. '}Lowering the limit below what is in use removes the oldest copies straight away.`}
      note="A file you have open is never removed. Available offline files don't count." />
  )
}

export function FreeUpDialog() {
  const { close } = useSettings()
  const local = useLocalSpace()
  const [state, setState] = useState<'idle' | 'working' | string>('idle')
  // Free up space removes the downloaded copies (`free_up_space`, DW-0780#0); a refusal stays in the alert.
  const free = () => {
    setState('working')
    void command('free_up_space').then((r) => { if (r.ok) close(); else setState(actionFailure('free up space', r.unsupported ? 'unsupported' : 'failed').text) })
  }
  const failed = state !== 'idle' && state !== 'working' ? state : null
  const bytes = local.state === 'ready' ? local.value.cache_bytes : 0
  return (
    <Dialog labelId="free-h" title={bytes > 0 ? `Free up ${fmtBytes(bytes)}?` : 'Free up space?'} onCancel={close}
      actions={<><Btn onClick={close}>Cancel</Btn><Btn variant="amber" onClick={free} disabled={state === 'working'}>Free up space</Btn></>}>
      {failed ? <p role="alert" className="mb-2 text-[12px] text-red-text">{failed}</p> : null}
      <p>This removes the copies of your files stored on this PC. File Explorer shows them as online only, and they download again when you open them.</p>
      <p className="mt-2">Never removed: Available offline files and any file that is open. Nothing leaves your Beebeeb account.</p>
    </Dialog>
  )
}
