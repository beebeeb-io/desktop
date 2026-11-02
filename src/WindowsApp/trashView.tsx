/**
 * Trash: what the engine wants to delete, with restore and permanent delete.
 */
import { useEffect, useState } from 'react'
import { commandUnavailableLabel, formatBytes, desktopConfirmAction, desktopTrashDeletePermanently, desktopTrashList, desktopTrashRestore, type DesktopTrashItem } from '../desktopApi'
import { T, NavIcon, Skeleton, PageHeader, Card, useToast } from '../windows/ui'
import { FILES_DANGER } from './filesView'

// ── Root: TrashView ───────────────────────────────────────────────────────────

export function trashTime(value: string): string {
  const numeric = Number(value)
  const ms = Number.isFinite(numeric) ? (numeric > 1_000_000_000_000 ? numeric : numeric * 1000) : Date.parse(value)
  if (!Number.isFinite(ms)) return '—'
  const secs = Math.max(0, Math.floor((Date.now() - ms) / 1000))
  if (secs < 45) return 'just now'
  const mins = Math.floor(secs / 60)
  if (mins < 60) return `${mins}m ago`
  const hours = Math.floor(mins / 60)
  if (hours < 24) return `${hours}h ago`
  const days = Math.floor(hours / 24)
  if (days < 30) return `${days}d ago`
  return new Date(ms).toLocaleDateString('en-US', { month: 'short', day: 'numeric' })
}

export function GhostButton({ children, onClick, disabled, danger = false }: { children: React.ReactNode; onClick?: () => void; disabled?: boolean; danger?: boolean }) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 6,
        minHeight: 30,
        padding: '6px 10px',
        borderRadius: 6,
        border: `1px solid ${danger ? 'oklch(0.74 0.09 25)' : T.line2}`,
        background: disabled ? T.paper2 : T.paper,
        color: danger ? FILES_DANGER : T.ink2,
        fontSize: 11.5,
        fontFamily: T.fontSans,
        fontWeight: 600,
        cursor: disabled ? 'not-allowed' : 'pointer',
        whiteSpace: 'nowrap' as const,
        opacity: disabled ? 0.55 : 1,
      }}
    >
      {children}
    </button>
  )
}

export function TrashView() {
  const [items, setItems] = useState<DesktopTrashItem[]>([])
  const [loading, setLoading] = useState(true)
  // `error` is the blocking list-load state (renders the full "Could not load
  // Trash" card with Retry) — kept inline. A failed restore now toasts instead
  // of setting `error`, so a transient action failure no longer nukes the list.
  const [error, setError] = useState<string | null>(null)
  const { showToast } = useToast()
  const [busyId, setBusyId] = useState<string | null>(null)
  const [pendingDelete, setPendingDelete] = useState<DesktopTrashItem | null>(null)
  const [password, setPassword] = useState('')
  const [deleteError, setDeleteError] = useState<string | null>(null)

  const load = async () => {
    setLoading(true)
    setError(null)
    const result = await desktopTrashList()
    if (result.ok) {
      setItems(result.value)
    } else {
      setError(result.unsupported ? commandUnavailableLabel('desktop_trash_list') : result.reason)
      setItems([])
    }
    setLoading(false)
  }

  useEffect(() => {
    void load()
  }, [])

  const restore = async (item: DesktopTrashItem) => {
    setBusyId(item.id)
    const result = await desktopTrashRestore(item.id, item.name)
    setBusyId(null)
    if (!result.ok) {
      showToast({
        variant: 'error',
        title: 'Restore failed',
        message: result.unsupported ? commandUnavailableLabel('desktop_trash_restore') : result.reason,
      })
      return
    }
    setItems((prev) => prev.filter((row) => row.id !== item.id))
  }

  const deletePermanently = async () => {
    if (!pendingDelete) return
    setBusyId(pendingDelete.id)
    setDeleteError(null)
    const confirmed = await desktopConfirmAction(password)
    if (!confirmed.ok) {
      setBusyId(null)
      setDeleteError(confirmed.unsupported ? commandUnavailableLabel('desktop_confirm_action') : confirmed.reason)
      return
    }
    const deleted = await desktopTrashDeletePermanently(pendingDelete.id, confirmed.value.confirmation_token)
    setBusyId(null)
    if (!deleted.ok) {
      setDeleteError(deleted.unsupported ? commandUnavailableLabel('desktop_trash_delete_permanently') : deleted.reason)
      return
    }
    setItems((prev) => prev.filter((row) => row.id !== pendingDelete.id))
    setPendingDelete(null)
    setPassword('')
  }

  return (
    <div style={{ overflow: 'auto', padding: '28px 36px', flex: 1 }}>
      <PageHeader
        title="Trash"
        subtitle="Deleted files in your vault. Restore puts them back through sync; permanent delete removes them from Trash."
        aside={<GhostButton onClick={() => void load()} disabled={loading}>Refresh</GhostButton>}
      />

      {loading ? (
        <Card style={{ padding: 0 }}>
          {[0, 1, 2, 3].map((i) => (
            <div key={i} style={{ display: 'grid', gridTemplateColumns: '32px 1fr 110px 170px', gap: 14, alignItems: 'center', padding: '14px 18px', borderBottom: i === 3 ? 'none' : `1px solid ${T.line}` }}>
              <Skeleton width={32} height={32} radius={7} />
              <div style={{ display: 'flex', flexDirection: 'column', gap: 7 }}>
                <Skeleton width={`${48 + i * 8}%`} height={12} />
                <Skeleton width="32%" height={10} />
              </div>
              <Skeleton width={72} height={11} />
              <Skeleton width={150} height={30} radius={6} />
            </div>
          ))}
        </Card>
      ) : error ? (
        <Card style={{ padding: 18 }}>
          <div style={{ fontSize: 13, fontWeight: 600, color: T.ink, marginBottom: 5 }}>Could not load Trash</div>
          <div style={{ fontSize: 12, color: FILES_DANGER, lineHeight: 1.5, marginBottom: 14 }}>{error}</div>
          <GhostButton onClick={() => void load()}>Try again</GhostButton>
        </Card>
      ) : items.length === 0 ? (
        <Card style={{ padding: 28, textAlign: 'center' as const }}>
          <div style={{ width: 42, height: 42, margin: '0 auto 12px', borderRadius: 8, border: `1px solid ${T.line}`, background: T.paper2, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
            <NavIcon name="trash" size={18} color={T.ink3} />
          </div>
          <div style={{ fontSize: 14, fontWeight: 650, color: T.ink, marginBottom: 5 }}>Trash is empty</div>
          <div style={{ fontSize: 12, color: T.ink3 }}>Deleted files will appear here.</div>
        </Card>
      ) : (
        <Card style={{ padding: 0 }}>
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 110px 105px 178px', gap: 14, alignItems: 'center', padding: '12px 18px', borderBottom: `1px solid ${T.line}`, color: T.ink3, fontSize: 10, fontFamily: T.fontMono, textTransform: 'uppercase' as const }}>
            <span>Name</span>
            <span>Size</span>
            <span>Deleted</span>
            <span style={{ textAlign: 'right' as const }}>Actions</span>
          </div>
          {items.map((item, i) => {
            const busy = busyId === item.id
            return (
              <div
                key={item.id}
                style={{
                  display: 'grid',
                  gridTemplateColumns: '1fr 110px 105px 178px',
                  gap: 14,
                  alignItems: 'center',
                  padding: '12px 18px',
                  borderBottom: i === items.length - 1 ? 'none' : `1px solid ${T.line}`,
                }}
              >
                <div style={{ display: 'flex', alignItems: 'center', gap: 10, minWidth: 0 }}>
                  <div style={{ width: 30, height: 30, borderRadius: 7, background: T.paper2, border: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
                    <NavIcon name={item.is_folder ? 'folder' : 'trash'} size={14} color={T.ink3} />
                  </div>
                  <div style={{ minWidth: 0 }}>
                    <div title={item.name} style={{ fontSize: 12.5, fontWeight: 600, color: T.ink, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' as const }}>{item.name}</div>
                    <div style={{ fontSize: 10.5, fontFamily: T.fontMono, color: T.ink4 }}>{item.is_folder ? 'Folder' : 'File'}</div>
                  </div>
                </div>
                <span style={{ fontSize: 11.5, fontFamily: T.fontMono, color: T.ink3 }}>{item.is_folder ? '—' : formatBytes(item.size_bytes)}</span>
                <span style={{ fontSize: 11.5, fontFamily: T.fontMono, color: T.ink3 }}>{trashTime(item.updated_at)}</span>
                <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8 }}>
                  <GhostButton onClick={() => void restore(item)} disabled={busy}>Restore</GhostButton>
                  <GhostButton onClick={() => { setPendingDelete(item); setDeleteError(null); setPassword('') }} disabled={busy} danger>Delete</GhostButton>
                </div>
              </div>
            )
          })}
        </Card>
      )}

      {pendingDelete && (
        <div style={{ position: 'fixed', inset: 0, background: T.scrimSoft, display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex: 20 }}>
          <Card style={{ width: 420, padding: 20, boxShadow: T.dialogShadow }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 10 }}>
              <NavIcon name="trash" size={16} color={FILES_DANGER} />
              <div style={{ fontSize: 14, fontWeight: 700, color: T.ink }}>Delete permanently</div>
            </div>
            <div style={{ fontSize: 12, color: T.ink2, lineHeight: 1.5, marginBottom: 14 }}>
              {pendingDelete.name} will be removed from Trash. This cannot be undone.
            </div>
            <input
              type="password"
              value={password}
              onChange={(event) => setPassword(event.currentTarget.value)}
              placeholder="Password"
              style={{ width: '100%', boxSizing: 'border-box', border: `1px solid ${T.line2}`, borderRadius: 6, padding: '9px 10px', fontFamily: T.fontSans, fontSize: 12.5, color: T.ink, background: T.paper, marginBottom: 10 }}
            />
            {deleteError && <div style={{ fontSize: 11.5, color: FILES_DANGER, lineHeight: 1.4, marginBottom: 10 }}>{deleteError}</div>}
            <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8 }}>
              <GhostButton onClick={() => { setPendingDelete(null); setPassword(''); setDeleteError(null) }} disabled={busyId === pendingDelete.id}>Cancel</GhostButton>
              <GhostButton onClick={() => void deletePermanently()} disabled={busyId === pendingDelete.id || password.length === 0} danger>
                {busyId === pendingDelete.id ? 'Deleting…' : 'Delete permanently'}
              </GhostButton>
            </div>
          </Card>
        </div>
      )}
    </div>
  )
}
