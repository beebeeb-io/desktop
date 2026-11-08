// The window the B03 frames are drawn in: the caption bar, the sidebar, a Files toolbar and list stand-in, the
// status line. B04 builds the real Files section; these frames show only the global element in its window, so
// the list here is the fixtures' rows, static.

import type { ReactNode } from 'react'
import { BBButton } from '@beebeeb/shared/design'
import type { StorageSummary, SyncSummary } from '@beebeeb/shared/drive-ui'
import { CaptionBarView } from './CaptionBar'
import { Sidebar } from './Sidebar'
import { StatusLine } from './StatusLine'
import { rootItems } from '../source/fixtures'
import type { SectionId } from '../routes'

const GB = 1024 ** 3
export const STORAGE_240: StorageSummary = { usedBytes: 240 * GB, quotaBytes: 1024 * GB, filesBytes: 150 * GB, photosBytes: 60 * GB, backupsBytes: 30 * GB }
export const SYNC_OK: SyncSummary = { phase: 'up-to-date', onDevice: 4, availableOffline: 3, onlineOnly: 4, pending: 0 }

export function Toolbar() {
  return (
    <div className="flex h-12 flex-none items-center gap-3 border-b border-line px-4">
      <h1 className="m-0 text-[15px] font-semibold text-ink">Files</h1>
      <span className="flex-1" />
      <BBButton variant="amber" size="md">New</BBButton>
    </div>
  )
}

function size(bytes: number | null): string {
  if (bytes === null) return ''
  return bytes >= 1024 ** 2 ? `${(bytes / 1024 ** 2).toFixed(1)} MB` : `${Math.round(bytes / 1024)} KB`
}

export function ListBackdrop({ dim }: { dim?: boolean }) {
  return (
    <ul className={`m-0 flex-1 list-none overflow-hidden p-0 ${dim ? 'opacity-60' : ''}`}>
      {rootItems().map((item) => (
        <li key={item.id} data-row className="flex h-10 items-center gap-3 border-b border-line px-4 text-[13px]">
          <span className="min-w-0 flex-1 truncate text-ink">{item.name}</span>
          <span className="w-24 text-right font-mono text-[12px] text-ink-3">{item.isFolder ? `${item.childCount} items` : size(item.size)}</span>
        </li>
      ))}
    </ul>
  )
}

export function FrameWindow({
  section = 'files', banner, strip, children, aside, storage = STORAGE_240, sync = SYNC_OK, counts, overlay, title = 'Beebeeb', toast,
}: {
  section?: SectionId
  banner?: ReactNode
  /** A slim strip above the list (DW-0118). */
  strip?: ReactNode
  children?: ReactNode
  /** The preview panel at the right (DW-0121). */
  aside?: ReactNode
  storage?: StorageSummary | null
  sync?: SyncSummary | null
  counts?: Partial<Record<SectionId, number>>
  overlay?: ReactNode
  title?: string
  toast?: ReactNode
}) {
  return (
    <div className="relative flex h-screen flex-col overflow-hidden bg-paper font-sans text-ink">
      <CaptionBarView title={title} />
      <div className="flex min-h-0 flex-1">
        <Sidebar active={section} onGo={() => {}} storage={storage} accountName="Anna" counts={counts} />
        <main className="flex min-w-0 flex-1 flex-col">
          <Toolbar />
          {banner}
          {strip}
          {children ?? <ListBackdrop />}
        </main>
        {aside}
      </div>
      <StatusLine sync={sync} />
      {toast && <div className="pointer-events-none absolute inset-x-0 bottom-10 z-40 flex justify-center">{toast}</div>}
      {overlay}
    </div>
  )
}
