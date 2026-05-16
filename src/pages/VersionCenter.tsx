import { useEffect, useState } from 'react'
import { command, commandUnavailableLabel, type SyncStatus, type VersionConflictEntry } from '../desktopApi'

export default function VersionCenter() {
  const [entries, setEntries] = useState<VersionConflictEntry[]>([])
  const [conflicts, setConflicts] = useState(0)
  const [loading, setLoading] = useState(true)
  const [notice, setNotice] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    const load = async () => {
      const status = await command<SyncStatus>('sync_status')
      if (!cancelled && status.ok) setConflicts(status.value.conflicts)

      const center = await command<VersionConflictEntry[]>('list_version_conflict_center')
      if (cancelled) return
      if (center.ok) setEntries(center.value)
      else setNotice(center.unsupported ? commandUnavailableLabel('list_version_conflict_center') : center.reason)
      setLoading(false)
    }
    void load()
    return () => {
      cancelled = true
    }
  }, [])

  const openConflict = async (entry: VersionConflictEntry) => {
    setNotice(null)
    const result = await command<void>('open_conflict_window', {
      fileId: entry.file_id,
      fileName: entry.file_name,
      isText: false,
    })
    if (!result.ok) {
      setNotice(result.unsupported ? commandUnavailableLabel('open_conflict_window') : result.reason)
    }
  }

  return (
    <section className="page">
      <div className="page-header">
        <div>
          <h1 className="page-title">Versions & conflicts</h1>
          <p className="page-copy">
            Desktop writes create server-side versions. Stale-base edits, upload failures, quota
            errors, and restore actions belong here instead of being hidden in Finder.
          </p>
        </div>
        <span className="status-pill">
          <span className={`dot ${conflicts > 0 ? 'error' : 'ok'}`} />
          {conflicts} active conflicts
        </span>
      </div>

      {notice && <div className="notice" style={{ marginBottom: 14 }}>{notice}</div>}

      <div className="grid three" style={{ marginBottom: 14 }}>
        <div className="metric">
          <div className="metric-label">Conflict policy</div>
          <div className="metric-value">Keep both</div>
          <div className="metric-detail">Never silently drop a version</div>
        </div>
        <div className="metric">
          <div className="metric-label">Restore model</div>
          <div className="metric-value">New version</div>
          <div className="metric-detail">Restore creates the next current version</div>
        </div>
        <div className="metric">
          <div className="metric-label">Notification entry</div>
          <div className="metric-value">Center</div>
          <div className="metric-detail">Status and notifications route here</div>
        </div>
      </div>

      {loading ? (
        <div className="empty-state">Loading version center…</div>
      ) : entries.length === 0 ? (
        <div className="empty-state">
          No version-center feed is available yet. The page is ready for
          `list_version_conflict_center`; current conflict count comes from `sync_status`.
        </div>
      ) : (
        <div className="panel">
          {entries.map((entry) => (
            <div className="row" key={entry.id}>
              <div>
                <div className="row-title">{entry.file_name}</div>
                <div className="row-detail">
                  {entry.kind} · {entry.status}
                  {entry.updated_at ? ` · ${entry.updated_at}` : ''}
                </div>
              </div>
              <button className="button" onClick={() => void openConflict(entry)}>
                Review
              </button>
            </div>
          ))}
        </div>
      )}
    </section>
  )
}
