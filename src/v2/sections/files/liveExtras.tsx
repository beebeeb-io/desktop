// B02b: what Files and Backups draw live from the extras source (../../source/extras.ts): This PC's row in Backups,
// per-row transfer progress, the Uploads panel (DW-0427), the getting-ready strip (DW-1107) and the conflict dialog
// (DW-0429). Values the engine can't know stay absent; nothing here is invented.
import { useCallback, useEffect, useMemo, useState } from 'react'
import { command } from '../../../desktopApi'
import { showToast } from '../../chrome/Toast'
import { useDriveExtras, type BackupView, type DriveStatus, type TransferRow } from '../../source/extras'
import type { BackupMeta, Transfer } from './FilesPane'
import type { UploadRow } from './Panels'
import { fmtSize } from './model'

/** The status line asks for the Uploads panel with this window event (DW-0427: "a panel over the status line"). */
export const UPLOADS_OPEN_EVENT = 'bb:uploads-open'

const BACKUP_STATUS: Record<BackupView['state'], string> = {
  on: 'Backed up',
  'getting-ready': 'Backing up for the first time',
  stopped: 'Stopped',
  off: 'Off',
}

/** Backups' columns: only This PC's own folder has a known last backup and status; the rest show "—". */
export function backupMetaFor(status: DriveStatus | null, items: { id: string; name: string }[]): Record<string, BackupMeta> {
  const folder = status?.backup.deviceFolder
  if (!status || !folder) return {}
  const row = items.find((i) => i.name === folder)
  if (!row) return {}
  return { [row.id]: { last: status.backup.lastBackupAt, status: BACKUP_STATUS[status.backup.state], thisPc: true } }
}

export function transfersByItem(rows: TransferRow[]): Record<string, Transfer> {
  const out: Record<string, Transfer> = {}
  for (const r of rows) {
    if (!r.fileId || r.state === 'stopped') continue
    out[r.fileId] = { kind: r.state === 'waiting' ? 'waiting' : r.direction === 'up' ? 'uploading' : 'downloading', done: r.bytes, total: r.total ?? 0 }
  }
  return out
}

export function uploadRows(rows: TransferRow[]): UploadRow[] {
  return rows.filter((r) => r.direction === 'up').map((r) => ({
    id: r.id, name: r.name, total: r.total ?? 0, done: r.bytes,
    state: r.state === 'stopped' ? 'attention' : r.state === 'moving' ? 'uploading' : 'waiting',
    note: r.reason, to: r.state === 'waiting' && r.folder ? r.folder.split(/[\\/]/).pop() : undefined,
  }))
}

/** "2 of 4 · 42 MB left · Continues in the background", with only the parts the rows know. */
export function uploadSummary(rows: UploadRow[]): string {
  const live = rows.filter((r) => r.state !== 'attention')
  const moving = live.filter((r) => r.state === 'uploading').length
  const known = live.length > 0 && live.every((r) => r.total > 0)
  const left = live.reduce((s, r) => s + Math.max(0, r.total - (r.done ?? 0)), 0)
  if (rows.length === 0) return 'Nothing is uploading'
  return [`${moving} of ${rows.length}`, known ? `${fmtSize(left)} left` : null, live.length ? 'Continues in the background' : null].filter(Boolean).join(' · ')
}

export function useLiveFolderExtras() {
  const extras = useDriveExtras()
  const [status, setStatus] = useState<DriveStatus | null>(null)
  const [rows, setRows] = useState<TransferRow[]>([])
  const [uploadsOpen, setUploadsOpen] = useState(false)
  const [dismissed, setDismissed] = useState<ReadonlySet<string>>(new Set())

  const refresh = useCallback(() => {
    extras.status().then(setStatus, () => setStatus(null))
    extras.transfers().then(setRows, () => setRows([]))
  }, [extras])
  useEffect(() => {
    refresh()
    const timer = window.setInterval(refresh, 10_000)
    // Live progress: the moving rows replace their earlier copies; the queue's rows stay until the next read.
    const stop = extras.subscribeTransfers((moving) => setRows((prev) => {
      const ids = new Set(moving.map((m) => m.id))
      const rest = prev.filter((r) => r.state !== 'moving' && !ids.has(r.id) && !moving.some((m) => m.fileId && m.fileId === r.fileId))
      return [...moving, ...rest]
    }))
    const open = () => setUploadsOpen(true)
    window.addEventListener(UPLOADS_OPEN_EVENT, open)
    return () => { window.clearInterval(timer); stop(); window.removeEventListener(UPLOADS_OPEN_EVENT, open) }
  }, [extras, refresh])

  const uploads = useMemo(() => uploadRows(rows), [rows])
  const conflicts = (status?.conflicts ?? []).filter((c) => !dismissed.has(c.id))
  const retry = (id: string) => {
    extras.retry([id]).then(({ retried }) => { showToast(retried > 0 ? { kind: 'done', text: 'Trying again' } : { kind: 'failed', text: 'Beebeeb is already trying again' }); refresh() },
      () => showToast({ kind: 'failed', text: 'Couldn’t try again' }))
  }
  const resolve = (id: string, choice: 'mine' | 'theirs' | 'both') => {
    const wire = choice === 'mine' ? 'local' : choice === 'theirs' ? 'remote' : 'both'
    command<void>('resolve_conflict', { fileId: id, choice: wire }).then(
      () => { setDismissed((d) => new Set(d).add(id)); refresh() },
      () => showToast({ kind: 'failed', text: 'Couldn’t keep that version. Nothing was changed.' }),
    )
  }
  return {
    status, rows, uploads, uploadsOpen, closeUploads: () => setUploadsOpen(false), retry,
    transfers: transfersByItem(rows), conflicts, resolve, later: (id: string) => setDismissed((d) => new Set(d).add(id)),
  }
}
