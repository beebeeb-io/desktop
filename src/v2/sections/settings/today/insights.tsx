// Storage › Storage insights (DW-0783 family): where the storage goes, by type, and the files that take the most.
import { useCallback, useEffect, useState } from 'react'
import { useDriveSource } from '@beebeeb/shared/drive-ui'
import { command, type StorageBreakdown, type StorageCategory } from '../../../../desktopApi'
import { navigateTo } from '../../../routes'
import { openFailureText } from '../../shared/api'
import { Btn, Group, Meter, Page, Row, fmtBytes, mono } from '../shell/kit'
import type { Kind } from '../../../source/fixtures/settings-today'
import { BackLink, ErrorBanner, Skeleton } from './ui'

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
/** How many of the largest files the page lists; the rest of the top 50 are counted. */
const LISTED = 10

interface InsightsData {
  usedBytes: number
  quotaBytes: number
  files: number
  byType: { kind: Kind; bytes: number; files: number }[]
  largest: { id: string; path: string; kind: string; bytes: number }[]
}

/**
 * The page's data (M3e): `account_storage_breakdown` (worked out on this PC from the state database: by type, and the
 * 50 largest files with their ids) and the account's storage summary. It never shows example numbers as the account's.
 */
function useInsights(initial: InsightsState) {
  const source = useDriveSource()
  const [state, setState] = useState<InsightsState>(initial === 'ready' ? 'loading' : initial)
  const [data, setData] = useState<InsightsData | null>(null)
  const load = useCallback(() => {
    setState('loading')
    Promise.all([command<StorageBreakdown>('account_storage_breakdown', { largestLimit: 50 }), source.storage().catch(() => null)]).then(([r, storage]) => {
      if (!r.ok) { setState('error'); return }
      const b = r.value
      setData({
        usedBytes: storage?.usedBytes ?? b.total_bytes,
        quotaBytes: storage?.quotaBytes ?? 0,
        files: b.total_files,
        byType: b.by_category.map((c) => ({ kind: KIND_OF[c.category], bytes: c.bytes, files: c.file_count })),
        largest: b.largest_files.map((f) => ({ id: f.file_id, path: f.path, kind: kindOf(f.path, f.category), bytes: f.size_bytes })),
      })
      setState(b.total_files === 0 ? 'empty' : 'ready')
    })
  }, [source])
  useEffect(() => { if (initial === 'ready') load() }, [initial, load])
  return { state, data, retry: load }
}

export function Insights({ state: initial = 'ready' }: { state?: InsightsState } = {}) {
  const source = useDriveSource()
  const { state, data, retry } = useInsights(initial)
  const [menu, setMenu] = useState<{ i: number; x: number; y: number } | null>(null)
  const [refused, setRefused] = useState<string | null>(null)
  useEffect(() => {
    if (!menu) return
    const off = () => setMenu(null)
    document.addEventListener('mousedown', off)
    return () => document.removeEventListener('mousedown', off)
  }, [menu])
  const heading = <BackLink to="storage">Storage</BackLink>
  const fail = (what: string) => (e: unknown) => setRefused(openFailureText(e, what))
  // A largest file shows in its folder, selected (DW-0783#1): in Backups for a device's backup, else in Files.
  const showInFiles = (id: string, path: string) => {
    setRefused(null)
    source.details(id).then((d) => navigateTo({ section: sectionOfPath(path), id: d.item.parentId, sub: `select:${id}` }), fail('show it in Files'))
  }
  // Its menu (DW-0783#2): Show in File Explorer, Move to Trash; each says so when it can't.
  const act = (label: string, id: string) => {
    setMenu(null)
    setRefused(null)
    if (label === 'Show in File Explorer') source.revealInFileManager(id).catch(fail('show it in File Explorer'))
    else source.moveToTrash([id]).then(retry, fail('move it to Trash'))
  }
  if (state === 'loading') {
    return (
      <Page title="Storage insights" back={heading}>
        {[0, 1, 2].map((i) => <Skeleton key={i} className="h-[120px] w-full" />)}
      </Page>
    )
  }
  if (state === 'error') {
    return (
      <Page title="Storage insights" back={heading}>
        <ErrorBanner title="Couldn't load your insights" action={<Btn onClick={retry}>Try again</Btn>}>
          <span className={`text-[12px] ${mono}`}>The file list didn't answer in time.</span>
        </ErrorBanner>
      </Page>
    )
  }
  if (state === 'empty') {
    return (
      <Page title="Storage insights" back={heading}>
        <Group><div className="px-4 py-6 text-center"><div className="text-[14px] font-medium text-ink">Nothing is stored yet</div><p className="text-[13px] text-ink-3">When you add files, this page shows where the storage goes, by type, and which files take the most.</p></div></Group>
      </Page>
    )
  }
  if (!data) return null
  const d = data
  const maxBytes = Math.max(...d.byType.map((t) => t.bytes), 1)
  const shade: Record<Kind, string> = { Media: 'bg-ink', Documents: 'bg-ink-3', Other: 'bg-ink-4' }
  return (
    <Page title="Storage insights" back={heading}>
      <Group title="Encrypted storage">
        <div className="px-4 py-3">
          <div className="mb-1.5 flex items-baseline justify-between"><span className={`text-[13px] text-ink ${mono}`}>{d.quotaBytes ? `${fmtBytes(d.usedBytes)} of ${fmtBytes(d.quotaBytes)}` : fmtBytes(d.usedBytes)}</span><span className="text-[12px] text-ink-3">Drive plan</span></div>
          {d.quotaBytes ? <>
            <Meter label="Encrypted storage used" fraction={d.usedBytes / d.quotaBytes} amber />
            <div className={`mt-1.5 flex justify-between text-[12px] text-ink-3 ${mono}`}><span>{Math.round((d.usedBytes / d.quotaBytes) * 100)}% used</span><span>{fmtBytes(d.quotaBytes - d.usedBytes)} free</span></div>
          </> : null}
        </div>
      </Group>
      <Group title="By type" note="Counts and sizes include Backups and Trash.">
        <div className={`px-4 pt-3 text-[12px] text-ink-3 ${mono}`}>{d.files.toLocaleString('en')} files · {fmtBytes(d.usedBytes)}</div>
        {d.byType.map((t) => (
          <div key={t.kind} className="px-4 py-2.5" data-testid="type-row">
            <div className="mb-1 flex justify-between text-[13px]"><span className="text-ink">{t.kind}</span><span className={`text-ink-2 ${mono}`}>{fmtBytes(t.bytes)} · {t.files.toLocaleString('en')} files</span></div>
            <div role="progressbar" aria-label={`${t.kind} share`} aria-valuenow={Math.round((t.bytes / maxBytes) * 100)} className="h-1.5 overflow-hidden rounded-full bg-paper-3">
              <div className={`h-full rounded-full ${t.bytes === maxBytes ? shade.Media : shade[t.kind]}`} style={{ width: `${(t.bytes / maxBytes) * 100}%` }} />
            </div>
          </div>
        ))}
      </Group>
      {refused ? <p role="alert" className="text-[12px] text-red-text">{refused}</p> : null}
      <Group title="Largest files, top 50" note="Worked out on this PC from the decrypted file list; Beebeeb's servers never see names or kinds.">
        {d.largest.slice(0, LISTED).map((f, i) => (
          <a key={f.id} href={`?section=${sectionOfPath(f.path)}&sub=select:${encodeURIComponent(f.id)}`} title={f.path} data-testid="largest-row" data-row={f.id}
            onClick={(e) => { e.preventDefault(); showInFiles(f.id, f.path) }}
            onContextMenu={(e) => { e.preventDefault(); setMenu({ i, x: e.clientX, y: e.clientY }) }}
            className="flex min-h-[40px] items-center gap-3 px-4 py-1.5 hover:bg-paper-2">
            <span className={`min-w-0 flex-1 truncate text-[12px] text-ink ${mono}`}>{cutPath(f.path)}</span>
            <span className="w-16 shrink-0 text-[12px] text-ink-3">{f.kind}</span>
            <span className={`w-16 shrink-0 text-right text-[12px] text-ink-2 ${mono}`}>{fmtBytes(f.bytes)}</span>
          </a>
        ))}
        {d.largest.length > LISTED ? <Row label={<span className="text-ink-3">and {d.largest.length - LISTED} more</span>} /> : null}
      </Group>
      {menu ? (
        <ul role="menu" className="fixed z-50 min-w-[190px] rounded-lg border border-line bg-paper py-1 shadow-lg" style={{ left: menu.x, top: menu.y }}>
          {['Show in File Explorer', 'Move to Trash'].map((l) => <li key={l}><button role="menuitem" type="button" onMouseDown={(e) => e.stopPropagation()} onClick={() => act(l, d.largest[menu.i].id)} className="w-full px-3 py-1.5 text-left text-[13px] text-ink hover:bg-paper-2">{l}</button></li>)}
        </ul>
      ) : null}
    </Page>
  )
}
