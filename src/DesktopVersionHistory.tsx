import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from 'react'
import {
  commandUnavailableLabel,
  desktopListFileVersions,
  desktopRestoreFileVersion,
  desktopSearchFiles,
  formatBytes,
  restorableFileVersions,
  restoreVersionId,
  type DesktopSearchResponse,
  type DesktopSearchResult,
  type FileVersionEntry,
  type FileVersionListResponse,
} from './desktopApi'
import { useToast } from './windows/ui'

const SEARCH_LIMIT = 12
const SEARCH_DEBOUNCE_MS = 120
const EMPTY_RESULTS: DesktopSearchResult[] = []

export function HistoryGlyph({ size = 14 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <circle cx="8" cy="8" r="5.5" />
      <path d="M8 4.8 L8 8 L10.2 9.3" />
      <path d="M4.2 3.9 L2.8 3.9 L2.8 2.5" />
      <path d="M3 4 C4.1 2.8 5.9 2 8 2" />
    </svg>
  )
}

export function DesktopVersionHistoryTrigger({
  onOpen,
  disabled = false,
}: {
  onOpen: () => void
  disabled?: boolean
}) {
  return (
    <button
      type="button"
      className="quick-search-trigger"
      onClick={onOpen}
      disabled={disabled}
      aria-label="Open version history"
      title="Open version history"
    >
      <span className="quick-search-trigger-icon">
        <HistoryGlyph size={13} />
      </span>
      <span className="quick-search-trigger-label">Version history</span>
    </button>
  )
}

export default function DesktopVersionHistory({
  open,
  onOpen,
  onClose,
}: {
  open: boolean
  onOpen: () => void
  onClose: () => void
}) {
  const [query, setQuery] = useState('')
  const [loading, setLoading] = useState(false)
  const [response, setResponse] = useState<DesktopSearchResponse | null>(null)
  const [notice, setNotice] = useState<string | null>(null)
  const [activeIndex, setActiveIndex] = useState(0)
  const [selectedFile, setSelectedFile] = useState<DesktopSearchResult | null>(null)
  const [versionList, setVersionList] = useState<FileVersionListResponse | null>(null)
  const [versionLoading, setVersionLoading] = useState(false)
  const [versionNotice, setVersionNotice] = useState<string | null>(null)
  const [selectedVersionIndex, setSelectedVersionIndex] = useState(0)
  const [restoringId, setRestoringId] = useState<string | null>(null)
  const inputRef = useRef<HTMLInputElement | null>(null)
  const requestId = useRef(0)
  const versionRequestId = useRef(0)
  const { showToast } = useToast()

  const trimmedQuery = query.trim()
  const results = response?.results ?? EMPTY_RESULTS
  const indexedFileCount = response?.indexed_file_count ?? 0
  const indexSyncing = response?.index_state === 'syncing'
  const earlierVersions = useMemo(() => restorableFileVersions(versionList?.versions ?? []), [versionList])
  const selectedVersion = earlierVersions[selectedVersionIndex] ?? earlierVersions[0] ?? null
  const currentVersion = versionList?.versions.find((version) => version.is_current) ?? null

  const resetDialog = useCallback(() => {
    setQuery('')
    setLoading(false)
    setResponse(null)
    setNotice(null)
    setActiveIndex(0)
    setSelectedFile(null)
    setVersionList(null)
    setVersionLoading(false)
    setVersionNotice(null)
    setSelectedVersionIndex(0)
    setRestoringId(null)
    versionRequestId.current += 1
  }, [])

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.shiftKey && event.key.toLowerCase() === 'h') {
        event.preventDefault()
        onOpen()
        return
      }
      if (open && event.key === 'Escape') {
        event.preventDefault()
        onClose()
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [onClose, onOpen, open])

  useEffect(() => {
    if (!open) return
    resetDialog()
    const focusTimer = window.setTimeout(() => inputRef.current?.focus(), 0)
    return () => window.clearTimeout(focusTimer)
  }, [open, resetDialog])

  useEffect(() => {
    if (!open || selectedFile != null || trimmedQuery.length === 0) {
      setLoading(false)
      if (selectedFile == null) {
        setResponse(null)
        setNotice(null)
      }
      return
    }

    const currentRequest = requestId.current + 1
    requestId.current = currentRequest
    let cancelled = false
    setLoading(true)
    const timer = window.setTimeout(() => {
      void desktopSearchFiles(trimmedQuery, SEARCH_LIMIT).then((result) => {
        if (cancelled || requestId.current !== currentRequest) return
        if (result.ok) {
          setResponse(result.value)
          setNotice(null)
          setActiveIndex(0)
        } else {
          setResponse(null)
          setNotice(result.unsupported ? commandUnavailableLabel('desktop_search_files') : result.reason)
        }
        setLoading(false)
      })
    }, SEARCH_DEBOUNCE_MS)

    return () => {
      cancelled = true
      window.clearTimeout(timer)
    }
  }, [open, selectedFile, trimmedQuery])

  useEffect(() => {
    setSelectedVersionIndex((index) => {
      if (earlierVersions.length === 0) return 0
      return Math.min(index, earlierVersions.length - 1)
    })
  }, [earlierVersions.length])

  const loadVersions = useCallback(async (file: DesktopSearchResult) => {
    const currentRequest = versionRequestId.current + 1
    versionRequestId.current = currentRequest
    setSelectedFile(file)
    setVersionList(null)
    setVersionNotice(null)
    setVersionLoading(true)
    setSelectedVersionIndex(0)

    const result = await desktopListFileVersions(file.file_id)
    if (versionRequestId.current !== currentRequest) return
    if (result.ok) {
      setVersionList(result.value)
    } else {
      setVersionList(null)
      setVersionNotice(result.unsupported ? commandUnavailableLabel('list_file_versions') : result.reason)
    }
    setVersionLoading(false)
  }, [])

  const restoreVersion = useCallback(async (version: FileVersionEntry) => {
    if (!selectedFile) return
    const versionId = restoreVersionId(version)
    setRestoringId(versionId)
    const result = await desktopRestoreFileVersion(selectedFile.file_id, versionId)
    setRestoringId(null)

    if (result.ok) {
      showToast({
        title: 'Restore queued',
        message: `${selectedFile.name} ${versionLabel(version)} will be restored by the sync engine.`,
        variant: 'success',
      })
      return
    }

    showToast({
      title: 'Restore failed',
      message: result.unsupported ? commandUnavailableLabel('restore_file_version') : result.reason,
      variant: 'error',
      durationMs: 9000,
    })
  }, [selectedFile, showToast])

  const backToSearch = useCallback(() => {
    setSelectedFile(null)
    setVersionList(null)
    setVersionLoading(false)
    setVersionNotice(null)
    setSelectedVersionIndex(0)
    setRestoringId(null)
    versionRequestId.current += 1
    window.setTimeout(() => inputRef.current?.focus(), 0)
  }, [])

  const onInputKeyDown = (event: ReactKeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'ArrowDown' && results.length > 0) {
      event.preventDefault()
      setActiveIndex((index) => Math.min(index + 1, results.length - 1))
      return
    }
    if (event.key === 'ArrowUp' && results.length > 0) {
      event.preventDefault()
      setActiveIndex((index) => Math.max(index - 1, 0))
      return
    }
    if (event.key === 'Enter' && results[activeIndex]) {
      event.preventDefault()
      void loadVersions(results[activeIndex])
    }
  }

  const searchBody = () => {
    if (trimmedQuery.length === 0) {
      return <div className="quick-search-empty">File name</div>
    }
    if (loading) {
      return <div className="quick-search-empty">Searching...</div>
    }
    if (notice != null) {
      return <div className="quick-search-notice">{notice}</div>
    }
    if (indexSyncing) {
      return <div className="quick-search-empty">Search index is syncing.</div>
    }
    if (results.length === 0) {
      return <div className="quick-search-empty">{indexedFileCount === 0 ? 'No indexed files.' : 'No matching files.'}</div>
    }
    return (
      <div className="quick-search-results" role="listbox" aria-label="Files">
        {results.map((result, index) => (
          <button
            key={result.file_id}
            type="button"
            className={`quick-search-result ${activeIndex === index ? 'active' : ''}`}
            onMouseEnter={() => setActiveIndex(index)}
            onClick={() => void loadVersions(result)}
            role="option"
            aria-selected={activeIndex === index}
          >
            <span className="quick-search-result-icon">
              <HistoryGlyph size={12} />
            </span>
            <span className="quick-search-result-main">
              <span className="quick-search-result-name" title={result.name}>{result.name}</span>
              <span className="quick-search-result-path" title={result.rel_path}>{result.rel_path}</span>
            </span>
            <span className="quick-search-result-meta">
              <span>{statusLabel(result.status)}</span>
              <span>{formatBytes(result.size_bytes)}</span>
              <span>{formatModifiedAt(result.modified_at)}</span>
            </span>
          </button>
        ))}
      </div>
    )
  }

  const versionBody = () => {
    if (!selectedFile) return searchBody()
    if (versionLoading) {
      return <div className="quick-search-empty">Loading versions...</div>
    }
    if (versionNotice != null) {
      return <div className="quick-search-notice">{versionNotice}</div>
    }
    if (earlierVersions.length === 0) {
      const onlyVersion = currentVersion ?? versionList?.versions[0] ?? null
      return (
        <div className="version-history-empty">
          <div className="version-history-empty-title">No earlier versions</div>
          <div className="version-history-empty-detail">
            {onlyVersion ? `${selectedFile.name} is currently ${versionLabel(onlyVersion)}.` : `${selectedFile.name} has no restorable versions.`}
          </div>
        </div>
      )
    }

    return (
      <div className="version-history-content">
        <div className="version-history-list">
          <div className="version-history-summary">
            <span>{currentVersion ? `Current ${versionLabel(currentVersion)}` : 'Current version'}</span>
            <span>{earlierVersions.length} earlier version{earlierVersions.length === 1 ? '' : 's'}</span>
          </div>
          <div className="quick-search-results" role="listbox" aria-label="Earlier versions">
            {earlierVersions.map((version, index) => {
              const active = selectedVersion?.id === version.id
              return (
                <button
                  key={version.id}
                  type="button"
                  className={`quick-search-result version-history-version ${active ? 'active' : ''}`}
                  onMouseEnter={() => setSelectedVersionIndex(index)}
                  onClick={() => setSelectedVersionIndex(index)}
                  role="option"
                  aria-selected={active}
                >
                  <span className="quick-search-result-icon">
                    <HistoryGlyph size={12} />
                  </span>
                  <span className="quick-search-result-main">
                    <span className="quick-search-result-name">{versionLabel(version)}</span>
                    <span className="quick-search-result-path">{formatTimestamp(version.created_at)}</span>
                  </span>
                  <span className="quick-search-result-meta">
                    <span>{formatBytes(version.size_bytes ?? -1)}</span>
                    <span>{sourceLabel(version.source)}</span>
                  </span>
                </button>
              )
            })}
          </div>
        </div>
        <VersionPreview
          file={selectedFile}
          version={selectedVersion}
          restoring={selectedVersion != null && restoringId === restoreVersionId(selectedVersion)}
          onRestore={restoreVersion}
        />
      </div>
    )
  }

  if (!open) return null

  return (
    // eslint-disable-next-line jsx-a11y/click-events-have-key-events, jsx-a11y/no-static-element-interactions -- Escape provides the keyboard-equivalent close action for this backdrop click-outside-to-dismiss pattern.
    <div className="quick-search-overlay" onClick={onClose}>
      {/* eslint-disable-next-line jsx-a11y/click-events-have-key-events, jsx-a11y/no-noninteractive-element-interactions -- This dialog container only stops backdrop clicks; Escape provides the keyboard-equivalent close action. */}
      <div
        className="quick-search-dialog version-history-dialog"
        role="dialog"
        aria-modal="true"
        aria-label="Version history"
        onClick={(event) => event.stopPropagation()}
        data-testid="desktop-version-history"
      >
        <div className={`quick-search-input-row version-history-input-row ${selectedFile ? 'selected' : ''}`}>
          <HistoryGlyph size={16} />
          {selectedFile ? (
            <div className="version-history-selected-file">
              <span className="version-history-selected-name" title={selectedFile.name}>{selectedFile.name}</span>
              <span className="version-history-selected-path" title={selectedFile.rel_path}>{selectedFile.rel_path}</span>
            </div>
          ) : (
            <input
              ref={inputRef}
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              onKeyDown={onInputKeyDown}
              placeholder="Search file names"
              aria-label="Search file names"
              spellCheck={false}
            />
          )}
          {selectedFile && (
            <button type="button" className="quick-search-close" onClick={backToSearch} aria-label="Back to file search">
              Back
            </button>
          )}
          <button type="button" className="quick-search-close" onClick={onClose} aria-label="Close version history">
            Close
          </button>
        </div>
        <div className="quick-search-body version-history-body">{versionBody()}</div>
      </div>
    </div>
  )
}

function VersionPreview({
  file,
  version,
  restoring,
  onRestore,
}: {
  file: DesktopSearchResult
  version: FileVersionEntry | null
  restoring: boolean
  onRestore: (version: FileVersionEntry) => void
}) {
  if (!version) {
    return <div className="version-history-preview">Select a version</div>
  }
  return (
    <div className="version-history-preview">
      <div>
        <div className="version-history-preview-label">Version preview</div>
        <div className="version-history-preview-title">{versionLabel(version)}</div>
      </div>
      <dl className="version-history-preview-meta">
        <div>
          <dt>File</dt>
          <dd title={file.name}>{file.name}</dd>
        </div>
        <div>
          <dt>Created</dt>
          <dd>{formatTimestamp(version.created_at)}</dd>
        </div>
        <div>
          <dt>Size</dt>
          <dd>{formatBytes(version.size_bytes ?? -1)}</dd>
        </div>
        <div>
          <dt>Source</dt>
          <dd>{sourceLabel(version.source)}</dd>
        </div>
        <div>
          <dt>Version id</dt>
          <dd title={restoreVersionId(version)}>{restoreVersionId(version)}</dd>
        </div>
      </dl>
      <button
        type="button"
        className="version-history-restore"
        disabled={restoring}
        onClick={() => onRestore(version)}
        aria-busy={restoring}
      >
        {restoring ? 'Queueing restore' : 'Queue restore'}
      </button>
    </div>
  )
}

function statusLabel(status: string): string {
  const words = status.replace(/_/g, ' ').trim()
  if (words.length === 0) return 'Unknown'
  return words.charAt(0).toUpperCase() + words.slice(1)
}

function formatModifiedAt(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return ''
  const date = new Date(seconds * 1000)
  if (Number.isNaN(date.getTime())) return ''
  return date.toLocaleDateString(undefined, {
    month: 'short',
    day: '2-digit',
  })
}

function formatTimestamp(value: string | number | null | undefined): string {
  if (typeof value === 'number') {
    const ms = value > 10_000_000_000 ? value : value * 1000
    const date = new Date(ms)
    return Number.isNaN(date.getTime()) ? 'Time unknown' : date.toLocaleString()
  }
  if (typeof value === 'string') {
    const parsed = Date.parse(value)
    return Number.isNaN(parsed) ? value : new Date(parsed).toLocaleString()
  }
  return 'Time unknown'
}

function versionLabel(version: FileVersionEntry): string {
  return version.version_number != null ? `v${version.version_number}` : version.id
}

function sourceLabel(source: string | null | undefined): string {
  switch (source) {
    case 'object_version':
      return 'Object'
    case 'file_version':
      return 'File'
    default:
      return 'Version'
  }
}
