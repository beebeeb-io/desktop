// Available offline, live: items the source reports as kept on this PC; Remove goes through setAvailableOffline.
// The list and the folder totals come from B02b's drive_offline_list (the top-most kept items, a folder with its files
// at every level).
import { useCallback, useEffect, useState } from 'react'
import { useDriveSource } from '@beebeeb/shared/drive-ui'
import { useDriveExtras, type OfflineEntry } from '../../source/extras'
import type { SectionProps } from '../types'
import { showToast } from '../../chrome/Toast'
import { OfflineView, type OfflineRow } from './OfflineView'

export default function Section(_props: SectionProps) {
  const source = useDriveSource()
  const extras = useDriveExtras()
  const [entries, setEntries] = useState<OfflineEntry[]>([])
  const items = entries.map((e) => e.item)

  const load = useCallback(async () => { setEntries(await extras.offlineList()) }, [extras])
  useEffect(() => {
    load().catch(() => setEntries([]))
    return source.subscribe((e) => { if (e.kind === 'changed' || e.kind === 'item' || e.kind === 'removed') load().catch(() => {}) })
  }, [source, load])

  const rows: OfflineRow[] = entries.map(({ item: i, items: n, bytes }) => ({
    id: i.id, name: i.name, isFolder: i.isFolder, items: n, bytes, mime: i.mime, status: i.local === 'syncing' ? 'Downloading' : 'Up to date',
  }))
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
