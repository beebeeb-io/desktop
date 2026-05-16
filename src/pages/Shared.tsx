import { useEffect, useState } from 'react'
import { command, commandUnavailableLabel, type SharedRoot } from '../desktopApi'

export default function Shared() {
  const [roots, setRoots] = useState<SharedRoot[]>([])
  const [loading, setLoading] = useState(true)
  const [notice, setNotice] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    command<SharedRoot[]>('list_shared_roots').then((result) => {
      if (cancelled) return
      if (result.ok) setRoots(result.value)
      else setNotice(result.unsupported ? commandUnavailableLabel('list_shared_roots') : result.reason)
      setLoading(false)
    })
    return () => {
      cancelled = true
    }
  }, [])

  const openRoot = async (root: SharedRoot) => {
    setNotice(null)
    const result = await command<void>('open_in_finder', {
      itemId: root.id,
      path: root.finder_path,
    })
    if (!result.ok) {
      setNotice(result.unsupported ? commandUnavailableLabel('open_in_finder') : result.reason)
    }
  }

  return (
    <section className="page">
      <div className="page-header">
        <div>
          <h1 className="page-title">Shared roots</h1>
          <p className="page-copy">
            Shared content is part of the Finder namespace, with permissions enforced by the daemon
            before any hydrate or write operation.
          </p>
        </div>
        <span className="status-pill">
          <span className={`dot ${roots.length > 0 ? 'ok' : ''}`} />
          {roots.length} roots
        </span>
      </div>

      {notice && <div className="notice" style={{ marginBottom: 14 }}>{notice}</div>}

      {loading ? (
        <div className="empty-state">Loading shared roots…</div>
      ) : roots.length === 0 ? (
        <div className="empty-state">
          No shared roots are available from the desktop daemon yet. This page is wired for
          `list_shared_roots` and `open_in_finder` so accepted shares can appear here without
          changing the UI contract.
        </div>
      ) : (
        <div className="panel">
          {roots.map((root) => (
            <div className="row" key={root.id}>
              <div>
                <div className="row-title">{root.name}</div>
                <div className="row-detail">
                  {root.owner_email ?? 'Unknown owner'} · {root.permission}
                </div>
              </div>
              <button className="button" onClick={() => void openRoot(root)}>
                Open in Finder
              </button>
            </div>
          ))}
        </div>
      )}
    </section>
  )
}
