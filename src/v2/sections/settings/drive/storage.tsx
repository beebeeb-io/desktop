// Drive › Storage (DW-0712) and its dialogs: preview limit (DW-0713), downloaded-files limit (DW-0714), free up (DW-0780).
// Used and quota come from DriveSource storage() (the same summary as the account header; snake_case is account_usage).
// A split is shown only when that answer includes it. This PC's copies come from desktop_storage_summary, the download
// limit from local_cache_limit_bytes via set_desktop_config. free_up_space is the only command this file calls.
// A figure no command reported says "Not reported yet". The storage meter is not Amber.
import { useState } from 'react'
import { command } from '../../../../desktopApi'
import { navigateTo } from '../../../routes'
import { actionFailure } from '../../files/model'
import { Btn, Dialog, Group, Meter, Page, Row, fmtBytes, GB, mono } from '../shell/kit'
import { useAccountStorage, useConfig, useLocalSpace } from '../shell/data'
import { useSettings } from '../shell/context'

/** "18.4 GB of 50 GB", or "of no limit" when the limit is 0 (Rust: 0 is Unlimited). */
export function ofLimit(bytes: number, limit: number | undefined): string {
  return limit && limit > 0 ? `${fmtBytes(bytes)} of ${fmtBytes(limit)}` : `${fmtBytes(bytes)} · no limit`
}

function num(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

/** storage() is camelCase; account_usage is snake_case. Either shape, no second command. */
function usageOf(value: object) {
  const raw = value as unknown as Record<string, unknown>
  return {
    used: num(raw.usedBytes) ?? num(raw.used_bytes),
    quota: num(raw.quotaBytes) ?? num(raw.quota_bytes),
    files: num(raw.filesBytes) ?? num(raw.files_bytes),
    photos: num(raw.photosBytes) ?? num(raw.photos_bytes),
    backups: num(raw.backupsBytes) ?? num(raw.backups_bytes),
    other: num(raw.otherBytes) ?? num(raw.other_bytes),
  }
}

function splitValue(n: number | null): string {
  return n === null ? 'Not reported yet' : fmtBytes(n)
}

/** A count only when free_up_space's own answer includes one. Today's command returns bytes_freed. */
function freedOf(value: unknown): { files: number | null; bytes: number | null } {
  const raw = value && typeof value === 'object' ? value as Record<string, unknown> : {}
  let files: number | null = null
  for (const key of ['files_removed', 'files_freed', 'file_count', 'count']) {
    const n = num(raw[key])
    if (n !== null) { files = n; break }
  }
  return { files, bytes: num(raw.bytes_freed) }
}

export function Storage() {
  const { open } = useSettings()
  const [refused, setRefused] = useState<string | null>(null)
  const { config } = useConfig()
  const account = useAccountStorage()
  const local = useLocalSpace()
  const a = account.state === 'ready' ? usageOf(account.value) : null
  const here = local.state === 'ready' ? local.value : null
  const third = a && a.backups === null && a.other !== null ? { label: 'Other', value: a.other } : { label: 'Backups', value: a?.backups ?? null }
  return (
    <Page title="Storage">
      <Group title="In your Beebeeb account">
        {account.state === 'loading' ? <div className="px-4 py-3 text-[13px] text-ink-3" aria-busy="true">Reading your account’s storage…</div>
          : account.state === 'failed' ? <p role="alert" className="px-4 py-3 text-[12px] text-red-text">{actionFailure('read your account’s storage', 'failed').text}</p>
            : a && a.used !== null ? (
              <div className="px-4 py-3" data-testid="account-meter">
                <div className="mb-1.5 flex justify-between text-[13px]"><span className="text-ink">{a.quota ? `${fmtBytes(a.used)} used of ${fmtBytes(a.quota)}` : `${fmtBytes(a.used)} used`}</span></div>
                {a.quota ? <Meter label="Account storage" fraction={a.used / a.quota} /> : null}
              </div>
            ) : <p className="px-4 py-3 text-[13px] text-ink-3">Not reported yet</p>}
        {a && a.used !== null ? (
          <>
            <Row label="Files" value={splitValue(a.files)} mono />
            <Row label="Photos and videos" value={splitValue(a.photos)} mono />
            <Row label={third.label} value={splitValue(third.value)} mono={third.value !== null ? true : undefined} onClick={third.label === 'Backups' && third.value !== null ? () => navigateTo({ section: 'backups' }) : undefined} />
          </>
        ) : null}
        <Row label="Trash" onClick={() => navigateTo({ section: 'trash' })} />
        <Row label="Your plan" onClick={() => open('plan')} />
        <Row label="Storage insights" sub="By type, and your largest files" onClick={() => open('insights')} />
      </Group>
      <Group title="On this PC" note="Limits are per PC. Thumbnails and previews count toward your storage.">
        <Row label="Downloaded files" value={here ? ofLimit(here.cache_bytes, config.local_cache_limit_bytes) : undefined} mono
          right={<><Btn onClick={() => open('free-up')}>Clear…</Btn><Btn onClick={() => open('limit-downloads')}>Change limit</Btn></>} />
        <Row label="Thumbnails and previews" sub="Not reported yet"
          right={<><Btn onClick={() => setRefused(actionFailure('clear the previews', 'unsupported').text)}>Clear</Btn><Btn onClick={() => open('limit-previews')}>Change limit</Btn></>} />
        {refused ? <p role="alert" className="px-4 pb-2 text-[12px] text-red-text">{refused}</p> : null}
        <Row label="Available offline" value={here ? fmtBytes(here.pinned_bytes) : undefined} mono href="?section=available-offline" />
        {local.state === 'failed' ? <p role="alert" className="px-4 pb-2 text-[12px] text-red-text">{actionFailure('read this PC’s storage', 'failed').text}</p> : null}
      </Group>
    </Page>
  )
}

/** DW-0713. No config field and no command report a preview limit, so there is nothing to save. */
export function PreviewLimitDialog() {
  const { close } = useSettings()
  return (
    <Dialog labelId="pv-h" title="Limit for thumbnails and previews" onCancel={close} actions={<Btn variant="amber" onClick={close}>Close</Btn>}>
      <p>Not reported yet. This build has no limit for thumbnails and previews, so there is nothing to change.</p>
    </Dialog>
  )
}

/** DW-0714. The number is the saved local_cache_limit_bytes (0 is no limit), written back through set_desktop_config. */
export function DownloadLimitDialog() {
  const { close } = useSettings()
  const { config, update, loaded } = useConfig()
  const local = useLocalSpace()
  if (!loaded) {
    return (
      <Dialog labelId="dl-h" title="Limit for downloaded files" onCancel={close} actions={<Btn onClick={close}>Cancel</Btn>}>
        <p aria-busy="true">Reading this PC’s limit…</p>
      </Dialog>
    )
  }
  const bytes = config.local_cache_limit_bytes ?? 0
  const held = local.state === 'ready' ? `${fmtBytes(local.value.cache_bytes)} in use. ` : ''
  return <DownloadLimitForm key={bytes} bytes={bytes} held={held} onSave={(next) => update({ local_cache_limit_bytes: next })} />
}

function DownloadLimitForm({ bytes, held, onSave }: { bytes: number; held: string; onSave: (next: number) => void }) {
  const { close } = useSettings()
  const initial = bytes > 0 ? Math.max(1, Math.round(bytes / GB)) : 1
  const [gb, setGb] = useState(initial)
  const [touched, setTouched] = useState(false)
  const shown = bytes > 0 || touched ? `${gb} GB` : 'No limit'
  const save = () => { onSave(bytes > 0 || touched ? gb * GB : 0); close() }
  return (
    <Dialog labelId="dl-h" title="Limit for downloaded files" onCancel={close}
      actions={<><Btn onClick={close}>Cancel</Btn><Btn variant="amber" onClick={save}>Save</Btn></>}>
      <div className={`mb-2 text-[22px] text-ink ${mono}`}>{shown}</div>
      <input type="range" aria-label="Limit for downloaded files" min={1} max={Math.max(initial, 186)} value={gb}
        className="h-1.5 w-full cursor-pointer accent-[color:var(--color-ink-3)]"
        onInput={(e) => { setTouched(true); setGb(Number(e.currentTarget.value)) }} />
      <div className={`mt-1 text-[12px] text-ink-3 ${mono}`}><span>1 GB</span></div>
      <p className="mt-3">{held}{bytes > 0 ? '' : 'No limit is set now. '}Lowering the limit below what is in use removes the oldest copies straight away.</p>
      <p className="mt-1 text-[12px] text-ink-3">A file you have open is never removed. Available offline files don't count.</p>
    </Dialog>
  )
}

export function FreeUpDialog() {
  const { close } = useSettings()
  const local = useLocalSpace()
  const [phase, setPhase] = useState<'idle' | 'working'>('idle')
  const [failure, setFailure] = useState<string | null>(null)
  const [result, setResult] = useState<{ files: number | null; bytes: number | null } | null>(null)
  // Free up space removes the downloaded copies (free_up_space, DW-0780#0). The dialog stays open so the answer can be read.
  const free = () => {
    setPhase('working')
    setFailure(null)
    void command('free_up_space').then((r) => {
      setPhase('idle')
      if (!r.ok) { setResult(null); setFailure(actionFailure('free up space', r.unsupported ? 'unsupported' : 'failed').text); return }
      setResult(freedOf(r.value))
    })
  }
  const bytes = local.state === 'ready' ? local.value.cache_bytes : 0
  return (
    <Dialog labelId="free-h" title={bytes > 0 ? `Free up ${fmtBytes(bytes)}?` : 'Free up space?'} onCancel={close}
      actions={<><Btn onClick={close}>Cancel</Btn><Btn variant="amber" onClick={free} disabled={phase === 'working'}>Free up space</Btn></>}>
      {failure ? <p role="alert" className="mb-2 text-[12px] text-red-text">{failure}</p> : null}
      {result ? <p data-testid="free-up-result" className="mb-2">Files: {result.files === null ? 'Not reported yet' : result.files} · Bytes: {result.bytes === null ? 'Not reported yet' : fmtBytes(result.bytes)}</p> : null}
      <p>This removes the copies of your files stored on this PC. File Explorer shows them as online only, and they download again when you open them.</p>
      <p className="mt-2">Never removed: Available offline files and any file that is open. Nothing leaves your Beebeeb account.</p>
    </Dialog>
  )
}
