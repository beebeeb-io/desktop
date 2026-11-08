// B12's data layer. Everything comes through command() from desktopApi (never the shared API); a plain browser
// (headless frames, bun run dev) has no IPC bridge, so the board's fixture values stand in. Fixtures live here
// (settings area only) and carry the board's example numbers.
import { useCallback, useEffect, useState } from 'react'
import { command, DEFAULT_CONFIG, type DesktopConfig } from '../../../../desktopApi'
import { hasTauri } from '../../../source'
import { GB, MB } from './kit'

export const FIXTURE_CONFIG: DesktopConfig = { ...DEFAULT_CONFIG, sync_overlays: true, local_cache_limit_bytes: 50 * GB }

/** Desktop settings: read once, written whole through set_desktop_config. */
export function useConfig(initial: Partial<DesktopConfig> = {}) {
  const [config, setConfig] = useState<DesktopConfig>({ ...FIXTURE_CONFIG, ...initial })
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    if (!hasTauri()) return
    let live = true
    command<DesktopConfig>('get_desktop_config').then((r) => {
      if (!live) return
      if (r.ok) setConfig({ ...DEFAULT_CONFIG, ...r.value, ...initial })
      else setError(r.reason)
    })
    return () => { live = false }
  }, [])
  const update = useCallback((patch: Partial<DesktopConfig>) => {
    setConfig((c) => {
      const next = { ...c, ...patch }
      if (hasTauri()) command('set_desktop_config', { config: next }).then((r) => { if (!r.ok) setError(r.reason) })
      return next
    })
  }, [])
  return { config, update, error }
}

export interface KnownFolder {
  key: string
  display_name: string
  enabled: boolean
  item_count: number | null
  /** Fixture-only: the board's size and last-backup text. The desktop reports an item count instead. */
  size?: string
  last?: string
  custom?: boolean
}

export const FIXTURE_FOLDERS: KnownFolder[] = [
  { key: 'desktop', display_name: 'Desktop', enabled: true, item_count: 412, size: '3.2 GB', last: '09:30' },
  { key: 'documents', display_name: 'Documents', enabled: true, item_count: 1820, size: '14.6 GB', last: '09:30' },
  { key: 'pictures', display_name: 'Pictures', enabled: true, item_count: 6208, size: '8.4 GB', last: '09:28' },
  { key: 'music', display_name: 'Music', enabled: false, item_count: 5344 },
  { key: 'videos', display_name: 'Videos', enabled: false, item_count: 88 },
  { key: 'downloads', display_name: 'Downloads', enabled: false, item_count: 301 },
  { key: 'projects', display_name: 'Projects', enabled: true, item_count: 905, size: '1.6 GB', last: '09:30', custom: true },
]

export function useKnownFolders() {
  const [folders, setFolders] = useState<KnownFolder[]>(FIXTURE_FOLDERS)
  useEffect(() => {
    if (!hasTauri()) return
    command<KnownFolder[]>('get_known_folder_backup').then((r) => { if (r.ok) setFolders(r.value) })
  }, [])
  const toggle = useCallback((key: string, enabled: boolean) => {
    setFolders((fs) => fs.map((f) => (f.key === key ? { ...f, enabled } : f)))
    if (hasTauri() && !FIXTURE_FOLDERS.find((f) => f.key === key)?.custom) command('set_known_folder_backup', { key, enabled })
  }, [])
  const remove = useCallback((key: string) => setFolders((fs) => fs.filter((f) => f.key !== key)), [])
  const add = useCallback((name: string) => setFolders((fs) => [...fs, { key: name.toLowerCase(), display_name: name, enabled: true, item_count: null, custom: true }]), [])
  return { folders, toggle, remove, add }
}

export interface StorageBreakdown {
  usedBytes: number
  quotaBytes: number
  rows: { label: string; bytes: number }[]
}
export const FIXTURE_BREAKDOWN: StorageBreakdown = {
  usedBytes: 240 * GB,
  quotaBytes: 1024 * GB,
  rows: [
    { label: 'Photos and videos', bytes: 216 * GB },
    { label: 'Files', bytes: 19.5 * GB },
    { label: 'Thumbnails and previews', bytes: 3.1 * GB },
    { label: 'Older versions', bytes: 1.2 * GB },
    { label: 'Trash', bytes: 240 * MB },
  ],
}
export const FIXTURE_LOCAL = { downloaded: 18.4 * GB, previews: 3.1 * GB, offline: 1.4 * GB, offlineItems: 3, previewLimit: 20 * GB, freeOnPc: 186 * 2 * GB }

export const FIXTURE_DEVICES = [
  { id: 'iphone', name: "Anna's iPhone", status: 'Backed up 12,480 photos · 96 GB' },
  { id: 'ipad', name: "Anna's iPad", status: 'Backed up 4,120 photos · 38 GB' },
  { id: 'old', name: 'Old iPhone', status: 'Backed up 2,040 photos · 11 GB' },
]
