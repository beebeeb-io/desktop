// Trash, live: DriveSource.trash / restoreFromTrash; permanent delete through drive_delete_permanently, whose step-up
// runs in Rust (crypto rule 11: it takes the password or code, never a token). Emptying goes item by item so the list
// shrinks as it goes (DW-0749b) and the window stays usable.
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useDriveSource, type TrashEntry } from '@beebeeb/shared/drive-ui'
import type { SectionProps } from '../types'
import { accountProfile } from '../../../desktopApi'
import { showToast } from '../../chrome/Toast'
import { Menu } from '../files/ui'
import { deletePermanently, failureText } from '../shared/api'
import { DeleteForGoodDialog, TrashDetailsDialog, TrashView, totalBytes, trashMenu } from './TrashView'
import { seed } from '../../seed'
import { publishCount } from '../../chrome/counts'
import { useCommand } from '../../shortcuts'

/** DW-0748 Restore: "Puts it back in Documents, with Undo". One item names where it went; Undo sends it back. */
export function restoredText(restored: TrashEntry[]): string {
  if (restored.length === 1) return `Restored to ${restored[0]!.originalPath || 'Files'}`
  return `${restored.length} items restored`
}

export default function Section({ nav }: SectionProps) {
  const source = useDriveSource()
  const [entries, setEntries] = useState<TrashEntry[]>([])
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [anchor, setAnchor] = useState<string | null>(null)
  const [menu, setMenu] = useState<{ entry: TrashEntry; x: number; y: number } | null>(null)
  const [confirm, setConfirm] = useState<{ ids: string[]; all: boolean } | null>(null)
  const [factor, setFactor] = useState<'code' | 'password'>('password')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [emptying, setEmptying] = useState<{ done: number; total: number } | null>(() => seed('trash.emptying', null))
  const [details, setDetails] = useState<TrashEntry | null>(null)
  const now = useMemo(() => new Date(), [])
  const removedRun = useRef<{ ids: Set<string>; done: number } | null>(null)

  const load = useCallback(() => source.trash().then((t) => { publishCount('trash', t.length); setEntries([...t].sort((a, b) => b.trashedAt.localeCompare(a.trashedAt))) }, () => setEntries([])), [source])
  useEffect(() => {
    void load()
    void accountProfile().then((r) => { if (r.ok) setFactor(r.value.totp_enabled ? 'code' : 'password') })
    return source.subscribe((e) => {
      const run = removedRun.current
      if (e.kind === 'removed' && run?.ids.has(e.id)) {
        run.done += 1
        setEntries((list) => list.filter((x) => x.item.id !== e.id))
        if (run.ids.size > 1) setEmptying({ done: run.done, total: run.ids.size })
      } else if (e.kind === 'changed' && !run) void load()
    })
  }, [source, load])

  const select = (id: string, o: { ctrl: boolean; shift: boolean }) => {
    if (o.shift && anchor) {
      const ids = entries.map((e) => e.item.id)
      const [a, b] = [ids.indexOf(anchor), ids.indexOf(id)].sort((x, y) => x - y)
      setSelected(new Set(ids.slice(a, b + 1)))
      return
    }
    setAnchor(id)
    setSelected((s) => { const n = new Set(o.ctrl ? s : []); if (o.ctrl && s.has(id)) n.delete(id); else n.add(id); return n })
  }

  const restore = async (ids: string[]) => {
    const restored = entries.filter((e) => ids.includes(e.item.id))
    try {
      await source.restoreFromTrash(ids)
      setEntries((e) => e.filter((x) => !ids.includes(x.item.id)))
      setSelected(new Set())
      // Undo moves what came back to Trash again (the same move as Files' Delete); the list follows its event.
      showToast({ kind: 'undo', text: restoredText(restored), onAction: () => void source.moveToTrash(ids).then(() => load(), () => showToast({ kind: 'failed', text: 'Couldn’t move it back to Trash. Try again.' })) })
    } catch {
      showToast({ kind: 'failed', text: 'Couldn’t restore. Try again.' })
    }
  }
  // Alt+Enter (the window's Details shortcut) on one selected item: DW-0411's dialog for it, over Trash.
  useCommand('details', useCallback(() => {
    if (selected.size !== 1) return
    const one = entries.find((e) => selected.has(e.item.id))
    if (one) setDetails(one)
  }, [selected, entries]))

  const run = async (ids: string[], secret: { code?: string; password?: string }) => {
    setBusy(true)
    setError(null)
    // One call: Rust proves the secret (its own step-up per delete, crypto rule 11), then deletes item by item and
    // emits `removed` for each, which the subscription below turns into the progress note (DW-0749b).
    removedRun.current = { ids: new Set(ids), done: 0 }
    if (ids.length > 1) setEmptying({ done: 0, total: ids.length })
    setConfirm(null)
    const r = await deletePermanently(ids, secret)
    setBusy(false)
    setEmptying(null)
    removedRun.current = null
    setSelected(new Set())
    if (!r.ok) {
      void load()
      if (/^(invalid_credentials|wrong_code)/.test(r.reason)) { setConfirm({ ids, all: ids.length === entries.length }); setError(failureText(r)) }
      else showToast({ kind: 'failed', text: failureText(r) })
      return
    }
    setEntries((e) => e.filter((x) => !ids.includes(x.item.id)))
    showToast({ kind: 'done', text: ids.length === 1 ? 'Deleted for good' : 'Trash emptied' })
  }

  const target = confirm ? entries.filter((e) => confirm.ids.includes(e.item.id)) : []
  return (
    <>
      <TrashView
        entries={entries} now={now} selected={selected} menuOn={menu?.entry.item.id} emptying={emptying}
        onSelect={select}
        onContext={(entry, x, y) => { if (!selected.has(entry.item.id)) setSelected(new Set([entry.item.id])); setMenu({ entry, x, y }) }}
        onDelete={() => setConfirm({ ids: [...selected], all: false })}
        onPreview={(e) => nav.go({ section: 'open', id: e.item.id, sub: 'preview' })}
        onEmpty={() => setConfirm({ ids: entries.map((e) => e.item.id), all: true })}
      />
      {menu && (
        <Menu x={menu.x} y={menu.y} entries={trashMenu()} label="Trash item" onClose={() => setMenu(null)}
          onPick={(id) => {
            const ids = selected.has(menu.entry.item.id) ? [...selected] : [menu.entry.item.id]
            if (id === 'restore') void restore(ids)
            else if (id === 'details') setDetails(menu.entry)
            else if (id === 'delete') setConfirm({ ids, all: false })
          }} />
      )}
      {details && <TrashDetailsDialog entry={details} now={now} onClose={() => setDetails(null)} />}
      {confirm && (
        <DeleteForGoodDialog
          title={confirm.all ? 'Empty Trash?' : 'Delete permanently?'} count={target.length} bytes={totalBytes(target)} factor={factor} busy={busy} error={error}
          onConfirm={(secret) => void run(confirm.ids, secret)} onCancel={() => { setConfirm(null); setError(null) }} />
      )}
    </>
  )
}
