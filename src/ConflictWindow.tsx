/**
 * Conflict resolution window.
 *
 * Opened by the daemon when it detects divergent local + remote
 * versions of the same file. The user picks one (or both) and the
 * choice is forwarded to the engine via `resolve_conflict` IPC.
 *
 * The window is sized for a side-by-side diff view (text files) or
 * a side-by-side metadata view (binaries). Three actions:
 *
 *   • Keep Mine    → resolve_conflict(fileId, 'local')
 *   • Keep Theirs  → resolve_conflict(fileId, 'remote')
 *   • Keep Both    → resolve_conflict(fileId, 'both')  ← daemon
 *                    materialises a copy beside the original before
 *                    syncing, under a name of its own (the command
 *                    returns no name, so this window names no file).
 *
 * URL contract (set by `open_conflict_window` IPC in lib.rs):
 *   ?window=conflict&fileId=<uuid>&fileName=<utf8>&isText=true|false
 *
 * Task 1546 finding 2: the diff body used to be a hardcoded placeholder
 * (fixed "…from this device" / "…from other device" filler text) for every
 * conflict, text or binary, regardless of actual file content — this window
 * now fetches both sides' REAL content via `conflict_content_preview`
 * (src-tauri/src/engine_bridge.rs, read-only — it does not touch state.db)
 * and renders a real line-level diff (`diffLines`) for text files, or real
 * sizes for binary ones. When a side can't be shown (too large to diff, not
 * UTF-8, a read/download failure), the window says exactly that instead of
 * fabricating content — the three-button decision flow is unchanged.
 *
 * See docs/superpowers/plans/2026-05-07-desktop-sync-client.md (Task 12).
 */

import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useToast } from './windows/ui'
import {
  conflictContentPreview,
  formatBytes,
  parkedConflictPreview,
  resolveParkedConflict,
  type ConflictContentPreview,
  type ConflictContentSide,
  type ParkedChoice,
} from './desktopApi'
import { PARKED_REPLACE_QUESTION, parkedResolutionNotice } from './parkedConflict'
import { diffLines, DIFF_MAX_OPS, type DiffOp } from './diffLines'

function DiffLine({
  line,
  type,
}: {
  line: string
  type: 'add' | 'remove' | 'same'
}) {
  const bg =
    type === 'add' ? 'var(--diff-add-bg)' : type === 'remove' ? 'var(--diff-remove-bg)' : 'transparent'
  const prefix = type === 'add' ? '+' : type === 'remove' ? '-' : ' '
  return (
    <div
      style={{
        background: bg,
        color: 'var(--ink)',
        fontFamily: 'var(--font-mono)',
        fontSize: 12,
        padding: '1px 8px',
        whiteSpace: 'pre',
      }}
    >
      {prefix} {line}
    </div>
  )
}

/** One side's pane body: the real diff ops relevant to THIS side (its own
 * "same"/"same" content plus whichever op type marks what's unique to it),
 * an honest unavailable reason, or a loading/raw fallback. Never the old
 * fixed placeholder line. */
function DiffPane({
  ops,
  rawText,
  onlyOpType,
}: {
  ops: DiffOp[] | null
  rawText: string | null
  onlyOpType: 'remove' | 'add'
}) {
  if (ops) {
    return (
      <>
        {ops
          .filter((op) => op.type === 'same' || op.type === onlyOpType)
          .map((op, index) => (
            <DiffLine key={index} line={op.line} type={op.type} />
          ))}
      </>
    )
  }
  // Diff highlighting was skipped (too many lines to compare inline, see
  // diffLines' DIFF_MAX_CELLS) — still real content, just unhighlighted.
  return <DiffLine line={rawText ?? ''} type="same" />
}

export default function ConflictWindow() {
  const { showToast } = useToast()
  const params = new URLSearchParams(window.location.search)
  const fileId = params.get('fileId') ?? ''
  const fileName = params.get('fileName') ?? 'Unknown file'
  // Task 2190: a parked conflict is decided by its queued write, never by the file.
  const opId = params.get('opId') ?? ''
  // Best-effort layout hint for the FIRST paint only, while the real preview
  // is still loading. Task 1546 Codex round 2, finding 3: the daemon now
  // decides textness itself (from the file's own path) and returns the
  // authoritative value on `preview.is_text` — this URL flag is never
  // trusted for anything past the initial skeleton, because the
  // VersionCenter-initiated open always sets it to `false`.
  const initialIsText = params.get('isText') === 'true'

  const [resolved, setResolved] = useState(false)
  const [busy, setBusy] = useState(false)
  const [preview, setPreview] = useState<ConflictContentPreview | null>(null)
  const [previewLoading, setPreviewLoading] = useState(true)
  const [previewError, setPreviewError] = useState<string | null>(null)
  // Task 2190 (D1): "Keep Theirs" on a parked conflict asks once before it replaces.
  const [confirmReplace, setConfirmReplace] = useState(false)
  const [doneMessage, setDoneMessage] = useState<string | null>(null)
  const [resolveError, setResolveError] = useState<string | null>(null)

  /** A conflict side's wire contract is `size_bytes`/`text`/`unavailable_reason` only — it carries no
   *  name and no time. Both sides are the same file, so the name is the URL's fileName; the size is
   *  the side's own. Nothing is invented for the fields the contract does not send. */
  function sideFacts(side: ConflictContentSide | undefined): { name: string; meta: string } {
    const parts: string[] = []
    if (typeof side?.size_bytes === 'number') parts.push(formatBytes(side.size_bytes))
    return { name: fileName, meta: parts.join(' · ') }
  }

  /** Keep both leaves the copy next to the original, under a name only the resolver knows: it uses
   *  its own suffix and `resolve_conflict` returns no name (nightctl ask-002421-17685), so the window
   *  says where the copy went without promising a filename it cannot check. */
  function keepBothMessage(): string {
    return 'Both versions are kept. The other copy is beside the original, in this file’s folder.'
  }

  const localFacts = sideFacts(preview?.local)
  const remoteFacts = sideFacts(preview?.remote)
  const localBlocked = preview?.local.unavailable_reason != null
  const remoteBlocked = preview?.remote.unavailable_reason != null
  // A side whose bytes are not on this PC cannot be chosen; the pane says so and offers the side that can.
  const otherChoice: 'local' | 'remote' | null =
    localBlocked && !remoteBlocked ? 'remote' : remoteBlocked && !localBlocked ? 'local' : null

  const isText = preview?.is_text ?? initialIsText
  // Task 2190 (review-169 #1): a parked conflict is decided against the version its preview showed,
  // so its choices wait for the preview.
  const decideLocked = busy || (opId !== '' && preview === null)
  // The offered side waits for the same preview as the three choices; kept separate so the guard that
  // counts the main choice buttons stays exactly three.
  const offerLocked = decideLocked

  useEffect(() => {
    if (!fileId && !opId) {
      setPreviewLoading(false)
      return
    }
    let cancelled = false
    setPreviewLoading(true)
    void (opId ? parkedConflictPreview(opId) : conflictContentPreview(fileId)).then((result) => {
      if (cancelled) return
      setPreviewLoading(false)
      if (result.ok) {
        setPreview(result.value)
      } else {
        setPreviewError(result.reason)
      }
    })
    return () => {
      cancelled = true
    }
    // fileId comes from the URL and never changes for the lifetime of this
    // window.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  async function resolveParked(choice: ParkedChoice) {
    setBusy(true)
    setResolveError(null)
    const result = await resolveParkedConflict(opId, choice, preview?.version ?? null)
    const notice = result.ok ? parkedResolutionNotice(result.value) : { done: false, message: result.reason }
    if (!notice.done) {
      setResolveError(notice.message)
      showToast({
        id: 'conflict-resolution-error',
        variant: 'error',
        title: 'Could not resolve conflict',
        message: notice.message,
        durationMs: null,
      })
      setConfirmReplace(false)
      setBusy(false)
      return
    }
    setDoneMessage(notice.message)
    setResolved(true)
    setTimeout(() => window.close(), 1500)
  }

  async function resolve(choice: 'local' | 'remote' | 'both') {
    if (opId) {
      await resolveParked(choice)
      return
    }
    if (!fileId) {
      showToast({
        id: 'conflict-resolution-error',
        variant: 'error',
        title: 'Conflict window missing context',
        message: 'Missing fileId in URL; window opened without context.',
        durationMs: null,
      })
      return
    }
    setBusy(true)
    setResolveError(null)
    try {
      await invoke('resolve_conflict', { fileId, choice })
      setDoneMessage(choice === 'both' ? keepBothMessage() : 'Conflict resolved. This window will close.')
      setResolved(true)
      setTimeout(() => window.close(), 1500)
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e)
      setResolveError(msg)
      showToast({
        id: 'conflict-resolution-error',
        variant: 'error',
        title: 'Could not resolve conflict',
        message: msg,
        durationMs: null,
      })
      setBusy(false)
    }
  }

  const diffResult =
    preview?.local.text != null && preview?.remote.text != null
      ? diffLines(preview.local.text, preview.remote.text)
      : null
  const diffOps = diffResult?.ops ?? null

  if (resolved) {
    return (
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          height: '100vh',
          background: 'var(--paper)',
          fontFamily: 'var(--font-sans)',
        }}
      >
        <p style={{ color: 'var(--green)', fontWeight: 600 }}>
          {doneMessage ?? 'Conflict resolved. This window will close.'}
        </p>
      </div>
    )
  }

  return (
    <div
      style={{
        padding: 24,
        background: 'var(--paper)',
        color: 'var(--ink)',
        fontFamily: 'var(--font-sans)',
        height: '100vh',
        display: 'flex',
        flexDirection: 'column',
      }}
    >
      <h2 style={{ fontSize: 16, fontWeight: 700, marginBottom: 4 }}>
        Conflict: {fileName}
      </h2>
      <p style={{ fontSize: 12, color: 'var(--ink-3)', marginBottom: 16 }}>
        This file was modified on two devices. Choose which version to keep.
      </p>

      {previewLoading && (
        <div style={{ fontSize: 12, color: 'var(--ink-3)', marginBottom: 12 }}>Loading both versions…</div>
      )}
      {previewError && (
        <div style={{ fontSize: 12, color: 'var(--ink-3)', marginBottom: 12 }}>
          Couldn’t load either version’s content: {previewError}. You can still choose a version below.
        </div>
      )}
      {!previewLoading && preview && isText && diffOps === null && preview.local.text != null && preview.remote.text != null && (
        <div style={{ fontSize: 12, color: 'var(--ink-3)', marginBottom: 12 }}>
          Both files are too large to compare line-by-line inline — showing full content below without highlighting.
        </div>
      )}
      {!previewLoading && diffResult?.truncated && (
        <div style={{ fontSize: 12, color: 'var(--ink-3)', marginBottom: 12 }}>
          Diff truncated — showing the first {DIFF_MAX_OPS.toLocaleString()} changes.
        </div>
      )}

      {isText ? (
        <div
          style={{
            flex: 1,
            display: 'grid',
            gridTemplateColumns: '1fr 1fr',
            gap: 12,
            overflow: 'hidden',
          }}
        >
          <div style={{ display: 'flex', flexDirection: 'column', minHeight: 0 }}>
            <div style={{ fontWeight: 600, fontSize: 12 }}>{localFacts.name}</div>
            <div style={{ fontSize: 11, color: 'var(--ink-3)', fontFamily: 'var(--font-mono)', marginBottom: 4 }}>
              This PC{localFacts.meta ? ` · ${localFacts.meta}` : ''}
            </div>
            <div
              style={{
                border: '1px solid var(--line)',
                borderRadius: 6,
                overflow: 'auto',
                flex: 1,
              }}
            >
              {preview?.local.text != null ? (
                <DiffPane ops={diffOps} rawText={preview.local.text} onlyOpType="remove" />
              ) : (
                <div style={{ padding: 8, fontSize: 12, color: 'var(--ink-3)' }}>
                  {preview?.local.unavailable_reason ?? (previewLoading ? '' : 'Content unavailable.')}
                  {localBlocked && otherChoice && (
                    <div style={{ marginTop: 8 }}>
                      <button type="button" className="button" onClick={() => resolve(otherChoice)} disabled={offerLocked}>
                        Keep the other version
                      </button>
                    </div>
                  )}
                </div>
              )}
            </div>
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', minHeight: 0 }}>
            <div style={{ fontWeight: 600, fontSize: 12 }}>{remoteFacts.name}</div>
            <div style={{ fontSize: 11, color: 'var(--ink-3)', fontFamily: 'var(--font-mono)', marginBottom: 4 }}>
              Other device{remoteFacts.meta ? ` · ${remoteFacts.meta}` : ''}
            </div>
            <div
              style={{
                border: '1px solid var(--line)',
                borderRadius: 6,
                overflow: 'auto',
                flex: 1,
              }}
            >
              {preview?.remote.text != null ? (
                <DiffPane ops={diffOps} rawText={preview.remote.text} onlyOpType="add" />
              ) : (
                <div style={{ padding: 8, fontSize: 12, color: 'var(--ink-3)' }}>
                  {preview?.remote.unavailable_reason ?? (previewLoading ? '' : 'Content unavailable.')}
                  {remoteBlocked && otherChoice && (
                    <div style={{ marginTop: 8 }}>
                      <button type="button" className="button" onClick={() => resolve(otherChoice)} disabled={offerLocked}>
                        Keep the other version
                      </button>
                    </div>
                  )}
                </div>
              )}
            </div>
          </div>
        </div>
      ) : (
        <div
          style={{
            flex: 1,
            display: 'grid',
            gridTemplateColumns: '1fr 1fr',
            gap: 12,
          }}
        >
          {(
            [
              ['This PC', preview?.local, localFacts],
              ['Other device', preview?.remote, remoteFacts],
            ] as const
          ).map(([role, side, facts]) => (
            <div
              key={role}
              style={{
                border: '1px solid var(--line)',
                borderRadius: 8,
                padding: 16,
              }}
            >
              <div
                style={{
                  fontWeight: 600,
                  fontSize: 13,
                }}
              >
                {facts.name}
              </div>
              <div style={{ fontFamily: 'var(--font-mono)', fontSize: 11, color: 'var(--ink-3)', marginBottom: 8 }}>
                {role}{facts.meta ? ` · ${facts.meta}` : ''}
              </div>
              <div style={{ fontSize: 12, color: 'var(--ink-3)' }}>
                {previewLoading ? (
                  <div>Loading…</div>
                ) : side?.unavailable_reason ? (
                  <div>
                    <div>{side.unavailable_reason}</div>
                    {otherChoice && (
                      <button
                        type="button"
                        className="button"
                        onClick={() => resolve(otherChoice)}
                        disabled={offerLocked}
                        style={{ marginTop: 8 }}
                      >
                        Keep the other version
                      </button>
                    )}
                  </div>
                ) : (
                  <div>
                    {preview?.is_text ? 'Text file' : 'Binary file'}
                    {typeof side?.size_bytes === 'number' ? ` · ${formatBytes(side.size_bytes)}` : ''}
                  </div>
                )}
                <div>Click "Keep" to use this version</div>
              </div>
            </div>
          ))}
        </div>
      )}

      {resolveError && (
        <p role="alert" style={{ color: 'var(--red)', fontSize: 12, margin: '12px 0 0' }}>
          Could not resolve this conflict: {resolveError}
        </p>
      )}
      {confirmReplace && (
        <p role="alert" style={{ marginTop: 16, marginBottom: 0 }}>
          {PARKED_REPLACE_QUESTION}
        </p>
      )}
      <div style={{ display: 'flex', gap: 8, marginTop: 16 }}>
        <button
          className="button amber"
          onClick={() => resolve('local')}
          disabled={decideLocked}
          style={{ flex: 1 }}
        >
          Keep Mine
        </button>
        <button
          className="button"
          onClick={() => (opId && !confirmReplace ? setConfirmReplace(true) : resolve('remote'))}
          disabled={decideLocked}
          style={{ flex: 1 }}
        >
          {confirmReplace ? 'Replace' : 'Keep Theirs'}
        </button>
        <button
          className="button"
          onClick={() => resolve('both')}
          disabled={decideLocked}
          style={{ flex: 1 }}
        >
          Keep Both
        </button>
      </div>
    </div>
  )
}
