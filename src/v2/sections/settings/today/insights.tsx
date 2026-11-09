// Storage › Storage insights (DW-0783 family): where the storage goes, by type, and the files that take the most.
// Every figure is a row `account_storage_breakdown` returned. The total is the sum of those rows.
import { useCallback, useEffect, useState } from 'react'
import { useDriveSource } from '@beebeeb/shared/drive-ui'
import { command, type StorageBreakdown, type StorageCategory } from '../../../../desktopApi'
import { navigateTo } from '../../../routes'
import { openFailureText } from '../../shared/api'
import { actionFailure } from '../../files/model'
import { Btn, Group, Meter, Page, fmtBytes, mono } from '../shell/kit'
import { BackLink, ErrorBanner, Skeleton } from './ui'

/** The three kinds the breakdown answers in (`account_storage_breakdown`'s category). */
export type Kind = 'Media' | 'Documents' | 'Other'

/** A path as the state DB stores it may start with a separator ("/docs/a.pdf" and "docs/a.pdf" are the same file,
 *  #274 review): the page shows it and routes it without one. */
const drivePath = (path: string) => path.replace(/^[\\/]+/, '').replace(/\\/g, '/')
/** Cut a path in the middle so the file name stays readable. */
export function cutPath(stored: string, max = 44): string {
  const path = drivePath(stored)
  if (path.length <= max) return path
  const parts = path.split('/')
  const name = parts[parts.length - 1]
  const head = parts[0]
  const room = Math.max(4, max - name.length - head.length - 5)
  return name.length + head.length + 5 > max ? `…/${name}` : `${head}/…/${parts.slice(-2, -1)[0].slice(-room)}/${name}`
}

export type InsightsState = 'ready' | 'loading' | 'error' | 'empty'

const KIND_OF: Record<StorageCategory, Kind> = { media: 'Media', documents: 'Documents', other: 'Other' }
const EXT_KIND: [RegExp, string][] = [
  [/\.(mov|mp4|m4v|mkv|avi|webm)$/i, 'Video'], [/\.(zip|tar|gz|7z|rar)$/i, 'Archive'], [/\.pdf$/i, 'PDF'],
  [/\.(wav|m4a|mp3|flac|aac)$/i, 'Audio'], [/\.(nef|cr2|arw|dng|heic|jpe?g|png|tiff?)$/i, 'Photo'],
]
/** A largest file's kind, from its name; else its category. */
export function kindOf(path: string, category: StorageCategory): string {
  return EXT_KIND.find(([re]) => re.test(path))?.[1] ?? KIND_OF[category]
}
/** The section a largest file is in: the Backups folder at the top of Beebeeb is its own section (DW-0422), the
 *  rest is Files. */
export function sectionOfPath(path: string): 'backups' | 'files' {
  return /^Backups(\/|$)/.test(drivePath(path)) ? 'backups' : 'files'
}

const SECTION_LABEL = { backups: 'Backups', files: 'Files' } as const
const FAILED_READ = actionFailure('read the storage breakdown', 'failed').text

export interface InsightSlice { label: string; bytes: number; files: number }
export interface InsightFile { id: string; path: string; shown: string; kind: string; section: 'backups' | 'files'; bytes: number }
export interface InsightSection { id: 'backups' | 'files'; label: string; bytes: number; files: number }

export type InsightsModel =
  | { kind: 'empty'; quotaBytes: number | null }
  | { kind: 'error'; text: string }
  | { kind: 'ready'; totalBytes: number; quotaBytes: number | null; slices: InsightSlice[]; largest: InsightFile[]; sections: InsightSection[] }

const countLabel = (n: number) => `${n.toLocaleString('en')} ${n === 1 ? 'file' : 'files'}`

/**
 * Chart slices are the category rows the command returned, in that order, including a count under 10.
 * A category it left out (no Other, unless it sent one) is not added. The total is the sum of those rows,
 * not a second number beside them. Largest files keep the kind, section and cut path of their own row.
 */
export function insightsFrom(
  read: { ok: true; value: StorageBreakdown } | { ok: false; unsupported: boolean },
  quotaBytes: number | null,
): InsightsModel {
  if (!read.ok) return { kind: 'error', text: actionFailure('read the storage breakdown', read.unsupported ? 'unsupported' : 'failed').text }
  const slices: InsightSlice[] = []
  for (const c of read.value.by_category) {
    const label = KIND_OF[c.category]
    if (!label) continue
    slices.push({ label, bytes: c.bytes, files: c.file_count })
  }
  const largest: InsightFile[] = read.value.largest_files.map((f) => ({
    id: f.file_id,
    path: f.path,
    shown: cutPath(f.path),
    kind: kindOf(f.path, f.category),
    section: sectionOfPath(f.path),
    bytes: f.size_bytes,
  }))
  const nothing = read.value.total_files === 0 && largest.length === 0 && slices.every((s) => s.bytes === 0 && s.files === 0)
  if (nothing) return { kind: 'empty', quotaBytes }
  const order: ('backups' | 'files')[] = []
  const acc = new Map<'backups' | 'files', { bytes: number; files: number }>()
  for (const f of largest) {
    const cur = acc.get(f.section) ?? { bytes: 0, files: 0 }
    if (!acc.has(f.section)) order.push(f.section)
    cur.bytes += f.bytes
    cur.files += 1
    acc.set(f.section, cur)
  }
  const sections = order.map((id) => ({ id, label: SECTION_LABEL[id], bytes: acc.get(id)!.bytes, files: acc.get(id)!.files }))
  const totalBytes = slices.reduce((sum, s) => sum + s.bytes, 0)
  return { kind: 'ready', totalBytes, quotaBytes, slices, largest, sections }
}

function quotaLine(used: number, quota: number | null): string {
  return quota == null ? fmtBytes(used) : `${fmtBytes(used)} of ${fmtBytes(quota)}`
}

/**
 * The page's data (M3e): `account_storage_breakdown` (worked out on this PC from the state database) and the quota
 * from the account storage summary. Used bytes on this page are the breakdown rows' sum, never the summary's used
 * bytes and never an example chart.
 */
function useInsights(initial: InsightsState) {
  const source = useDriveSource()
  const [model, setModel] = useState<InsightsModel | { kind: 'loading' }>(
    initial === 'error' ? { kind: 'error', text: FAILED_READ } : initial === 'empty' ? { kind: 'empty', quotaBytes: null } : { kind: 'loading' },
  )
  const load = useCallback(() => {
    setModel({ kind: 'loading' })
    Promise.all([
      command<StorageBreakdown>('account_storage_breakdown', { largestLimit: 50 }),
      source.storage().then((s) => s, () => null),
    ]).then(([r, storage]) => {
      const quota = storage && typeof storage.quotaBytes === 'number' ? storage.quotaBytes : null
      setModel(r.ok ? insightsFrom({ ok: true, value: r.value }, quota) : insightsFrom({ ok: false, unsupported: r.unsupported }, quota))
    })
  }, [source])
  useEffect(() => { if (initial === 'ready') load() }, [initial, load])
  return { model, retry: load }
}

export function InsightsScreen({ model, onRetry, onOpen, onAct, refused }: {
  model: InsightsModel
  onRetry: () => void
  onOpen?: (id: string, path: string) => void
  onAct?: (label: string, id: string) => void
  refused?: string | null
}) {
  const [menu, setMenu] = useState<{ id: string; x: number; y: number } | null>(null)
  useEffect(() => {
    if (!menu) return
    const off = () => setMenu(null)
    document.addEventListener('mousedown', off)
    return () => document.removeEventListener('mousedown', off)
  }, [menu])
  const heading = <BackLink to="storage">Storage</BackLink>
  if (model.kind === 'error') {
    return (
      <Page title="Storage insights" back={heading}>
        <ErrorBanner title="Couldn't load your insights" action={<Btn onClick={onRetry}>Try again</Btn>}>{model.text}</ErrorBanner>
      </Page>
    )
  }
  if (model.kind === 'empty') {
    return (
      <Page title="Storage insights" back={heading}>
        <Group>
          <div className="px-4 py-6 text-center">
            <div className="text-[14px] font-medium text-ink">Nothing stored yet</div>
            <p className="mt-1 text-[13px] text-ink-3" data-testid="quota-line">{model.quotaBytes == null ? 'Quota isn’t available.' : quotaLine(0, model.quotaBytes)}</p>
          </div>
        </Group>
      </Page>
    )
  }
  const maxBytes = Math.max(...model.slices.map((t) => t.bytes), 1)
  const shade: Record<Kind, string> = { Media: 'bg-ink', Documents: 'bg-ink-3', Other: 'bg-ink-4' }
  const fileCount = model.slices.reduce((sum, s) => sum + s.files, 0)
  const filesIn = (id: 'backups' | 'files') => model.largest.filter((f) => f.section === id)
  return (
    <Page title="Storage insights" back={heading}>
      <Group title="Encrypted storage">
        <div className="px-4 py-3">
          <div className="mb-1.5 flex items-baseline justify-between">
            <span className={`text-[13px] text-ink ${mono}`} data-testid="quota-line">{quotaLine(model.totalBytes, model.quotaBytes)}</span>
            <span className="text-[12px] text-ink-3">Drive plan</span>
          </div>
          {model.quotaBytes ? <>
            <Meter label="Encrypted storage used" fraction={model.totalBytes / model.quotaBytes} amber />
            <div className={`mt-1.5 flex justify-between text-[12px] text-ink-3 ${mono}`}><span>{Math.round((model.totalBytes / model.quotaBytes) * 100)}% used</span><span>{fmtBytes(model.quotaBytes - model.totalBytes)} free</span></div>
          </> : null}
        </div>
      </Group>
      <Group title="By type" note="Counts and sizes are the breakdown’s own rows.">
        <div className={`px-4 pt-3 text-[12px] text-ink-3 ${mono}`}>{countLabel(fileCount)} · {fmtBytes(model.totalBytes)}</div>
        {model.slices.map((t) => (
          <div key={t.label} className="px-4 py-2.5" data-testid="type-row">
            <div className="mb-1 flex justify-between text-[13px]"><span className="text-ink">{t.label}</span><span className={`text-ink-2 ${mono}`}>{fmtBytes(t.bytes)} · {countLabel(t.files)}</span></div>
            <div role="progressbar" aria-label={`${t.label} share`} aria-valuenow={Math.round((t.bytes / maxBytes) * 100)} className="h-1.5 overflow-hidden rounded-full bg-paper-3">
              <div className={`h-full rounded-full ${t.bytes === maxBytes ? shade.Media : shade[t.label as Kind] ?? shade.Other}`} style={{ width: `${(t.bytes / maxBytes) * 100}%` }} />
            </div>
          </div>
        ))}
      </Group>
      {refused ? <p role="alert" className="text-[12px] text-red-text">{refused}</p> : null}
      <Group title="Largest files, top 50" note="Worked out on this PC from the decrypted file list; Beebeeb's servers never see names or kinds. A section’s size is the sum of the files listed in it.">
        {model.sections.map((s) => (
          <div key={s.id} data-testid="section-row">
            <div className="flex min-h-[40px] items-center justify-between gap-3 px-4 py-1.5">
              <span className="text-[13px] text-ink">{s.label}</span>
              <span className={`text-[12px] text-ink-2 ${mono}`}>{fmtBytes(s.bytes)} · {countLabel(s.files)}</span>
            </div>
            {filesIn(s.id).map((f) => (
              <a key={f.id} href={`?section=${f.section}&sub=select:${encodeURIComponent(f.id)}`} title={f.path} data-testid="largest-row" data-row={f.id}
                onClick={(e) => { e.preventDefault(); onOpen?.(f.id, f.path) }}
                onContextMenu={(e) => { e.preventDefault(); setMenu({ id: f.id, x: e.clientX, y: e.clientY }) }}
                className="flex min-h-[40px] items-center gap-3 px-4 py-1.5 hover:bg-paper-2">
                <span className={`min-w-0 flex-1 truncate text-[12px] text-ink ${mono}`}>{f.shown}</span>
                <span className="w-16 shrink-0 text-[12px] text-ink-3">{f.kind}</span>
                <span className={`w-16 shrink-0 text-right text-[12px] text-ink-2 ${mono}`}>{fmtBytes(f.bytes)}</span>
              </a>
            ))}
          </div>
        ))}
      </Group>
      {menu ? (
        <ul role="menu" className="fixed z-50 min-w-[190px] rounded-lg border border-line bg-paper py-1 shadow-lg" style={{ left: menu.x, top: menu.y }}>
          {['Show in File Explorer', 'Move to Trash'].map((l) => <li key={l}><button role="menuitem" type="button" onMouseDown={(e) => e.stopPropagation()} onClick={() => { setMenu(null); onAct?.(l, menu.id) }} className="w-full px-3 py-1.5 text-left text-[13px] text-ink hover:bg-paper-2">{l}</button></li>)}
        </ul>
      ) : null}
    </Page>
  )
}

export function Insights({ state: initial = 'ready' }: { state?: InsightsState } = {}) {
  const source = useDriveSource()
  const { model, retry } = useInsights(initial)
  const [refused, setRefused] = useState<string | null>(null)
  const fail = (what: string) => (e: unknown) => setRefused(openFailureText(e, what))
  // A largest file shows in its folder, selected (DW-0783#1): in Backups for a device's backup, else in Files.
  const showInFiles = (id: string, path: string) => {
    setRefused(null)
    source.details(id).then((d) => navigateTo({ section: sectionOfPath(path), id: d.item.parentId, sub: `select:${id}` }), fail('show it in Files'))
  }
  // Its menu (DW-0783#2): Show in File Explorer, Move to Trash; each says so when it can't.
  const act = (label: string, id: string) => {
    setRefused(null)
    if (label === 'Show in File Explorer') source.revealInFileManager(id).catch(fail('show it in File Explorer'))
    else source.moveToTrash([id]).then(() => retry(), fail('move it to Trash'))
  }
  if (model.kind === 'loading') {
    return (
      <Page title="Storage insights" back={<BackLink to="storage">Storage</BackLink>}>
        {[0, 1, 2].map((i) => <Skeleton key={i} className="h-[120px] w-full" />)}
      </Page>
    )
  }
  return <InsightsScreen model={model} onRetry={() => { setRefused(null); retry() }} onOpen={showInFiles} onAct={act} refused={refused} />
}
