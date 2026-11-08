// Owned by batch B04 (design/desktop-v2/BATCHES.md). This scaffold list proves the DriveSource wiring
// end to end (list, live events, Ctrl+Shift+N); B04 replaces it with web's shared FileList (B01).

import { useCallback, useEffect, useState } from 'react'
import { BBButton, Icon } from '@beebeeb/shared/design'
import { useDriveSource, DriveSourceError, type Listing } from '@beebeeb/shared/drive-ui'
import { useCommand } from '../../shortcuts'
import type { SectionProps } from '../types'

const LOCAL_LABEL = {
  'online-only': 'Online only',
  downloaded: 'On this PC',
  'available-offline': 'Available offline',
  syncing: 'Syncing',
  failed: 'Not synced',
} as const

function bytes(n: number | null): string {
  if (n == null) return '—'
  if (n < 1024) return `${n} B`
  if (n < 1024 ** 2) return `${(n / 1024).toFixed(0)} KB`
  if (n < 1024 ** 3) return `${(n / 1024 ** 2).toFixed(1)} MB`
  return `${(n / 1024 ** 3).toFixed(1)} GB`
}

export default function Section({ nav }: SectionProps) {
  const source = useDriveSource()
  const folderId = nav.route.id
  const [listing, setListing] = useState<Listing | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [selected, setSelected] = useState<string | null>(null)

  const load = useCallback(() => {
    source.list(folderId).then(
      (l) => { setListing(l); setError(null) },
      (e: unknown) => setError(e instanceof DriveSourceError ? e.code : 'failed'),
    )
  }, [source, folderId])

  useEffect(() => {
    load()
    return source.subscribe((e) => { if (e.kind === 'changed') load() })
  }, [source, load])

  const newFolder = useCallback(() => {
    void source.createFolder(folderId, `New folder ${Date.now() % 1000}`).catch(() => undefined)
  }, [source, folderId])
  useCommand('new-folder', newFolder)

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex h-12 flex-none items-center gap-3 border-b border-line px-4">
        <h1 className="text-[15px] font-semibold text-ink">{listing?.folder.name ?? 'Files'}</h1>
        <span className="font-mono text-[12px] text-ink-3">{listing ? `${listing.items.length} items` : ''}</span>
        <span className="flex-1" />
        <BBButton variant="amber" size="sm" onClick={newFolder}>
          <Icon name="plus" size={14} /> New
        </BBButton>
      </div>
      {error && (
        <p role="alert" className="px-4 py-3 text-[13px] text-ink-2">
          {error === 'unsupported' ? 'This build cannot list files yet.' : 'The list could not be read.'}
        </p>
      )}
      <div role="grid" aria-label="Files" className="min-h-0 flex-1 overflow-auto">
        {listing?.items.map((item) => (
          <div
            key={item.id}
            role="row"
            aria-selected={selected === item.id}
            tabIndex={0}
            onClick={() => setSelected(item.id)}
            onDoubleClick={() => item.isFolder && nav.go({ section: 'files', id: item.id })}
            onKeyDown={(e) => { if (e.key === 'Enter' && item.isFolder) nav.go({ section: 'files', id: item.id }) }}
            className={`grid h-9 cursor-default grid-cols-[1fr_140px_100px_140px] items-center gap-3 px-4 text-[13px] ${selected === item.id ? 'bg-paper-3' : 'hover:bg-paper-2'}`}
          >
            <span role="gridcell" className="flex min-w-0 items-center gap-2 text-ink">
              <Icon name={item.isFolder ? 'folder' : 'file'} size={16} />
              <span className="truncate">{item.name}</span>
            </span>
            <span role="gridcell" className="font-mono text-[12px] text-ink-3">{item.modifiedAt.slice(0, 10)}</span>
            <span role="gridcell" className="font-mono text-[12px] text-ink-3">
              {item.isFolder ? `${item.childCount ?? 0} items` : bytes(item.size)}
            </span>
            <span role="gridcell" className="text-[12px] text-ink-3">{item.local ? LOCAL_LABEL[item.local] : ''}</span>
          </div>
        ))}
      </div>
    </div>
  )
}
