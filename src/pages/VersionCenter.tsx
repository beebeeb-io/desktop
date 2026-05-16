import { useEffect, useMemo, useState } from 'react'
import {
  command,
  commandUnavailableLabel,
  formatBytes,
  type FileVersionEntry,
  type SyncStatus,
  type VersionConflictEntry,
} from '../desktopApi'

type RawObject = Record<string, unknown>

function asObject(value: unknown): RawObject | null {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as RawObject) : null
}

function asString(value: unknown): string | null {
  return typeof value === 'string' && value.length > 0 ? value : null
}

function asNumber(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

function normalizeVersion(value: unknown): FileVersionEntry | null {
  const raw = asObject(value)
  if (!raw) return null
  const id =
    asString(raw.id) ??
    asString(raw.version_id) ??
    asString(raw.object_version_id) ??
    asString(raw.objectVersionId)
  if (!id) return null

  return {
    id,
    version: asNumber(raw.version) ?? asNumber(raw.version_number) ?? asNumber(raw.versionNumber),
    object_version_id:
      asString(raw.object_version_id) ?? asString(raw.objectVersionId) ?? asString(raw.version_id),
    created_at: asString(raw.created_at) ?? asString(raw.createdAt) ?? asNumber(raw.created_at),
    size_bytes: asNumber(raw.size_bytes) ?? asNumber(raw.sizeBytes),
    is_current: raw.is_current === true || raw.current === true,
    created_by: asString(raw.created_by) ?? asString(raw.createdBy),
  }
}

function normalizeVersions(value: unknown): FileVersionEntry[] {
  const raw = asObject(value)
  const list = Array.isArray(value)
    ? value
    : Array.isArray(raw?.versions)
      ? raw.versions
      : Array.isArray(raw?.file_versions)
        ? raw.file_versions
        : []
  return list
    .map(normalizeVersion)
    .filter((version): version is FileVersionEntry => version !== null)
}

function formatTimestamp(value: string | number | null | undefined): string {
  if (typeof value === 'number') {
    const ms = value > 10_000_000_000 ? value : value * 1000
    return new Date(ms).toLocaleString()
  }
  if (typeof value === 'string') {
    const parsed = Date.parse(value)
    return Number.isNaN(parsed) ? value : new Date(parsed).toLocaleString()
  }
  return 'Time unknown'
}

function kindLabel(kind: VersionConflictEntry['kind']): string {
  switch (kind) {
    case 'quota_failure':
      return 'Quota'
    case 'permission_failure':
      return 'Permission'
    case 'stale_base':
      return 'Stale base'
    case 'failed_upload':
      return 'Upload'
    case 'metadata':
      return 'Metadata'
    case 'delete':
      return 'Delete'
    case 'restore':
      return 'Restore'
    default:
      return 'Conflict'
  }
}

export default function VersionCenter({ refreshSignal = 0 }: { refreshSignal?: number }) {
  const [entries, setEntries] = useState<VersionConflictEntry[]>([])
  const [selected, setSelected] = useState<VersionConflictEntry | null>(null)
  const [versions, setVersions] = useState<FileVersionEntry[]>([])
  const [conflicts, setConflicts] = useState(0)
  const [loading, setLoading] = useState(true)
  const [refreshing, setRefreshing] = useState(false)
  const [notice, setNotice] = useState<string | null>(null)
  const [versionNotice, setVersionNotice] = useState<string | null>(null)
  const [restoring, setRestoring] = useState<string | null>(null)

  const selectedEntry = selected ?? entries[0] ?? null

  useEffect(() => {
    let cancelled = false
    const load = async () => {
      setRefreshing(true)
      const status = await command<SyncStatus>('sync_status')
      if (!cancelled && status.ok) setConflicts(status.value.conflicts)

      const center = await command<VersionConflictEntry[]>('list_version_conflict_center')
      if (cancelled) return
      if (center.ok) {
        setEntries(center.value)
        setSelected((current) => {
          if (current && center.value.some((entry) => entry.id === current.id)) return current
          return center.value[0] ?? null
        })
        setNotice(null)
      } else {
        setNotice(center.unsupported ? commandUnavailableLabel('list_version_conflict_center') : center.reason)
      }
      setLoading(false)
      setRefreshing(false)
    }
    void load()
    return () => {
      cancelled = true
    }
  }, [refreshSignal])

  useEffect(() => {
    let cancelled = false
    const loadVersions = async () => {
      if (!selectedEntry) {
        setVersions([])
        return
      }
      setVersionNotice(null)
      const result = await command<unknown>('list_file_versions', { fileId: selectedEntry.file_id })
      if (cancelled) return
      if (result.ok) {
        setVersions(normalizeVersions(result.value))
      } else {
        setVersions([])
        setVersionNotice(result.unsupported ? commandUnavailableLabel('list_file_versions') : result.reason)
      }
    }
    void loadVersions()
    return () => {
      cancelled = true
    }
  }, [selectedEntry])

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

  const restoreVersion = async (version: FileVersionEntry) => {
    if (!selectedEntry) return
    const versionId = version.object_version_id ?? version.id
    setRestoring(versionId)
    setVersionNotice(null)
    const result = await command<unknown>('restore_file_version', {
      fileId: selectedEntry.file_id,
      versionId,
    })
    setRestoring(null)
    if (result.ok) {
      setVersionNotice('Restore created a new current version. The daemon will refresh metadata on the next sync tick.')
      return
    }
    setVersionNotice(result.unsupported ? commandUnavailableLabel('restore_file_version') : result.reason)
  }

  const metrics = useMemo(() => {
    const failed = entries.filter((entry) => entry.kind !== 'conflict').length
    const stale = entries.filter((entry) => entry.kind === 'stale_base').length
    return { failed, stale }
  }, [entries])

  return (
    <section className="page">
      <div className="page-header">
        <div>
          <h1 className="page-title">Versions & conflicts</h1>
          <p className="page-copy">
            Desktop writes create server-side versions. Stale-base edits, upload failures, quota
            errors, and restore actions stay visible here until the daemon can finish them.
          </p>
        </div>
        <span className="status-pill">
          <span className={`dot ${conflicts > 0 || entries.length > 0 ? 'error' : 'ok'}`} />
          {refreshing ? 'Refreshing' : `${entries.length} review item${entries.length === 1 ? '' : 's'}`}
        </span>
      </div>

      {notice && <div className="notice" style={{ marginBottom: 14 }}>{notice}</div>}

      <div className="grid three" style={{ marginBottom: 14 }}>
        <div className="metric">
          <div className="metric-label">Active conflicts</div>
          <div className="metric-value">{conflicts}</div>
          <div className="metric-detail">Rows marked conflict by the daemon</div>
        </div>
        <div className="metric">
          <div className="metric-label">Upload reviews</div>
          <div className="metric-value">{metrics.failed}</div>
          <div className="metric-detail">Failed or queued Finder writes</div>
        </div>
        <div className="metric">
          <div className="metric-label">Stale-base edits</div>
          <div className="metric-value">{metrics.stale}</div>
          <div className="metric-detail">Local bytes preserved for review</div>
        </div>
      </div>

      {loading ? (
        <div className="empty-state">Loading version center...</div>
      ) : entries.length === 0 ? (
        <div className="empty-state">
          No conflicts, failed writes, or restore reviews are recorded in the local state DB.
        </div>
      ) : (
        <div className="grid two">
          <div className="panel">
            <h2 className="section-title">Review feed</h2>
            {entries.map((entry) => (
              <div className="row" key={entry.id}>
                <button
                  className={`link-row ${selectedEntry?.id === entry.id ? 'active' : ''}`}
                  onClick={() => setSelected(entry)}
                >
                  <span>
                    <span className="row-title">{entry.file_name}</span>
                    <span className="row-detail">
                      {kindLabel(entry.kind)} · {entry.status} · {formatTimestamp(entry.updated_at)}
                    </span>
                    <span className="row-detail">{entry.detail}</span>
                  </span>
                </button>
                {entry.action === 'open_conflict' && (
                  <button className="button" onClick={() => void openConflict(entry)}>
                    Review
                  </button>
                )}
              </div>
            ))}
          </div>

          <div className="panel">
            <h2 className="section-title">Version history</h2>
            {selectedEntry ? (
              <>
                <div className="row" style={{ paddingTop: 0 }}>
                  <div>
                    <div className="row-title">{selectedEntry.file_name}</div>
                    <div className="row-detail mono">{selectedEntry.file_id}</div>
                  </div>
                </div>
                {versionNotice && <div className="notice" style={{ margin: '0 0 12px' }}>{versionNotice}</div>}
                {versions.length === 0 ? (
                  <div className="empty-state">
                    Version history is unavailable for this file until the server list-versions route responds.
                  </div>
                ) : (
                  versions.map((version) => {
                    const versionId = version.object_version_id ?? version.id
                    return (
                      <div className="row" key={version.id}>
                        <div>
                          <div className="row-title">
                            Version {version.version ?? version.id}
                            {version.is_current ? ' · current' : ''}
                          </div>
                          <div className="row-detail">
                            {formatTimestamp(version.created_at)}
                            {typeof version.size_bytes === 'number' ? ` · ${formatBytes(version.size_bytes)}` : ''}
                          </div>
                          <div className="row-detail mono">{versionId}</div>
                        </div>
                        <button
                          className="button"
                          disabled={version.is_current || restoring === versionId}
                          onClick={() => void restoreVersion(version)}
                        >
                          {restoring === versionId ? 'Restoring' : 'Restore'}
                        </button>
                      </div>
                    )
                  })
                )}
              </>
            ) : (
              <div className="empty-state">Select a review item to inspect historical versions.</div>
            )}
          </div>
        </div>
      )}
    </section>
  )
}
