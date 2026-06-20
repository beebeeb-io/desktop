import { useEffect, useMemo, useState } from 'react'
import { command, commandUnavailableLabel, type VaultItem } from '../desktopApi'

export default function SelectiveSync() {
  const [items, setItems] = useState<VaultItem[]>([])
  const [loading, setLoading] = useState(true)
  const [savingId, setSavingId] = useState<string | null>(null)
  const [notice, setNotice] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    const load = async () => {
      const tree = await command<VaultItem[]>('list_remote_tree')
      if (cancelled) return
      if (tree.ok) {
        setItems(tree.value)
        setLoading(false)
        return
      }

      const folders = await command<VaultItem[]>('list_vault_folders')
      if (cancelled) return
      if (folders.ok) setItems(folders.value)
      setNotice(tree.unsupported ? commandUnavailableLabel('list_remote_tree') : tree.reason)
      setLoading(false)
    }
    void load()
    return () => {
      cancelled = true
    }
  }, [])

  const folderItems = useMemo(() => flatten(items).filter((item) => item.is_folder), [items])
  const pinnedCount = folderItems.filter((item) => item.pinned).length

  const toggleFolder = async (folder: VaultItem) => {
    setSavingId(folder.id)
    setNotice(null)
    const nextPinned = !folder.pinned
    const result = await command<void>('set_recursive_pin', {
      itemId: folder.id,
      pinned: nextPinned,
    })
    setSavingId(null)
    if (!result.ok) {
      setNotice(result.unsupported ? commandUnavailableLabel('set_recursive_pin') : result.reason)
      return
    }
    setItems((current) => updateItem(current, folder.id, { pinned: nextPinned }))
  }

  return (
    <section className="page">
      <div className="page-header">
        <div>
          <h1 className="page-title">Selective sync</h1>
          <p className="page-copy">
            The drive is online-only by default. Pin a folder to recursively keep it hydrated on
            this Mac; unpinning returns it to remote-first behavior after pending uploads settle.
          </p>
        </div>
        <span className="status-pill">
          <span className={`dot ${pinnedCount > 0 ? 'ok' : ''}`} />
          {pinnedCount} pinned
        </span>
      </div>

      {notice && <div className="notice" style={{ marginBottom: 14 }}>{notice}</div>}

      <div className="panel" style={{ marginBottom: 14 }}>
        <div className="grid three">
          <div>
            <div className="metric-label">Default</div>
            <div className="metric-detail">No folders pinned</div>
          </div>
          <div>
            <div className="metric-label">Scope</div>
            <div className="metric-detail">Recursive folder toggles</div>
          </div>
          <div>
            <div className="metric-label">Persisted by</div>
            <div className="metric-detail">Local daemon state</div>
          </div>
        </div>
      </div>

      {loading ? (
        <div className="empty-state">Loading remote tree…</div>
      ) : folderItems.length === 0 ? (
        <div className="empty-state">
          No remote tree is available. The UI is ready for `list_remote_tree` and
          `set_recursive_pin`, but this build has no folder data to pin yet.
        </div>
      ) : (
        <div className="tree">
          {folderItems.map((item) => (
            <div className="tree-row" key={item.id}>
              <div>
                <div className="row-title">{item.path ?? item.name}</div>
                <div className="row-detail">
                  {item.pinned
                    ? 'Kept local recursively'
                    : item.inheritedPinned
                      ? 'Pinned by parent'
                      : 'Online-only until opened'}
                </div>
              </div>
              <button
                className={`button ${item.pinned ? 'primary' : ''}`}
                disabled={savingId === item.id}
                onClick={() => void toggleFolder(item)}
              >
                {savingId === item.id ? 'Saving…' : item.pinned ? 'Pinned' : 'Make offline'}
              </button>
            </div>
          ))}
        </div>
      )}
    </section>
  )
}

function flatten(items: VaultItem[], prefix = ''): VaultItem[] {
  return items.flatMap((item) => {
    const path = item.path ?? (prefix ? `${prefix}/${item.name}` : item.name)
    return [{ ...item, path }, ...flatten(item.children ?? [], path)]
  })
}

function updateItem(items: VaultItem[], id: string, patch: Partial<VaultItem>): VaultItem[] {
  return items.map((item) => {
    if (item.id === id) return { ...item, ...patch }
    return { ...item, children: item.children ? updateItem(item.children, id, patch) : [] }
  })
}
