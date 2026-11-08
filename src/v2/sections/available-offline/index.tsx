// Available offline, live: items the source reports as kept on this PC; Remove goes through setAvailableOffline.
// DriveSource has no "list everything kept" or folder byte totals yet, so this reads the root and the folders one level
// down and shows sizes only where the source knows them (the PR's gap list).
import { useCallback, useEffect, useState } from 'react'
import { useDriveSource, type DriveItem } from '@beebeeb/shared/drive-ui'
import type { SectionProps } from '../types'
import { showToast } from '../../chrome/Toast'
import { OfflineView, type OfflineRow } from './OfflineView'

export default function Section(_props: SectionProps) {
  const source = useDriveSource()
  const [items, setItems] = useState<DriveItem[]>([])

  const load = useCallback(async () => {
    const root = await source.list(null)
    const nested = await Promise.all(root.items.filter((i) => i.isFolder && i.local !== 'available-offline').map((f) => source.list(f.id).then((l) => l.items, () => [] as DriveItem[])))
    setItems([...root.items, ...nested.flat()].filter((i) => i.local === 'available-offline'))
  }, [source])
  useEffect(() => {
    load().catch(() => setItems([]))
    return source.subscribe((e) => { if (e.kind === 'changed' || e.kind === 'item' || e.kind === 'removed') load().catch(() => {}) })
  }, [source, load])

  const rows: OfflineRow[] = items.map((i) => ({ id: i.id, name: i.name, isFolder: i.isFolder, items: i.childCount, bytes: i.size, mime: i.mime, status: 'Up to date' }))
  const remove = (ids: string[]) => Promise.all(ids.map((id) => source.setAvailableOffline(id, false))).then(load, () => showToast({ kind: 'failed', text: 'Couldn’t remove it from this PC' }))
  return (
    <OfflineView
      rows={rows}
      onOpen={(id) => { source.open(id).catch(() => showToast({ kind: 'failed', text: 'Couldn’t open it' })) }}
      onMenu={(id, r) => {
        if (id === 'open') source.open(r.id).catch(() => showToast({ kind: 'failed', text: 'Couldn’t open it' }))
        else if (id === 'reveal') source.revealInFileManager(r.id).catch(() => showToast({ kind: 'failed', text: 'Couldn’t show it in File Explorer' }))
        else void remove([r.id]).then(() => showToast({ kind: 'done', text: 'Removed from this PC. It stays in Beebeeb.' }))
      }}
      onRemoveAll={() => { void remove(items.map((i) => i.id)) }}
    />
  )
}
