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
 * The `resolve_conflict` IPC command is added by the rust-engineer in
 * a sister task (plan §1749). Until then `invoke` will reject — we
 * surface the error instead of silently failing.
 *
 * The diff body is currently a placeholder ("Content from this
 * device…" / "Content from other device…") because actual diffing
 * needs the daemon to expose both blob bytes — that's tracked in
 * the plan as a follow-up. The shell, the URL contract, and the
 * three-button decision flow are correct as-is.
 *
 * See docs/superpowers/plans/2026-05-07-desktop-sync-client.md (Task 12).
 */

import { useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useToast } from './windows/ui'

function DiffLine({
  line,
  type,
}: {
  line: string
  type: 'add' | 'remove' | 'same'
}) {
  const bg =
    type === 'add' ? '#dcfce7' : type === 'remove' ? '#fee2e2' : 'transparent'
  const prefix = type === 'add' ? '+' : type === 'remove' ? '-' : ' '
  return (
    <div
      style={{
        background: bg,
        fontFamily: 'monospace',
        fontSize: 12,
        padding: '1px 8px',
        whiteSpace: 'pre',
      }}
    >
      {prefix} {line}
    </div>
  )
}

export default function ConflictWindow() {
  const { showToast } = useToast()
  const params = new URLSearchParams(window.location.search)
  const fileId = params.get('fileId') ?? ''
  const fileName = params.get('fileName') ?? 'Unknown file'
  const isText = params.get('isText') === 'true'

  const [resolved, setResolved] = useState(false)
  const [busy, setBusy] = useState(false)

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

  if (resolved) {
    return (
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          height: '100vh',
          fontFamily: 'Inter, sans-serif',
        }}
      >
        <p style={{ color: '#22c55e', fontWeight: 600 }}>
          ✓ Conflict resolved
        </p>
      </div>
    )
  }

  return (
    <div
      style={{
        padding: 24,
        fontFamily: 'Inter, sans-serif',
        height: '100vh',
        display: 'flex',
        flexDirection: 'column',
      }}
    >
      <h2 style={{ fontSize: 16, fontWeight: 700, marginBottom: 4 }}>
        Conflict: {fileName}
      </h2>
      <p style={{ fontSize: 12, color: '#6b7280', marginBottom: 16 }}>
        This file was modified on two devices. Choose which version to keep.
      </p>

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
                border: '1px solid #e5e7eb',
                borderRadius: 6,
                overflow: 'auto',
                flex: 1,
              }}
            >
              <DiffLine line="Content from this device…" type="add" />
            </div>
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', minHeight: 0 }}>
            <div style={{ fontWeight: 600, fontSize: 12, marginBottom: 4 }}>
              Other device
            </div>
            <div
              style={{
                border: '1px solid #e5e7eb',
                borderRadius: 6,
                overflow: 'auto',
                flex: 1,
              }}
            >
              <DiffLine line="Content from other device…" type="remove" />
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
          {['This device', 'Other device'].map((label) => (
            <div
              key={label}
              style={{
                border: '1px solid #e5e7eb',
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
              <div style={{ fontSize: 12, color: '#6b7280' }}>
                <div>Binary file</div>
                <div>Click "Keep" to use this version</div>
              </div>
            </div>
          ))}
        </div>
      )}

      <div style={{ display: 'flex', gap: 8, marginTop: 16 }}>
        <button
          onClick={() => resolve('local')}
          disabled={busy}
          style={{
            flex: 1,
            padding: '10px',
            background: '#3b82f6',
            color: 'white',
            border: 'none',
            borderRadius: 6,
            cursor: busy ? 'not-allowed' : 'pointer',
            opacity: busy ? 0.6 : 1,
            fontWeight: 600,
          }}
        >
          Keep Mine
        </button>
        <button
          onClick={() => resolve('remote')}
          disabled={busy}
          style={{
            flex: 1,
            padding: '10px',
            background: '#6b7280',
            color: 'white',
            border: 'none',
            borderRadius: 6,
            cursor: busy ? 'not-allowed' : 'pointer',
            opacity: busy ? 0.6 : 1,
            fontWeight: 600,
          }}
        >
          Keep Theirs
        </button>
        <button
          onClick={() => resolve('both')}
          disabled={busy}
          style={{
            flex: 1,
            padding: '10px',
            background: '#f3f4f6',
            color: '#374151',
            border: '1px solid #e5e7eb',
            borderRadius: 6,
            cursor: busy ? 'not-allowed' : 'pointer',
            opacity: busy ? 0.6 : 1,
            fontWeight: 600,
          }}
        >
          Keep Both
        </button>
      </div>
    </div>
  )
}
