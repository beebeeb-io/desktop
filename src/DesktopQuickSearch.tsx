import { useEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from 'react'
import {
  command,
  commandUnavailableLabel,
  desktopSearchFiles,
  formatBytes,
  type DesktopSearchResponse,
  type DesktopSearchResult,
} from './desktopApi'

const SEARCH_LIMIT = 12
const SEARCH_DEBOUNCE_MS = 120

export function SearchGlyph({ size = 14 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" aria-hidden="true">
      <circle cx="7" cy="7" r="4.2" />
      <path d="M10.2 10.2 L13.5 13.5" />
    </svg>
  )
}

export function DesktopQuickSearchTrigger({
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
      aria-label="Search files"
      title="Search files"
    >
      <span className="quick-search-trigger-icon">
        <SearchGlyph size={13} />
      </span>
      <span className="quick-search-trigger-label">Search</span>
    </button>
  )
}

export default function DesktopQuickSearch({
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
  const [openingId, setOpeningId] = useState<string | null>(null)
  const inputRef = useRef<HTMLInputElement | null>(null)
  const requestId = useRef(0)

  const trimmedQuery = query.trim()
  const results = response?.results ?? []
  const indexedFileCount = response?.indexed_file_count ?? 0
  const indexSyncing = response?.index_state === 'syncing'

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') {
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
    setQuery('')
    setLoading(false)
    setResponse(null)
    setNotice(null)
    setActiveIndex(0)
    setOpeningId(null)
    const focusTimer = window.setTimeout(() => inputRef.current?.focus(), 0)
    return () => window.clearTimeout(focusTimer)
  }, [open])

  useEffect(() => {
    if (!open || trimmedQuery.length === 0) {
      setLoading(false)
      setResponse(null)
      setNotice(null)
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
  }, [open, trimmedQuery])

  const revealResult = async (result: DesktopSearchResult) => {
    setOpeningId(result.file_id)
    setNotice(null)
    const opened = await command<void>('open_in_finder', {
      itemId: result.file_id,
      path: result.rel_path,
    })
    if (opened.ok) {
      onClose()
    } else {
      setNotice(opened.unsupported ? commandUnavailableLabel('open_in_finder') : opened.reason)
    }
    setOpeningId(null)
  }

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
    if (event.key === 'Enter' && results[activeIndex] && openingId == null) {
      event.preventDefault()
      void revealResult(results[activeIndex])
    }
  }

  const body = useMemo(() => {
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
      <div className="quick-search-results" role="listbox" aria-label="Search results">
        {results.map((result, index) => (
          <button
            key={result.file_id}
            type="button"
            className={`quick-search-result ${activeIndex === index ? 'active' : ''}`}
            onMouseEnter={() => setActiveIndex(index)}
            onClick={() => void revealResult(result)}
            disabled={openingId === result.file_id}
            role="option"
            aria-selected={activeIndex === index}
          >
            <span className="quick-search-result-icon">
              <SearchGlyph size={12} />
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
  }, [activeIndex, indexedFileCount, indexSyncing, loading, notice, openingId, results, trimmedQuery])

  if (!open) return null

  return (
    <div className="quick-search-overlay" onClick={onClose}>
      <div
        className="quick-search-dialog"
        role="dialog"
        aria-modal="true"
        aria-label="File search"
        onClick={(event) => event.stopPropagation()}
        data-testid="desktop-quick-search"
      >
        <div className="quick-search-input-row">
          <SearchGlyph size={16} />
          <input
            ref={inputRef}
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={onInputKeyDown}
            placeholder="Search file names"
            aria-label="Search file names"
            spellCheck={false}
          />
          <button type="button" className="quick-search-close" onClick={onClose} aria-label="Close search">
            Close
          </button>
        </div>
        <div className="quick-search-body">{body}</div>
      </div>
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
