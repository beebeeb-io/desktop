// Drive › Thumbnails and previews (DW-0744), its confirm (DW-0744b) and the compare view (DW-0745).
// The choice is read and saved through get_desktop_config / set_desktop_config. Usage comes from the same two
// summaries as Storage. A figure neither summary carries says "Not reported yet"; nothing here is a made-up size.
import { useState } from 'react'
import type { DesktopConfig } from '../../../../desktopApi'
import { actionFailure } from '../../files/model'
import { useSettings } from '../shell/context'
import { useAccountStorage, useConfig, useLocalSpace } from '../shell/data'
import { Btn, Dialog, Group, Meter, Page, Row, Slider, Switch, fmtBytes, mono } from '../shell/kit'

type Size = '720p' | '1080p' | '2K' | '4K'
type Fmt = 'HEIC' | 'JPEG'
const SIZES: readonly Size[] = ['720p', '1080p', '2K', '4K']
const FMTS: readonly Fmt[] = ['HEIC', 'JPEG']

// Not on the TypeScript DesktopConfig: that file is over its line baseline. A patch still sends the key, and Rust
// merges only that key. A missing key on an older file reads as the default below.
const extra = {
  photo_previews: true,
  preview_size: '4K',
  preview_format: 'HEIC',
} as const

type PreviewConfig = DesktopConfig & Partial<{ photo_previews: boolean; preview_size: string; preview_format: string }>

function oneOf<T extends string>(value: unknown, options: readonly T[]): T | null {
  return typeof value === 'string' && (options as readonly string[]).includes(value) ? value as T : null
}

function readNum(source: object | null, keys: readonly string[]): number | null {
  if (!source) return null
  const raw = source as Record<string, unknown>
  for (const key of keys) {
    const value = raw[key]
    if (typeof value === 'number' && Number.isFinite(value)) return value
  }
  return null
}

interface Usage {
  previews: number | null
  fallback: number | null
  grid: number | null
  onPc: number | null
  limit: number | null
  original: number | null
  sample: number | null
  photos: number | null
}

/** Account summary first for the cloud split; the local summary for what this PC holds. */
function readUsage(account: object | null, local: object | null): Usage {
  return {
    previews: readNum(account, ['preview_bytes', 'previewBytes']),
    fallback: readNum(account, ['fallback_preview_bytes', 'fallbackPreviewBytes']) ?? readNum(local, ['fallback_preview_bytes', 'fallbackPreviewBytes']),
    grid: readNum(account, ['thumbnail_bytes', 'thumbnailBytes']) ?? readNum(local, ['thumbnail_bytes', 'thumbnailBytes']),
    onPc: readNum(local, ['local_preview_bytes', 'localPreviewBytes', 'preview_bytes', 'previewBytes']),
    limit: readNum(local, ['preview_limit_bytes', 'previewLimitBytes']) ?? readNum(account, ['preview_limit_bytes', 'previewLimitBytes']),
    original: readNum(account, ['original_bytes', 'originalBytes']) ?? readNum(local, ['original_bytes', 'originalBytes']),
    sample: readNum(account, ['sample_preview_bytes', 'samplePreviewBytes']) ?? readNum(local, ['sample_preview_bytes', 'samplePreviewBytes']),
    photos: readNum(account, ['photo_count', 'photoCount']) ?? readNum(local, ['photo_count', 'photoCount']),
  }
}

function reported(n: number | null): string {
  return n === null ? 'Not reported yet' : fmtBytes(n)
}

function onPcValue(bytes: number | null, limit: number | null): string {
  if (bytes === null) return 'Not reported yet'
  if (limit === null) return fmtBytes(bytes)
  if (limit <= 0) return `${fmtBytes(bytes)} · no limit`
  return `${fmtBytes(bytes)} of ${fmtBytes(limit)}`
}

function sampleLine(size: Size | null, fmt: Fmt | null, measured: number | null, original: number | null): string {
  if (!size || !fmt) return 'Not reported yet'
  if (measured === null && original === null) return `${size} ${fmt} · Not reported yet`
  const preview = measured === null ? 'Not reported yet' : fmtBytes(measured)
  const orig = original === null ? 'Not reported yet' : fmtBytes(original)
  return `${size} ${fmt} · ${preview}, original ${orig}`
}

/** Saved size and format against the original only when both are known. */
function comparison(size: Size | null, fmt: Fmt | null, original: number | null): string {
  const missing: string[] = []
  if (!size) missing.push('Preview size is missing')
  if (!fmt) missing.push('Preview format is missing')
  if (original === null) missing.push('Original size is missing')
  if (!size || !fmt || original === null) return missing.join('. ')
  return `${size} ${fmt} against original ${fmtBytes(original)}`
}

function previewLabel(size: Size | null, fmt: Fmt | null): string {
  if (size && fmt) return `${size} ${fmt}`
  const missing: string[] = []
  if (!size) missing.push('Preview size is missing')
  if (!fmt) missing.push('Preview format is missing')
  return missing.join('. ')
}

function usePreview() {
  const { config, update } = useConfig()
  const account = useAccountStorage()
  const local = useLocalSpace()
  const raw = config as PreviewConfig
  const savedSize = oneOf(raw.preview_size, SIZES)
  const savedFmt = oneOf(raw.preview_format, FMTS)
  const a = account.state === 'ready' ? account.value : null
  const l = local.state === 'ready' ? local.value : null
  return {
    update,
    on: typeof raw.photo_previews === 'boolean' ? raw.photo_previews : extra.photo_previews,
    savedSize,
    savedFmt,
    shownSize: savedSize ?? extra.preview_size,
    shownFmt: savedFmt ?? extra.preview_format,
    usage: readUsage(a, l),
  }
}

function Seg<T extends string>({ value, options, onPick, label }: { value: T; options: readonly T[]; onPick: (v: T) => void; label: string }) {
  return (
    <div role="radiogroup" aria-label={label} className="flex rounded-md border border-line bg-paper-2 p-0.5">
      {options.map((o) => (
        <button key={o} type="button" role="radio" aria-checked={o === value} onClick={() => onPick(o)}
          className={`h-6 rounded px-2.5 text-[12px] ${o === value ? 'bg-paper text-ink shadow-sm' : 'text-ink-2'}`}>{o}</button>
      ))}
    </div>
  )
}

export function Thumbnails() {
  const { go, open, dialog } = useSettings()
  const { update, on, savedSize, savedFmt, shownSize, shownFmt, usage } = usePreview()
  const [rebuild, setRebuild] = useState<string | null>(null)
  const choose = (s: Size, f: Fmt) => open(`apply:${s}:${f}`)
  const headline = usage.onPc ?? usage.previews
  const onPc = usage.onPc
  const limit = usage.limit
  return (
    <Page title="Thumbnails and previews">
      <Group>
        <Row label="Photo previews" sub="Stored previews of your photos, encrypted like everything else."
          right={<Switch label="Photo previews" on={on} onChange={(next) => { void update({ photo_previews: next } as Partial<DesktopConfig>) }} />} />
        <Row label="Sample" value={sampleLine(savedSize, savedFmt, usage.sample, usage.original)} mono right={<Btn onClick={() => go('compare')}>Compare</Btn>} />
        <Row label="Preview size" right={<Seg label="Preview size" value={shownSize} options={SIZES} onPick={(s) => choose(s, shownFmt)} />} />
        <Row label="Format" right={<Seg label="Format" value={shownFmt} options={FMTS} onPick={(f) => choose(shownSize, f)} />} />
      </Group>
      <Group title={headline === null ? 'What it uses' : `What it uses · ${fmtBytes(headline)}`} note="New previews replace the old ones only after they've uploaded.">
        {onPc !== null && limit !== null && limit > 0 ? (
          <div className="px-4 py-3">
            <Meter label="Preview storage" fraction={onPc / limit} />
          </div>
        ) : null}
        <Row label="Previews" value={reported(usage.previews)} mono />
        <Row label="Fallback previews" value={reported(usage.fallback)} mono />
        <Row label="Grid thumbnails" value={reported(usage.grid)} mono />
        <Row label="On this PC" value={onPcValue(usage.onPc, usage.limit)} mono onClick={() => go('storage')} />
        <Row label="Made automatically" right={<Btn onClick={() => setRebuild('Rebuilding previews in the background.')}>Rebuild previews</Btn>} sub={rebuild ?? undefined} />
      </Group>
      {dialog?.startsWith('apply:') ? (
        <ApplyDialog arg={dialog} bytes={usage.previews} photos={usage.photos} onApply={(s, f) => {
          const patch: Record<string, string> = {}
          if (s !== savedSize) patch.preview_size = s
          if (f !== savedFmt) patch.preview_format = f
          if (Object.keys(patch).length) void update(patch as Partial<DesktopConfig>)
          setRebuild('Rebuilding previews in the background.')
        }} />
      ) : null}
    </Page>
  )
}

function ApplyDialog({ arg, bytes, photos, onApply }: { arg: string; bytes: number | null; photos: number | null; onApply: (s: Size, f: Fmt) => void }) {
  const { close } = useSettings()
  const parts = arg.split(':')
  const s = oneOf(parts[1], SIZES)
  const f = oneOf(parts[2], FMTS)
  if (!s || !f) return null
  const who = photos !== null ? ` to ${photos.toLocaleString('en')} photos` : ''
  return (
    <Dialog labelId="apply-h" title={`Apply ${s} ${f}${who}?`} onCancel={close}
      actions={<><Btn onClick={close}>Cancel</Btn><Btn variant="amber" onClick={() => { onApply(s, f); close() }}>{photos !== null ? `Apply to ${photos.toLocaleString('en')} photos` : 'Apply'}</Btn></>}>
      <p>New previews: <span className={mono}>Not reported yet</span>. Current previews: <span className={mono}>{reported(bytes)}</span>.</p>
      <p className="mt-2">Nothing changes until Apply. The old previews stay until the new ones have uploaded.</p>
    </Dialog>
  )
}

type Mode = 'Stacked' | 'Wipe' | 'Hold to flip'
export function Compare() {
  const { go } = useSettings()
  const { savedSize, savedFmt, usage } = usePreview()
  const [mode, setMode] = useState<Mode>('Wipe')
  const [wipe, setWipe] = useState(50)
  const [refused, setRefused] = useState<string | null>(null)
  const versus = comparison(savedSize, savedFmt, usage.original)
  return (
    <Page title="Compare previews" action={<Btn variant="amber" onClick={() => go('thumbnails')}>Done</Btn>}>
      <Seg label="View" value={mode} options={['Stacked', 'Wipe', 'Hold to flip'] as Mode[]} onPick={setMode} />
      <div className="relative aspect-[3/2] overflow-hidden rounded-[12px] border border-line bg-paper-3" data-testid="compare-photo">
        <div className="absolute inset-0 bg-gradient-to-br from-paper-2 to-paper-3" />
        <div className="absolute inset-y-0 w-px bg-ink" style={{ left: `${wipe}%` }} aria-hidden />
        <span className={`absolute left-3 top-3 rounded bg-paper px-1.5 text-[11px] ${mono}`}>{usage.original === null ? 'Original size is missing' : `Original · ${fmtBytes(usage.original)}`}</span>
        <span className={`absolute right-3 top-3 rounded bg-paper px-1.5 text-[11px] ${mono}`}>{previewLabel(savedSize, savedFmt)}</span>
      </div>
      <Slider label="Wipe position" min={0} max={100} value={wipe} onChange={setWipe} />
      <Group>
        <Row label="Saved" value={savedSize && savedFmt ? `${savedSize} ${savedFmt}` : 'Not reported yet'} mono />
        <Row label="Original" value={reported(usage.original)} mono />
        <Row label="Comparison" value={versus} mono />
        <Row label="Choose photo" onClick={() => setRefused(actionFailure('choose another photo', 'unsupported').text)} />
      </Group>
      {refused ? <p role="alert" className="text-[12px] text-red-text">{refused}</p> : null}
      <p className="text-[12px] text-ink-3">Sizes are measured on the chosen photo.</p>
    </Page>
  )
}
