// Available offline, live: top-most kept items from drive_offline_list, with that command's folder counts, removed through set_recursive_pin.
// A syncing row says Downloading until a source event. A failed read is not an empty list.
import { useCallback, useEffect, useState, type ReactNode } from 'react'
import { useDriveSource } from '@beebeeb/shared/drive-ui'
import { command } from '../../../desktopApi'
import { useDriveExtras, type OfflineEntry } from '../../source/extras'
import type { SectionProps } from '../types'
import { showToast } from '../../chrome/Toast'
import { BTN } from '../files/ui'
import { OfflineView, type OfflineRow } from './OfflineView'

function readFailure(error: unknown): string {
  if (error instanceof Error && error.message.trim()) return error.message.trim()
  if (typeof error === 'string' && error.trim()) return error.trim()
  return 'Couldn’t read what is kept on this PC.'
}

/** Ids set_recursive_pin says it changed. Absent when the answer is not that shape: no count is invented. */
function changedIds(value: unknown): string[] | null {
  if (!value || typeof value !== 'object') return null
  const raw = (value as Record<string, unknown>).changed_item_ids ?? (value as Record<string, unknown>).changedItemIds
  if (!Array.isArray(raw) || raw.some((id) => typeof id !== 'string')) return null
  return raw as string[]
}

/** A kept item whose parent is also kept is already inside that folder's row. */
function topMost(entries: OfflineEntry[]): OfflineEntry[] {
  const ids = new Set(entries.map((entry) => entry.item.id))
  return entries.filter((entry) => entry.item.parentId == null || !ids.has(entry.item.parentId))
}

function rowOf(entry: OfflineEntry): OfflineRow {
  return {
    id: entry.item.id,
    name: entry.item.name,
    isFolder: entry.item.isFolder,
    items: entry.item.isFolder && typeof entry.items === 'number' ? entry.items : undefined,
    bytes: typeof entry.bytes === 'number' ? entry.bytes : null,
    mime: entry.item.mime,
    status: entry.item.local === 'syncing' ? 'Downloading' : 'Up to date',
  }
}

function Shell({ children }: { children: ReactNode }) {
  return (
    <div className="flex min-h-0 flex-1 flex-col" data-section="available-offline">
      <div className="flex h-12 flex-none items-center gap-3 border-b border-line px-4">
        <h1 className="m-0 text-[15px] font-semibold text-ink">Available offline</h1>
      </div>
      {children}
    </div>
  )
}

export default function Section(_props: SectionProps) {
  const source = useDriveSource()
  const extras = useDriveExtras()
  const [entries, setEntries] = useState<OfflineEntry[] | null>(null)
  const [readError, setReadError] = useState<string | null>(null)

  const load = useCallback(async () => {
    try {
      const next = await extras.offlineList()
      if (!Array.isArray(next)) throw new Error('Couldn’t read what is kept on this PC.')
      setEntries(next)
      setReadError(null)
    } catch (error) {
      setEntries(null)
      setReadError(readFailure(error))
    }
  }, [extras])

  useEffect(() => {
    void load()
    return source.subscribe((event) => {
      if (event.kind === 'item') {
        const { id, local } = event.item
        setEntries((prev) => prev?.map((entry) => entry.item.id === id ? { ...entry, item: { ...entry.item, local } } : entry) ?? prev)
        return
      }
      if (event.kind === 'changed' || event.kind === 'removed') void load()
    })
  }, [source, load])

  const remove = async (ids: string[]) => {
    const gone = new Set<string>()
    let count = 0
    let failure: string | null = null
    for (const id of ids) {
      const result = await command<unknown>('set_recursive_pin', { itemId: id, pinned: false })
      if (!result.ok) {
        failure = result.reason.trim() || 'Couldn’t remove it from this PC.'
        break
      }
      const changed = changedIds(result.value)
      if (!changed) {
        failure = 'Couldn’t remove it from this PC.'
        break
      }
      count += changed.length
      for (const changedId of changed) gone.add(changedId)
    }
    if (gone.size > 0) setEntries((prev) => prev?.filter((entry) => !gone.has(entry.item.id)) ?? prev)
    if (failure) showToast({ kind: 'failed', text: failure })
    else showToast({ kind: 'done', text: `Removed ${count} from this PC.` })
  }

  if (readError) {
    return (
      <Shell>
        <div className="flex flex-1 flex-col items-center justify-center gap-3 px-8 text-center" role="alert">
          <p className="m-0 max-w-sm text-[13px] leading-5 text-ink">{readError}</p>
          <button type="button" className={BTN} onClick={() => { void load() }}>Try again</button>
        </div>
      </Shell>
    )
  }
  if (!entries) return <Shell><span className="sr-only">Loading</span></Shell>

  const rows = topMost(entries).map(rowOf)
  if (rows.length === 0) {
    return (
      <Shell>
        <div className="flex flex-1 flex-col items-center justify-center gap-3 px-8 text-center" data-empty="offline">
          <h2 className="m-0 text-[15px] font-semibold text-ink">Nothing is kept on this PC</h2>
          <p className="m-0 max-w-sm text-[13px] leading-5 text-ink-3">
            Make a file or folder available offline from <a href="?section=files" className="text-ink underline">Files</a>.
          </p>
        </div>
      </Shell>
    )
  }

  return (
    <OfflineView
      rows={rows}
      onOpen={(id) => { source.open(id).catch(() => showToast({ kind: 'failed', text: 'Couldn’t open it' })) }}
      onMenu={(id, row) => {
        if (id === 'open') source.open(row.id).catch(() => showToast({ kind: 'failed', text: 'Couldn’t open it' }))
        else if (id === 'reveal') source.revealInFileManager(row.id).catch(() => showToast({ kind: 'failed', text: 'Couldn’t show it in File Explorer' }))
        else void remove([row.id])
      }}
      onRemoveAll={() => { void remove(rows.map((row) => row.id)) }}
    />
  )
}
