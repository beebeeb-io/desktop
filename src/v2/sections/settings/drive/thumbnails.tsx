// Drive › Thumbnails and previews (DW-0744), its confirm (DW-0744b) and the compare view (DW-0745).
import { useState } from 'react'
import { Btn, Dialog, Group, Page, Row, Slider, Switch, mono } from '../shell/kit'
import { useSettings } from '../shell/context'
import { actionFailure } from '../../files/model'

type Size = '720p' | '1080p' | '2K' | '4K'
type Fmt = 'HEIC' | 'JPEG'
const SIZES: Size[] = ['720p', '1080p', '2K', '4K']
const SIZE_KB: Record<Size, string> = { '720p': '213 KB', '1080p': '420 KB', '2K': '640 KB', '4K': '1.0 MB' }

function Seg<T extends string>({ value, options, onPick, label }: { value: T; options: T[]; onPick: (v: T) => void; label: string }) {
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
  const [on, setOn] = useState(true)
  const [size, setSize] = useState<Size>('4K')
  const [fmt, setFmt] = useState<Fmt>('HEIC')
  const [rebuild, setRebuild] = useState<string | null>(null)
  // A change shows its cost in a confirm (DW-0744b); "Cancel" keeps the old choice.
  const choose = (s: Size, f: Fmt) => open(`apply:${s}:${f}`)
  return (
    <Page title="Thumbnails and previews">
      <Group>
        <Row label="Photo previews" sub="Stored previews of your photos, encrypted like everything else." right={<Switch label="Photo previews" on={on} onChange={setOn} />} />
        <Row label="Sample" value={`${size} ${fmt} · ${SIZE_KB[size]}, original 30.9 MB`} mono right={<Btn onClick={() => go('compare')}>Compare</Btn>} />
        <Row label="Preview size" right={<Seg label="Preview size" value={size} options={SIZES} onPick={(s) => choose(s, fmt)} />} />
        <Row label="Format" right={<Seg label="Format" value={fmt} options={['HEIC', 'JPEG'] as Fmt[]} onPick={(f) => choose(size, f)} />} />
      </Group>
      <Group title="What it uses ≈ 3.1 GB" note="New previews replace the old ones only after they've uploaded.">
        <Row label="Previews" value="2.4 GB" mono />
        <Row label="Fallback previews" value="420 MB" mono />
        <Row label="Grid thumbnails" value="300 MB" mono />
        <Row label="On this PC" value="3.1 GB of 20 GB limit" mono onClick={() => go('storage')} />
        <Row label="Made automatically" right={<Btn onClick={() => setRebuild('Rebuilding previews in the background.')}>Rebuild previews</Btn>} sub={rebuild ?? undefined} />
      </Group>
      {dialog?.startsWith('apply:') ? <ApplyDialog arg={dialog} onApply={(s, f) => { setSize(s); setFmt(f); setRebuild('Rebuilding 2,431 photos in the background.') }} /> : null}
    </Page>
  )
}

const COST: Record<string, string> = { '2K:HEIC': '0.6 GB', '720p:HEIC': '0.5 GB' }
function ApplyDialog({ arg, onApply }: { arg: string; onApply: (s: Size, f: Fmt) => void }) {
  const { close } = useSettings()
  const [, s, f] = arg.split(':') as [string, Size, Fmt]
  return (
    <Dialog labelId="apply-h" title={`Apply ${s} ${f} to 2,431 photos?`} onCancel={close}
      actions={<><Btn onClick={close}>Cancel</Btn><Btn variant="amber" onClick={() => { onApply(s, f); close() }}>Apply to 2,431 photos</Btn></>}>
      <p>New previews: <span className={mono}>{COST[`${s}:${f}`] ?? '1.4 GB'}</span>. Current previews: <span className={mono}>2.4 GB</span>.</p>
      <p className="mt-2">Nothing changes until Apply. The old previews stay until the new ones have uploaded.</p>
    </Dialog>
  )
}
type Mode = 'Stacked' | 'Wipe' | 'Hold to flip'
export function Compare() {
  const { go } = useSettings()
  const [mode, setMode] = useState<Mode>('Wipe')
  const [wipe, setWipe] = useState(50)
  const [refused, setRefused] = useState<string | null>(null)
  return (
    <Page title="Compare previews" action={<Btn variant="amber" onClick={() => go('thumbnails')}>Done</Btn>}>
      <Seg label="View" value={mode} options={['Stacked', 'Wipe', 'Hold to flip'] as Mode[]} onPick={setMode} />
      <div className="relative aspect-[3/2] overflow-hidden rounded-[12px] border border-line bg-paper-3" data-testid="compare-photo">
        <div className="absolute inset-0 bg-gradient-to-br from-paper-2 to-paper-3" />
        <div className="absolute inset-y-0 w-px bg-ink" style={{ left: `${wipe}%` }} aria-hidden />
        <span className={`absolute left-3 top-3 rounded bg-paper px-1.5 text-[11px] ${mono}`}>Original · 30.9 MB</span>
        <span className={`absolute right-3 top-3 rounded bg-paper px-1.5 text-[11px] ${mono}`}>4K HEIC · 1.0 MB</span>
      </div>
      {/* The kit's neutral slider (punch list, DW-0745): the browser's default range input is blue. */}
      <Slider label="Wipe position" min={0} max={100} value={wipe} onChange={setWipe} />
      <Group>
        <Row label="Sizes" value={SIZES.map((s) => `${s} ${SIZE_KB[s]}`).join(' · ')} mono />
        <Row label="Format" value="HEIC 1.0 MB · JPEG 2.1 MB" mono />
        {/* Blocked (M3e): nothing measures another of your photos at each size yet; Choose photo says so. */}
        <Row label="Choose photo" onClick={() => setRefused(actionFailure('choose another photo', 'unsupported').text)} />
      </Group>
      {refused ? <p role="alert" className="text-[12px] text-red-text">{refused}</p> : null}
      <p className="text-[12px] text-ink-3">Sizes are measured on the chosen photo.</p>
    </Page>
  )
}
