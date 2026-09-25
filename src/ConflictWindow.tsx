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
 *                    materialises a `(Conflict from Device, HH:MM)`
 *                    copy before syncing — see plan Phase 4.
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
import { conflictContentPreview, formatBytes, type ConflictContentPreview } from './desktopApi'
import { diffLines, type DiffOp } from './diffLines'

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
  const isText = params.get('isText') === 'true'

  const [resolved, setResolved] = useState(false)
  const [busy, setBusy] = useState(false)
  const [preview, setPreview] = useState<ConflictContentPreview | null>(null)
  const [previewLoading, setPreviewLoading] = useState(true)
  const [previewError, setPreviewError] = useState<string | null>(null)

  useEffect(() => {
    if (!fileId) {
      setPreviewLoading(false)
      return
    }
    let cancelled = false
    setPreviewLoading(true)
    void conflictContentPreview(fileId, isText).then((result) => {
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
    // fileId/isText come from the URL and never change for the lifetime of
    // this window.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  async function resolve(choice: 'local' | 'remote' | 'both') {
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
    try {
      await invoke('resolve_conflict', { fileId, choice })
      setResolved(true)
      setTimeout(() => window.close(), 1500)
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e)
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

  const diffOps =
    preview?.local.text != null && preview?.remote.text != null
      ? diffLines(preview.local.text, preview.remote.text)
      : null

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
          ✓ Conflict resolved
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
            <div style={{ fontWeight: 600, fontSize: 12, marginBottom: 4 }}>
              This device
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
                </div>
              )}
            </div>
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', minHeight: 0 }}>
            <div style={{ fontWeight: 600, fontSize: 12, marginBottom: 4 }}>
              Other device
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
              ['This device', preview?.local],
              ['Other device', preview?.remote],
            ] as const
          ).map(([label, side]) => (
            <div
              key={label}
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
                  marginBottom: 8,
                }}
              >
                {label}
              </div>
              <div style={{ fontSize: 12, color: 'var(--ink-3)' }}>
                {previewLoading ? (
                  <div>Loading…</div>
                ) : side?.unavailable_reason ? (
                  <div>{side.unavailable_reason}</div>
                ) : (
                  <div>
                    Binary file{typeof side?.size_bytes === 'number' ? ` · ${formatBytes(side.size_bytes)}` : ''}
                  </div>
                )}
                <div>Click "Keep" to use this version</div>
              </div>
            </div>
          ))}
        </div>
      )}

      <div style={{ display: 'flex', gap: 8, marginTop: 16 }}>
        <button
          className="button amber"
          onClick={() => resolve('local')}
          disabled={busy}
          style={{ flex: 1 }}
        >
          Keep Mine
        </button>
        <button
          className="button"
          onClick={() => resolve('remote')}
          disabled={busy}
          style={{ flex: 1 }}
        >
          Keep Theirs
        </button>
        <button
          className="button"
          onClick={() => resolve('both')}
          disabled={busy}
          style={{ flex: 1 }}
        >
          Keep Both
        </button>
      </div>
    </div>
  )
}
