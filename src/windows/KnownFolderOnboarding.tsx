/**
 * KnownFolderOnboarding — first-run "Back up these folders?" prompt (task 0804).
 *
 * Decision 0797 locked Desktop / Documents / Pictures as the default-ON backup
 * set (matching OneDrive), Music / Videos / Downloads OFF. But a settings panel
 * must never silently start uploading personal data — so the default-ON lives
 * HERE, in a one-time opt-in prompt the user sees and confirms, not in the
 * Manage-backup panel (which ships every folder default-OFF, task 0797).
 *
 * Show-once governs only the AUTOMATIC first-run appearance: the prompt surfaces
 * on app start when `!known_folder_onboarding_seen` AND no folders are backed up.
 * After either choice ("Back up these folders" or "Not now") it marks the flag
 * and never auto-shows again — but the Manage-backup panel can re-open it
 * manually (`forced` mode) so a user who skipped can still set backup up later.
 *
 * Brand: amber is reserved for encryption state + the primary action — here the
 * single "Back up these folders" CTA. Everything else is neutral ink. Inter for
 * humans, JetBrains Mono for paths/counts. Honest voice (what's copied, that
 * originals stay put so it coexists with OneDrive, that backed-up folders are
 * kept on this PC). City only ("Falkenstein"), never the storage provider. No
 * emojis.
 */

import { useEffect, useMemo, useRef, useState } from 'react'
import {
  getKnownFolderBackup,
  getKnownFolderOnboardingSeen,
  setKnownFolderBackup,
  setRecursivePin,
  markKnownFolderOnboardingSeen,
  listVaultFolders,
  commandUnavailableLabel,
  type KnownFolderStatus,
} from '../desktopApi'
import { T, NavIcon, useToast } from './ui'
import { useRegionLabel } from './useRegion'

/** Decision 0797's default-ON set: pre-checked on first run. */
const DEFAULT_ON_KEYS = new Set(['desktop', 'documents', 'pictures'])

/** Rough item-count label, e.g. "1,240 files" / "10,000+ files". Empty when unknown. */
function itemCountLabel(f: KnownFolderStatus): string {
  if (f.item_count == null || f.item_count <= 0) return ''
  const n = f.item_count.toLocaleString('en-US')
  const plus = f.item_count >= 10000 ? '+' : ''
  return `${n}${plus} ${f.item_count === 1 ? 'file' : 'files'}`
}

/** One folder row with a checkbox. Neutral palette — amber is the CTA only. */
function FolderRow({
  folder,
  checked,
  onToggle,
}: {
  folder: KnownFolderStatus
  checked: boolean
  onToggle: () => void
}) {
  const count = itemCountLabel(folder)
  return (
    <button
      type="button"
      role="checkbox"
      aria-checked={checked}
      onClick={onToggle}
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 12,
        width: '100%',
        padding: '11px 14px',
        textAlign: 'left' as const,
        background: checked ? T.paper2 : 'transparent',
        border: 'none',
        borderTop: `1px solid ${T.line}`,
        cursor: 'pointer',
        fontFamily: T.fontSans,
      }}
    >
      {/* Checkbox */}
      <span
        aria-hidden
        style={{
          width: 18,
          height: 18,
          flexShrink: 0,
          borderRadius: 5,
          border: `1.5px solid ${checked ? T.ink : T.line2}`,
          background: checked ? T.ink : T.paper,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          transition: 'background 100ms ease, border-color 100ms ease',
        }}
      >
        {checked && <NavIcon name="check" size={11} color={T.paper} />}
      </span>

      {/* Folder icon */}
      <span
        style={{
          width: 28,
          height: 28,
          flexShrink: 0,
          borderRadius: 7,
          background: T.paper2,
          border: `1px solid ${T.line}`,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
        }}
      >
        <NavIcon name="folder" size={13} color={T.ink3} />
      </span>

      {/* Name + path + count */}
      <span style={{ flex: 1, minWidth: 0 }}>
        <span style={{ display: 'flex', alignItems: 'baseline', gap: 8 }}>
          <span style={{ fontSize: 13, fontWeight: 600, color: T.ink }}>{folder.display_name}</span>
          {count && (
            <span style={{ fontSize: 10.5, fontFamily: T.fontMono, color: T.ink4 }}>{count}</span>
          )}
        </span>
        {folder.source_path && (
          <span
            title={folder.source_path}
            style={{
              display: 'block',
              fontSize: 10.5,
              fontFamily: T.fontMono,
              color: T.ink4,
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap' as const,
              marginTop: 1,
            }}
          >
            {folder.source_path}
          </span>
        )}
      </span>
    </button>
  )
}

export default function KnownFolderOnboarding({
  /** When true, render unconditionally (manual "Set up backup" re-open). When
   *  false, the component self-gates on the seen flag + empty backup set. */
  forced = false,
  /** Called after the prompt resolves (confirm or skip) so the host can hide
   *  it. Receives whether any folder was enabled, so the host can refresh. */
  onClose,
}: {
  forced?: boolean
  onClose: (enabledAny: boolean) => void
}) {
  const regionLabel = useRegionLabel()

  // null = still loading; [] = no Windows known folders here (non-Windows) →
  // never show. Otherwise the resolvable rows.
  const [folders, setFolders] = useState<KnownFolderStatus[] | null>(null)
  const [checked, setChecked] = useState<Set<string>>(new Set())
  const [submitting, setSubmitting] = useState(false)
  const { showToast } = useToast()
  // false until we've confirmed this is a genuine first-run (or forced). Keeps
  // the overlay from flashing before the seen-flag/backup checks resolve.
  const [shouldShow, setShouldShow] = useState(forced)

  // The load+gate runs exactly ONCE per mount. The parent passes a fresh
  // `onClose`/`forced` on every render (it inlines the callback and re-renders
  // on a status poll), so we must NOT put them in the dep array — a re-run
  // would re-fetch and reset the user's checkbox edits mid-decision. The
  // parent remounts (via a bumping `key`) for a forced re-open, which re-runs
  // this effect cleanly. Read the latest props through refs so the one run
  // still sees current values.
  const onCloseRef = useRef(onClose)
  const forcedRef = useRef(forced)
  onCloseRef.current = onClose
  forcedRef.current = forced

  useEffect(() => {
    let cancelled = false
    void (async () => {
      const isForced = forcedRef.current
      const close = (enabledAny: boolean) => {
        if (!cancelled) onCloseRef.current(enabledAny)
      }

      // Auto path only: respect show-once. A forced re-open ("Set up backup")
      // bypasses the seen flag — it's an explicit user request.
      if (!isForced) {
        const seen = await getKnownFolderOnboardingSeen()
        if (cancelled) return
        // Already shown once → never auto-surface again. (A read failure is
        // treated as "seen" so we never nag on a flaky read.)
        if (!seen.ok || seen.value) {
          close(false)
          return
        }
      }

      const res = await getKnownFolderBackup()
      if (cancelled) return
      if (!res.ok) {
        // Unsupported (non-Windows) or errored — never show the prompt.
        close(false)
        return
      }
      // Only folders that resolve on this machine are backupable. On a
      // non-Windows build every row has source_path: null → nothing to offer.
      const resolvable = res.value.filter((f) => f.source_path != null)
      if (resolvable.length === 0) {
        close(false)
        return
      }
      // Auto-show gate: if ANY folder is already backed up, the user has set
      // backup up before — don't surface the first-run prompt. (A forced
      // re-open bypasses this.)
      if (!isForced && resolvable.some((f) => f.enabled)) {
        close(false)
        return
      }
      setFolders(resolvable)
      // Pre-check the default-ON set (decision 0797), intersected with what
      // actually resolves on this machine.
      setChecked(new Set(resolvable.filter((f) => DEFAULT_ON_KEYS.has(f.key)).map((f) => f.key)))
      setShouldShow(true)
    })()
    return () => {
      cancelled = true
    }
  }, [])

  const toggle = (key: string) => {
    setChecked((prev) => {
      const next = new Set(prev)
      if (next.has(key)) next.delete(key)
      else next.add(key)
      return next
    })
  }

  const checkedKeys = useMemo(() => folders?.filter((f) => checked.has(f.key)).map((f) => f.key) ?? [], [folders, checked])

  /** "Not now": just record that the prompt was shown; enable nothing. */
  const skip = async () => {
    setSubmitting(true)
    await markKnownFolderOnboardingSeen()
    onClose(false)
  }

  /** Confirm: enable each checked folder (kicks the mirror), pin the matching
   *  vault folders that already exist (best-effort — fresh folders materialise
   *  as the upload completes and are picked up by the reconcile flow), then
   *  record the prompt as seen so it never auto-shows again. */
  const confirm = async () => {
    setSubmitting(true)

    // 1. Enable each checked folder. Each enable persists the choice and (on
    //    Windows) kicks an immediate one-way mirror pass.
    let anyEnabled = false
    for (const key of checkedKeys) {
      const res = await setKnownFolderBackup(key, true)
      if (!res.ok) {
        showToast({
          variant: 'error',
          title: 'Backup setup failed',
          message: res.unsupported ? commandUnavailableLabel('set_known_folder_backup') : res.reason,
        })
        setSubmitting(false)
        return
      }
      anyEnabled = true
    }

    // 2. Pin the matching vault folders so the backed-up copies are kept on
    //    this PC (always-local), honouring the prompt's promise. Best-effort:
    //    a vault folder only exists once its files have uploaded, so on a fresh
    //    backup there may be nothing to pin yet — that's expected, the existing
    //    pin/reconcile flow keeps newly-materialised folders pinned. Pin
    //    failures never undo the enable above.
    if (anyEnabled && checkedKeys.length > 0) {
      const tree = await listVaultFolders()
      if (tree.ok) {
        const wantNames = new Set(
          (folders ?? []).filter((f) => checked.has(f.key)).map((f) => f.display_name),
        )
        for (const node of tree.value) {
          if (wantNames.has(node.name)) {
            // Ignore the outcome — pinning is a best-effort durability nicety.
            await setRecursivePin(node.id, true)
          }
        }
      }
    }

    // 3. Record the prompt as shown (idempotent) — never auto-show again.
    await markKnownFolderOnboardingSeen()
    onClose(anyEnabled)
  }

  if (!shouldShow || folders == null) return null

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label="Back up your Windows folders"
      style={{
        position: 'fixed' as const,
        inset: 0,
        zIndex: 1000,
        background: 'rgba(24, 20, 10, 0.38)',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        padding: 24,
        fontFamily: T.fontSans,
      }}
    >
      <div
        style={{
          width: '100%',
          maxWidth: 460,
          maxHeight: 'calc(100vh - 48px)',
          display: 'flex',
          flexDirection: 'column',
          background: T.paper,
          border: `1px solid ${T.line}`,
          borderRadius: 14,
          boxShadow: 'var(--shadow-3)',
          overflow: 'hidden',
        }}
      >
        {/* Header */}
        <div style={{ padding: '22px 24px 14px' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 9, marginBottom: 12 }}>
            <span
              style={{
                width: 30,
                height: 30,
                borderRadius: 8,
                background: T.amberBg,
                border: '1px solid oklch(0.86 0.07 90)',
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
              }}
            >
              <NavIcon name="shield" size={15} color={T.amberDeep} />
            </span>
            <span
              style={{
                fontSize: 10,
                fontFamily: T.fontMono,
                textTransform: 'uppercase' as const,
                letterSpacing: '0.06em',
                color: T.ink3,
              }}
            >
              Set up backup
            </span>
          </div>
          <h2 style={{ margin: '0 0 8px', fontSize: 19, fontWeight: 700, letterSpacing: '-0.02em', color: T.ink, lineHeight: 1.2 }}>
            Back up your Windows folders
          </h2>
          <p style={{ margin: 0, fontSize: 12.5, color: T.ink2, lineHeight: 1.6 }}>
            Beebeeb copies the folders you pick into your encrypted vault. Your original folders
            stay exactly where they are, so this works alongside OneDrive — nothing is moved or
            redirected. Backed-up folders are kept on this PC, so they’re always available offline.
          </p>
        </div>

        {/* Folder list */}
        <div style={{ overflowY: 'auto' as const, borderTop: `1px solid ${T.line}`, borderBottom: `1px solid ${T.line}` }}>
          {folders.map((f) => (
            <FolderRow key={f.key} folder={f} checked={checked.has(f.key)} onToggle={() => toggle(f.key)} />
          ))}
        </div>

        {/* Footer */}
        <div style={{ padding: '14px 24px 18px' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginBottom: 14 }}>
            <NavIcon name="lock" size={11} color={T.ink3} />
            <span style={{ fontSize: 10.5, fontFamily: T.fontMono, color: T.ink3 }}>
              Encrypted before it leaves this PC · {regionLabel}
            </span>
          </div>

          <div style={{ display: 'flex', gap: 10, justifyContent: 'flex-end', alignItems: 'center' }}>
            <button
              type="button"
              onClick={() => void skip()}
              disabled={submitting}
              style={{
                padding: '8px 14px',
                fontSize: 12.5,
                fontFamily: T.fontSans,
                fontWeight: 500,
                borderRadius: 7,
                border: `1px solid ${T.line2}`,
                background: 'transparent',
                color: T.ink2,
                cursor: submitting ? 'wait' : 'pointer',
                opacity: submitting ? 0.6 : 1,
              }}
            >
              Not now
            </button>
            {/* The single amber primary action. */}
            <button
              type="button"
              onClick={() => void confirm()}
              disabled={submitting || checkedKeys.length === 0}
              style={{
                padding: '9px 16px',
                fontSize: 12.5,
                fontFamily: T.fontSans,
                fontWeight: 600,
                borderRadius: 7,
                border: `1px solid ${T.amberDeep}`,
                background: T.amber,
                color: T.ink,
                cursor: submitting || checkedKeys.length === 0 ? 'not-allowed' : 'pointer',
                opacity: submitting || checkedKeys.length === 0 ? 0.55 : 1,
                whiteSpace: 'nowrap' as const,
                letterSpacing: '-0.005em',
              }}
            >
              {submitting ? 'Setting up…' : 'Back up these folders'}
            </button>
          </div>
        </div>
      </div>
    </div>
  )
}
