/**
 * The version-history sheet (task 1683 slice 4; spec `docs/specs/2026-10-24-macos-two-surface-app.md` §7.6 "Version
 * history… (toolbar button): a sheet that searches file names and lists versions with 'Restore'", §10.2 rows 14 and
 * 15). It is the one Mac implementation of what `DesktopVersionHistory.tsx` (Windows and Linux, a quick-search style
 * dialog) and `pages/VersionCenter.tsx` (Linux, a page) each do, both of which stay as they are.
 *
 * Two steps in one sheet: search the vault by file name (`desktop_search_files`, debounced), pick a file, and its
 * earlier versions (`list_file_versions`) each get "Restore" (`restore_file_version`). A restore is QUEUED, and the toast
 * says so: the sync picks it up (the server creates a new current version when it succeeds), so the sheet never claims
 * "restored".
 *
 * The sheet is the window's `Modal` (an open sheet: nothing navigates under it, §8.2). It opens on the search field,
 * not on its close button: typing is what the sheet is for, and nothing in it is destructive on Enter. This is a
 * deliberate difference from the conflict and delete sheets (§11), recorded in the PR.
 *
 * The components are plain function declarations that read only their props, hooks and the bindings imported here, so
 * `tests/versionHistorySheet.test.tsx` can execute them with the component harness.
 */
import { useEffect, useRef, useState } from 'react'
import {
  commandUnavailableLabel,
  desktopListFileVersions,
  desktopRestoreFileVersion,
  desktopSearchFiles,
  restorableFileVersions,
  restoreVersionId,
  type DesktopSearchResponse,
  type DesktopSearchResult,
  type FileVersionEntry,
  type FileVersionListResponse,
} from './desktopApi'
import { Btn, Note } from './macSettingsParts'
import { restoreQueuedToast, VERSIONS_LOAD_FAILED, versionLabel, versionLine } from './reviewModel'
import { Modal, useToast } from './windows/ui'

const SEARCH_LIMIT = 12
const SEARCH_DEBOUNCE_MS = 120

type SearchState =
  | { status: 'idle' }
  | { status: 'loading' }
  | { status: 'ready'; response: DesktopSearchResponse }
  | { status: 'failed'; reason: string }

type VersionsState =
  | { status: 'idle' }
  | { status: 'loading' }
  | { status: 'ready'; list: FileVersionListResponse }
  | { status: 'failed'; reason: string }

export function VersionHistorySheet({ onClose }: { onClose: () => void }) {
  const { showToast } = useToast()
  const [query, setQuery] = useState('')
  const [search, setSearch] = useState<SearchState>({ status: 'idle' })
  const [file, setFile] = useState<DesktopSearchResult | null>(null)
  const [versions, setVersions] = useState<VersionsState>({ status: 'idle' })
  const [restoringId, setRestoringId] = useState<string | null>(null)
  const inputRef = useRef<HTMLInputElement | null>(null)
  const searchRequest = useRef(0)
  const versionsRequest = useRef(0)
  // Bumped by "Try again": the versions of the file picked are a load, so a failure to load them is shown inline.
  const [versionsAttempt, setVersionsAttempt] = useState(0)
  const trimmed = query.trim()

  // The search settles after the typing does; an answer to an older query is dropped.
  useEffect(() => {
    if (file !== null || trimmed.length === 0) {
      setSearch({ status: 'idle' })
      return
    }
    const current = ++searchRequest.current
    let cancelled = false
    setSearch({ status: 'loading' })
    const timer = window.setTimeout(() => {
      void desktopSearchFiles(trimmed, SEARCH_LIMIT).then((result) => {
        if (cancelled || searchRequest.current !== current) return
        if (result.ok) setSearch({ status: 'ready', response: result.value })
        else setSearch({ status: 'failed', reason: result.unsupported ? commandUnavailableLabel('desktop_search_files') : result.reason })
      })
    }, SEARCH_DEBOUNCE_MS)
    return () => {
      cancelled = true
      window.clearTimeout(timer)
    }
  }, [file, trimmed])

  // The picked file's versions: a load, started by the pick and by "Try again". An answer for a file the person has
  // since left (Back, or another pick) is dropped.
  useEffect(() => {
    if (file === null) {
      setVersions({ status: 'idle' })
      return
    }
    const current = ++versionsRequest.current
    let cancelled = false
    setVersions({ status: 'loading' })
    void desktopListFileVersions(file.file_id).then((result) => {
      if (cancelled || versionsRequest.current !== current) return
      if (result.ok) setVersions({ status: 'ready', list: result.value })
      else setVersions({ status: 'failed', reason: result.unsupported ? commandUnavailableLabel('list_file_versions') : result.reason })
    })
    return () => {
      cancelled = true
    }
  }, [file, versionsAttempt])

  const back = () => {
    setFile(null)
    setRestoringId(null)
  }

  const restore = async (version: FileVersionEntry) => {
    if (file === null) return
    setRestoringId(version.id)
    const result = await desktopRestoreFileVersion(file.file_id, restoreVersionId(version))
    setRestoringId(null)
    if (result.ok) {
      showToast({ ...restoreQueuedToast(file.name, version.version_number) })
      return
    }
    showToast({
      variant: 'error',
      title: 'Restore failed',
      message: result.unsupported ? commandUnavailableLabel('restore_file_version') : result.reason,
      durationMs: 9000,
    })
  }

  const now = new Date()
  let body
  if (file === null) {
    let results
    if (search.status === 'idle') {
      results = <div className="mw-sheet-note">Search for a file to see its earlier versions.</div>
    } else if (search.status === 'loading') {
      results = <div className="mw-sheet-note" role="status">Searching…</div>
    } else if (search.status === 'failed') {
      results = <div className="mw-sheet-note">{search.reason}</div>
    } else if (search.response.index_state === 'syncing') {
      results = <div className="mw-sheet-note">Search index is syncing.</div>
    } else if (search.response.results.length === 0) {
      results = <div className="mw-sheet-note">{search.response.indexed_file_count === 0 ? 'No indexed files.' : 'No matching files.'}</div>
    } else {
      results = (
        <div className="mw-pick-list">
          {search.response.results.map((hit) => (
            <button
              key={hit.file_id}
              type="button"
              className="mw-pick"
              aria-label={hit.rel_path && hit.rel_path !== hit.name ? `${hit.name} ${hit.rel_path}` : hit.name}
              onClick={() => setFile(hit)}
            >
              <span className="mw-pick-name">{hit.name}</span>
              <span className="mw-pick-path ms-mono">{hit.rel_path}</span>
            </button>
          ))}
        </div>
      )
    }
    body = (
      <>
        <input
          ref={inputRef}
          type="text"
          className="mw-search"
          value={query}
          onChange={(event) => setQuery(event.currentTarget.value)}
          placeholder="Search file names"
          aria-label="Search file names"
          spellCheck={false}
        />
        {results}
      </>
    )
  } else {
    let list
    if (versions.status === 'loading' || versions.status === 'idle') {
      list = <div className="mw-sheet-note" role="status">Loading versions…</div>
    } else if (versions.status === 'failed') {
      list = (
        <Note kind="alert" surface="version-history" reason={versions.reason} actions={<Btn onClick={() => setVersionsAttempt((n) => n + 1)}>Try again</Btn>}>
          {VERSIONS_LOAD_FAILED}
        </Note>
      )
    } else {
      const earlier = restorableFileVersions(versions.list.versions)
      const current = versions.list.versions.find((version) => version.is_current)
      if (earlier.length === 0) {
        list = (
          <div className="mw-sheet-empty">
            <div className="mw-row-title">No earlier versions</div>
            <div className="mw-row-hint">{`${file.name} has no earlier versions to go back to.`}</div>
          </div>
        )
      } else {
        const lead = current?.version_number != null ? `Version ${current.version_number} is current` : 'Showing earlier versions'
        list = (
          <>
            <div className="mw-row-hint">{`${lead} · ${earlier.length} earlier ${earlier.length === 1 ? 'version' : 'versions'}`}</div>
            <div className="mw-card">
              {earlier.map((version) => (
                <div className="mw-row" data-row="version" key={version.id}>
                  <div className="mw-row-text">
                    <div className="mw-row-title">{versionLabel(version)}</div>
                    <div className="mw-row-hint">{versionLine(version, now)}</div>
                  </div>
                  <Btn disabled={restoringId !== null} onClick={() => void restore(version)}>
                    {restoringId === version.id ? 'Restoring…' : 'Restore'}
                  </Btn>
                </div>
              ))}
            </div>
          </>
        )
      }
    }
    body = (
      <>
        <div className="mw-sheet-file">
          <div className="mw-row-text">
            <div className="mw-row-title">{file.name}</div>
            <div className="mw-row-hint ms-mono">{file.rel_path}</div>
          </div>
          <Btn onClick={back}>Back</Btn>
        </div>
        {list}
      </>
    )
  }

  return (
    <Modal open onClose={onClose} title="Version history" ariaLabel="Version history" maxWidth={520} initialFocusRef={inputRef}>
      <div className="mw-sheet-stack">{body}</div>
    </Modal>
  )
}
