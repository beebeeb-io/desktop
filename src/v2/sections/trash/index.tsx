// Trash lists only what drive_trash returned. Restore reports that command's count and leaves a failed
// row in place. Delete for good asks for the password only after drive_delete_permanently requires it.
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { DriveItem, TrashEntry } from '@beebeeb/shared/drive-ui'
import type { SectionProps } from '../types'
import { command, commandUnavailableLabel, type CommandResult } from '../../../desktopApi'
import { showToast, type ToastMessage } from '../../chrome/Toast'
import { Menu } from '../files/ui'
import { DeleteForGoodDialog, TrashDetailsDialog, TrashView, totalBytes, trashMenu } from './TrashView'
import { publishCount } from '../../chrome/counts'
import { useCommand } from '../../shortcuts'

/**
 * One restored item names where it went. A count from the command is that count, including when it is
 * smaller than the rows that were sent. Several rows never borrow a single path.
 */
export function restoredText(restored: TrashEntry[], count = restored.length): string {
  if (count === 1 && restored.length === 1) return `Restored to ${restored[0]!.originalPath || 'Files'}`
  return `${count} ${count === 1 ? 'item' : 'items'} restored`
}

const FACTOR = /^(invalid_credentials|password_required|confirmation_required|step_up_required)\b/
const WRONG_PASSWORD = /^(invalid_credentials|wrong_password)\b/

function codeOf(reason: string): string {
  return reason.split(':')[0]?.trim() ?? ''
}

/** The command's own count, or the whole call when it accepted every id. Any other shape is unknown. */
function confirmedCount(value: unknown, sent: number): number | null {
  if (value == null) return sent
  let raw: unknown
  if (typeof value === 'number') raw = value
  else if (typeof value === 'object') {
    const row = value as { count?: unknown; restored?: unknown }
    if ('count' in row) raw = row.count
    else if ('restored' in row) raw = row.restored
    else return null
  } else return null
  if (typeof raw === 'number' && Number.isInteger(raw) && raw >= 0 && raw <= sent) return raw
  return null
}

function rowsFrom(value: unknown): TrashEntry[] | null {
  if (!Array.isArray(value)) return null
  const rows: TrashEntry[] = []
  for (const raw of value) {
    const row = asTrash(raw)
    if (!row) return null
    rows.push(row)
  }
  rows.sort((a, b) => b.trashedAt.localeCompare(a.trashedAt))
  return rows
}

function asTrash(raw: unknown): TrashEntry | null {
  if (!raw || typeof raw !== 'object') return null
  const row = raw as { item?: unknown; trashedAt?: unknown; originalPath?: unknown }
  if (!row.item || typeof row.item !== 'object' || typeof row.trashedAt !== 'string') return null
  const item = row.item as Partial<DriveItem>
  if (typeof item.id !== 'string' || typeof item.name !== 'string') return null
  const shared = item.shared && typeof item.shared === 'object' && 'kind' in item.shared ? item.shared : { kind: 'none' as const }
  return {
    item: {
      id: item.id,
      parentId: typeof item.parentId === 'string' ? item.parentId : null,
      name: item.name,
      isFolder: item.isFolder === true,
      size: typeof item.size === 'number' ? item.size : null,
      modifiedAt: typeof item.modifiedAt === 'string' ? item.modifiedAt : row.trashedAt,
      shared,
      local: typeof item.local === 'string' ? item.local : null,
      hasThumbnail: item.hasThumbnail === true,
      ...(typeof item.childCount === 'number' ? { childCount: item.childCount } : {}),
    },
    trashedAt: row.trashedAt,
    originalPath: typeof row.originalPath === 'string' ? row.originalPath : '',
  }
}

function restoreWhy(reason: string, unsupported: boolean): string {
  if (unsupported) return commandUnavailableLabel('drive_restore_from_trash')
  if (codeOf(reason) === 'not-found') return 'Couldn’t restore. It isn’t in the trash.'
  if (codeOf(reason) === 'offline') return 'Couldn’t restore. You’re offline.'
  return 'Couldn’t restore. Try again.'
}

function deleteWhy(reason: string, unsupported: boolean): string {
  if (unsupported) return commandUnavailableLabel('drive_delete_permanently')
  if (WRONG_PASSWORD.test(codeOf(reason))) return 'Incorrect password.'
  if (codeOf(reason) === 'not-found') return 'It isn’t in the trash. Nothing was deleted.'
  if (codeOf(reason) === 'offline') return 'You’re offline. Nothing was deleted.'
  return 'Couldn’t delete. Try again.'
}

export default function Section({ nav }: SectionProps) {
  const [entries, setEntries] = useState<TrashEntry[] | null>(null)
  const [loadFailed, setLoadFailed] = useState(false)
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [anchor, setAnchor] = useState<string | null>(null)
  const [menu, setMenu] = useState<{ entry: TrashEntry; x: number; y: number } | null>(null)
  const [confirm, setConfirm] = useState<{ ids: string[]; all: boolean } | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [details, setDetails] = useState<TrashEntry | null>(null)
  const now = useMemo(() => new Date(), [])
  const loadGen = useRef(0)
  const busyRef = useRef(false)
  const list = entries ?? []

  const load = useCallback(async () => {
    const gen = ++loadGen.current
    const result = await command<unknown>('drive_trash')
    if (gen !== loadGen.current) return
    if (!result.ok) { setLoadFailed(true); return }
    const rows = rowsFrom(result.value)
    if (!rows) { setLoadFailed(true); return }
    publishCount('trash', rows.length)
    setEntries(rows)
    setLoadFailed(false)
  }, [])

  useEffect(() => { void load() }, [load])

  const select = (id: string, o: { ctrl: boolean; shift: boolean }) => {
    if (o.shift && anchor) {
      const ids = list.map((e) => e.item.id)
      const [a, b] = [ids.indexOf(anchor), ids.indexOf(id)].sort((x, y) => x - y)
      setSelected(new Set(ids.slice(a, b + 1)))
      return
    }
    setAnchor(id)
    setSelected((s) => { const n = new Set(o.ctrl ? s : []); if (o.ctrl && s.has(id)) n.delete(id); else n.add(id); return n })
  }

  const restore = async (ids: string[]) => {
    if (ids.length === 0 || busyRef.current) return
    const snapshot = list.filter((e) => ids.includes(e.item.id))
    busyRef.current = true
    const result = await command<unknown>('drive_restore_from_trash', { ids })
    busyRef.current = false
    if (!result.ok) {
      showToast({ kind: 'failed', text: restoreWhy(result.reason, result.unsupported) })
      return
    }
    const count = confirmedCount(result.value, ids.length)
    const text = count == null ? 'Restored.' : restoredText(snapshot, count)
    const toast: ToastMessage = count === ids.length
      ? { kind: 'undo', text, onAction: () => void putBack(ids) }
      : { kind: 'done', text }
    await load()
    showToast(toast)
  }

  const putBack = async (ids: string[]) => {
    const back = await command('drive_trash_items', { ids })
    if (!back.ok) showToast({ kind: 'failed', text: 'Couldn’t move it back to Trash. Try again.' })
    else await load()
  }

  const remove = async (ids: string[], all: boolean, secret?: { password?: string }) => {
    if (ids.length === 0 || busyRef.current) return
    busyRef.current = true
    setBusy(true)
    if (secret) setError(null)
    const result = await command<unknown>('drive_delete_permanently', {
      ids,
      password: secret?.password ?? null,
      code: null,
    })
    busyRef.current = false
    setBusy(false)
    if (!result.ok) {
      if (secret == null && asksForFactor(result)) {
        setError(null)
        setConfirm({ ids, all })
        return
      }
      if (secret) {
        setError(deleteWhy(result.reason, result.unsupported))
        setConfirm({ ids, all })
        return
      }
      showToast({ kind: 'failed', text: deleteWhy(result.reason, result.unsupported) })
      return
    }
    setConfirm(null)
    setError(null)
    setSelected(new Set())
    await load()
    showToast({ kind: 'done', text: all || ids.length !== 1 ? 'Trash emptied' : 'Deleted for good' })
  }

  // Alt+Enter on one selected item opens Details, unless a dialog or the menu already has the keyboard.
  useCommand('details', useCallback(() => {
    if (selected.size !== 1 || confirm || menu || details) return
    const one = list.find((e) => selected.has(e.item.id))
    if (one) setDetails(one)
  }, [selected, list, confirm, menu, details]))

  const target = confirm ? list.filter((e) => confirm.ids.includes(e.item.id)) : []
  return (
    <>
      <TrashView
        entries={list} now={now} selected={selected} menuOn={menu?.entry.item.id}
        ready={entries !== null || loadFailed} failed={loadFailed && entries === null}
        onRetry={() => void load()} onFiles={() => nav.go({ section: 'files' })}
        onSelect={select}
        onContext={(entry, x, y) => { if (!selected.has(entry.item.id)) setSelected(new Set([entry.item.id])); setMenu({ entry, x, y }) }}
        onDelete={() => void remove([...selected], false)}
        onPreview={(e) => nav.go({ section: 'open', id: e.item.id, sub: 'preview' })}
        onEmpty={() => void remove(list.map((e) => e.item.id), true)}
      />
      {menu && (
        <Menu x={menu.x} y={menu.y} entries={trashMenu()} label="Trash item" onClose={() => setMenu(null)}
          onPick={(id) => {
            const ids = selected.has(menu.entry.item.id) ? [...selected] : [menu.entry.item.id]
            if (id === 'restore') void restore(ids)
            else if (id === 'details') setDetails(menu.entry)
            else if (id === 'delete') void remove(ids, false)
          }} />
      )}
      {details && <TrashDetailsDialog entry={details} now={now} onClose={() => setDetails(null)} />}
      {confirm && (
        <DeleteForGoodDialog
          title={confirm.all ? 'Empty Trash?' : 'Delete permanently?'} count={target.length} bytes={totalBytes(target)}
          factor="password" busy={busy} error={error}
          onConfirm={(secret) => void remove(confirm.ids, confirm.all, secret)}
          onCancel={() => { setConfirm(null); setError(null) }} />
      )}
    </>
  )
}

function asksForFactor(result: CommandResult<unknown>): boolean {
  return !result.ok && !result.unsupported && FACTOR.test(codeOf(result.reason))
}
