/**
 * SelectiveSyncView — pick which folders live on this PC (PKG-DATA bind for the
 * `selective-sync` nav slot, the final wave-2 view).
 *
 * Self-fetches listVaultFolders() on mount: a NESTED tree of folder VaultItem
 * nodes, each carrying `excluded` (in the online-only set) plus subtree
 * aggregates (size_bytes / file_count / on_disk_bytes). We render an
 * expandable/collapsible tree where each folder has a TRISTATE checkbox:
 *
 *   - CHECKED        (fully synced)  — not excluded AND no descendant excluded.
 *   - UNCHECKED      (online-only)   — this folder is excluded, OR an ancestor is
 *                                      (descendants of an excluded folder read as
 *                                      effectively online-only / disabled).
 *   - INDETERMINATE  (mixed)         — not excluded itself, but ≥1 descendant
 *                                      folder is excluded.
 *
 * State model: a local working `Set<string>` of excluded folder ids, initialised
 * from the tree's `excluded` flags. Toggling a CHECKED folder ADDS its id;
 * toggling an excluded one REMOVES it. We derive each node's tristate from the
 * working set + the tree on every render — we do NOT auto-prune redundant
 * descendant exclusions (keep it simple + correct), but excluding a parent makes
 * its whole subtree render as online-only.
 *
 * Footer: on-disk total (synced subtrees) + online-only total (excluded
 * subtrees), both mono, plus a primary "Apply" that is disabled until the
 * working set differs from the original. Apply persists via setSelectiveSync(),
 * shows a pending state, and on success reloads the tree (excluding frees disk,
 * so the aggregates change).
 *
 * Brand: amber ONLY for the encryption header chip + the primary Apply button.
 * Folder icons, checkboxes, and the "Online-only" chip are NEUTRAL (excluding a
 * folder is not an encryption-state signal). JetBrains Mono (neutral) for every
 * machine value (sizes, counts); Inter for human copy. No emojis. Honest voice.
 *
 * Design grounding: design/hifi/hifi-desktop.jsx → HiSelectiveSync.
 * Sibling idiom: DevicesView.tsx + InsightsView.tsx + BandwidthView.tsx.
 */

import { useCallback, useEffect, useRef, useState } from 'react'
import { listVaultFolders, setSelectiveSync, formatBytes, type VaultItem } from '../../desktopApi'
import { T, Card, PageHeader, Chip, Skeleton, NavIcon, PrimaryBtn, useToast } from '../ui'
import { useRegionLabel } from '../useRegion'

const RED = 'oklch(0.5 0.18 25)'

/**
 * How often the open view silently re-fetches the folder tree.
 *
 * The engine's `sync_tick` (runner.rs, `TICK_INTERVAL = 5s`) re-walks the
 * server tree every ~5s and inserts any brand-new folder into the local
 * state DB (engine_bridge.rs `sync_tick` case (1): "New to us — insert as
 * cloud_only"). `list_vault_folders` reads that DB FRESH on every call, so a
 * folder created during the session lands in state.db within ~5s — the only
 * thing missing was the view re-asking for it. We poll at 10s (2× the engine
 * tick) so a new folder shows up within ~10-15s without the view ever needing
 * to be reopened.
 *
 * This poll is a LOCAL SQLite read via IPC — it does NOT hit the API, so it
 * adds ZERO new 429/rate-limit pressure (the 0789 re-walk throttle lives in
 * the engine tick, which runs at its own cadence regardless of this view).
 * Even so we keep it modest (10s, within the ~10-15s target band) — it's still
 * an IPC round-trip per tick, and there's no value re-reading the local tree
 * faster than the engine writes to it. Guarded so an in-progress edit/Apply is
 * never clobbered (see `refresh`).
 */
const POLL_INTERVAL_MS = 10_000

// ── Tree helpers ──────────────────────────────────────────────────────────────

/** Collect every folder id whose `excluded` flag is set, across the whole tree —
 *  the original (server) online-only set the working set is diffed against. */
function collectExcludedIds(nodes: VaultItem[], into: Set<string> = new Set()): Set<string> {
  for (const node of nodes) {
    if (node.excluded) into.add(node.id)
    if (node.children.length > 0) collectExcludedIds(node.children, into)
  }
  return into
}

/** Does any descendant (not self) of `node` sit in the excluded working set? */
function hasExcludedDescendant(node: VaultItem, excluded: Set<string>): boolean {
  for (const child of node.children) {
    if (excluded.has(child.id)) return true
    if (hasExcludedDescendant(child, excluded)) return true
  }
  return false
}

type TriState = 'checked' | 'unchecked' | 'indeterminate'

/**
 * Derive a node's tristate from the working set + whether an ancestor is excluded.
 * - ancestorExcluded → the node is effectively online-only (unchecked, disabled).
 * - in the set       → online-only (unchecked).
 * - has an excluded descendant → indeterminate (mixed).
 * - otherwise        → checked (fully synced).
 */
function triStateFor(node: VaultItem, excluded: Set<string>, ancestorExcluded: boolean): TriState {
  if (ancestorExcluded || excluded.has(node.id)) return 'unchecked'
  if (hasExcludedDescendant(node, excluded)) return 'indeterminate'
  return 'checked'
}

/**
 * Sum the on-disk bytes that will remain (synced) and the size that becomes
 * online-only, WITHOUT double-counting nested subtrees. `size_bytes` and
 * `on_disk_bytes` are subtree aggregates, so for the grand on-disk total we sum
 * only the ROOTS, then subtract the on-disk bytes that sit inside an excluded
 * (about-to-be-dehydrated) subtree. Online-only size is each excluded subtree's
 * `size_bytes`, counted only at its SHALLOWEST excluded point so a nested
 * exclusion under an already-excluded ancestor isn't added twice.
 */
function computeTotals(roots: VaultItem[], excluded: Set<string>): { onDisk: number; onlineOnly: number } {
  let onlineOnly = 0
  // On-disk bytes freed because they sit inside an excluded subtree.
  let excludedOnDisk = 0
  let totalOnDisk = 0

  const walk = (node: VaultItem, ancestorExcluded: boolean) => {
    // Count a subtree's online-only size + freed disk at its shallowest excluded
    // point only (ancestorExcluded means it's already inside a counted subtree).
    if (!ancestorExcluded && excluded.has(node.id)) {
      onlineOnly += node.size_bytes
      excludedOnDisk += node.on_disk_bytes
    }
    const isExcluded = ancestorExcluded || excluded.has(node.id)
    for (const child of node.children) walk(child, isExcluded)
  }

  for (const root of roots) {
    totalOnDisk += root.on_disk_bytes
    walk(root, false)
  }

  return { onDisk: Math.max(0, totalOnDisk - excludedOnDisk), onlineOnly }
}

/** Working set === original set? (used to disable Apply). */
function setsEqual(a: Set<string>, b: Set<string>): boolean {
  if (a.size !== b.size) return false
  for (const id of a) if (!b.has(id)) return false
  return true
}

// ── Tristate checkbox ─────────────────────────────────────────────────────────

function TriCheckbox({ state, disabled }: { state: TriState; disabled: boolean }) {
  const checked = state === 'checked'
  const indeterminate = state === 'indeterminate'
  const filled = checked || indeterminate
  return (
    <span
      aria-hidden
      style={{
        width: 15,
        height: 15,
        borderRadius: 4,
        border: '1.5px solid',
        borderColor: filled ? T.ink : T.line2,
        background: filled ? T.ink : T.paper,
        color: T.paper,
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        flexShrink: 0,
        opacity: disabled ? 0.5 : 1,
        transition: 'background 100ms, border-color 100ms',
      }}
    >
      {checked && (
        <svg width="9" height="9" viewBox="0 0 12 12" fill="none" stroke={T.paper} strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
          <path d="M2.5 6.5 L5 9 L9.5 3.5" />
        </svg>
      )}
      {indeterminate && (
        <span style={{ width: 7, height: 2, borderRadius: 1, background: T.paper, display: 'block' }} />
      )}
    </span>
  )
}

// ── Folder row ────────────────────────────────────────────────────────────────

function FolderRow({
  node,
  depth,
  excluded,
  ancestorExcluded,
  expanded,
  onToggleExpand,
  onToggleSync,
  last,
}: {
  node: VaultItem
  depth: number
  excluded: Set<string>
  ancestorExcluded: boolean
  expanded: Set<string>
  onToggleExpand: (id: string) => void
  onToggleSync: (node: VaultItem) => void
  last: boolean
}) {
  const tri = triStateFor(node, excluded, ancestorExcluded)
  const isOnlineOnly = tri === 'unchecked'
  // A folder under an excluded ancestor can't be individually toggled — it
  // follows its parent. Its own checkbox is disabled.
  const checkboxDisabled = ancestorExcluded
  const hasChildren = node.children.length > 0
  const isOpen = expanded.has(node.id)

  return (
    <div
      style={{
        display: 'grid',
        gridTemplateColumns: '16px 15px 16px 1fr auto',
        alignItems: 'center',
        gap: 8,
        padding: '8px 14px',
        paddingLeft: 14 + depth * 18,
        borderBottom: last ? 'none' : `1px solid ${T.line}`,
        background: isOnlineOnly ? T.paper2 : T.paper,
        opacity: isOnlineOnly ? 0.72 : 1,
      }}
    >
      {/* Chevron (only when there are children) */}
      {hasChildren ? (
        <button
          onClick={() => onToggleExpand(node.id)}
          aria-label={isOpen ? 'Collapse' : 'Expand'}
          style={{
            background: 'transparent',
            border: 'none',
            padding: 0,
            margin: 0,
            cursor: 'pointer',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            color: T.ink3,
            width: 16,
            height: 16,
          }}
        >
          <svg
            width="10"
            height="10"
            viewBox="0 0 12 12"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.6"
            strokeLinecap="round"
            strokeLinejoin="round"
            style={{ transform: isOpen ? 'rotate(90deg)' : 'none', transition: 'transform 120ms' }}
          >
            <path d="M4 2.5 L8 6 L4 9.5" />
          </svg>
        </button>
      ) : (
        <span style={{ width: 16, height: 16, display: 'block' }} />
      )}

      {/* Tristate checkbox */}
      <button
        onClick={() => !checkboxDisabled && onToggleSync(node)}
        disabled={checkboxDisabled}
        aria-label={
          tri === 'checked' ? 'Synced — switch to online-only'
          : tri === 'indeterminate' ? 'Some subfolders are online-only — sync all'
          : 'Online-only — sync to this PC'
        }
        style={{
          background: 'transparent',
          border: 'none',
          padding: 0,
          margin: 0,
          cursor: checkboxDisabled ? 'not-allowed' : 'pointer',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
        }}
      >
        <TriCheckbox state={tri} disabled={checkboxDisabled} />
      </button>

      {/* Folder icon — NEUTRAL (not amber); dimmer when online-only */}
      <NavIcon name="folder" size={13} color={isOnlineOnly ? T.ink4 : T.ink3} />

      {/* Name + online-only chip */}
      <span
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          minWidth: 0,
        }}
      >
        <span
          style={{
            fontSize: 12.5,
            fontWeight: depth === 0 ? 600 : 400,
            color: isOnlineOnly ? T.ink3 : T.ink,
            fontFamily: T.fontSans,
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap' as const,
          }}
          title={node.name}
        >
          {node.name}
        </span>
        {isOnlineOnly && <Chip tone="neutral">Online-only</Chip>}
      </span>

      {/* Machine values: size + file count, mono + neutral */}
      <span
        style={{
          display: 'flex',
          alignItems: 'baseline',
          gap: 10,
          fontFamily: T.fontMono,
          whiteSpace: 'nowrap' as const,
          justifySelf: 'end',
        }}
      >
        <span style={{ fontSize: 11, color: T.ink3 }}>{formatBytes(node.size_bytes)}</span>
        <span style={{ fontSize: 10.5, color: T.ink4 }}>
          {node.file_count.toLocaleString('en-US')} {node.file_count === 1 ? 'file' : 'files'}
        </span>
      </span>
    </div>
  )
}

// ── Tree (flattened to honour expand/collapse, with row borders) ──────────────

interface FlatRow {
  node: VaultItem
  depth: number
  ancestorExcluded: boolean
}

/** Flatten the tree into the visible rows (skipping the children of collapsed
 *  folders), carrying each row's depth + whether an ancestor is excluded. */
function flatten(
  nodes: VaultItem[],
  expanded: Set<string>,
  excluded: Set<string>,
  depth = 0,
  ancestorExcluded = false,
  into: FlatRow[] = [],
): FlatRow[] {
  for (const node of nodes) {
    into.push({ node, depth, ancestorExcluded })
    const selfExcluded = ancestorExcluded || excluded.has(node.id)
    if (node.children.length > 0 && expanded.has(node.id)) {
      flatten(node.children, expanded, excluded, depth + 1, selfExcluded, into)
    }
  }
  return into
}

function FolderTree({
  roots,
  excluded,
  expanded,
  onToggleExpand,
  onToggleSync,
}: {
  roots: VaultItem[]
  excluded: Set<string>
  expanded: Set<string>
  onToggleExpand: (id: string) => void
  onToggleSync: (node: VaultItem) => void
}) {
  const rows = flatten(roots, expanded, excluded)
  return (
    <Card style={{ overflow: 'hidden', padding: 0 }}>
      <div style={{ maxHeight: 420, overflow: 'auto' }}>
        {rows.map((row, i) => (
          <FolderRow
            key={row.node.id}
            node={row.node}
            depth={row.depth}
            excluded={excluded}
            ancestorExcluded={row.ancestorExcluded}
            expanded={expanded}
            onToggleExpand={onToggleExpand}
            onToggleSync={onToggleSync}
            last={i === rows.length - 1}
          />
        ))}
      </div>
    </Card>
  )
}

// ── Footer (totals + Apply) ───────────────────────────────────────────────────

function Footer({
  onDisk,
  onlineOnly,
  dirty,
  applying,
  onApply,
}: {
  onDisk: number
  onlineOnly: number
  dirty: boolean
  applying: boolean
  onApply: () => void
}) {
  return (
    <div style={{ marginTop: 16 }}>
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 16,
          flexWrap: 'wrap' as const,
        }}
      >
        <div style={{ fontSize: 11.5, fontFamily: T.fontMono, color: T.ink3 }}>
          On disk <strong style={{ color: T.ink, fontWeight: 600 }}>{formatBytes(onDisk)}</strong>
          <span style={{ color: T.ink4, margin: '0 8px' }}>·</span>
          Online-only <strong style={{ color: T.ink, fontWeight: 600 }}>{formatBytes(onlineOnly)}</strong>
        </div>
        <div style={{ marginLeft: 'auto' }}>
          <PrimaryBtn onClick={onApply} disabled={!dirty || applying}>
            {applying ? 'Applying…' : 'Apply'}
          </PrimaryBtn>
        </div>
      </div>

      <div style={{ fontSize: 11, color: T.ink3, lineHeight: 1.5, marginTop: 12, maxWidth: 560 }}>
        Files already on this PC in folders you switch to online-only will be removed from local
        disk (they stay safe in your vault).
      </div>

    </div>
  )
}

// ── States: loading / error / empty ───────────────────────────────────────────

function LoadingState() {
  return (
    <Card style={{ overflow: 'hidden', padding: 0 }}>
      {Array.from({ length: 6 }).map((_, i) => {
        const depth = i === 0 ? 0 : i % 3 === 0 ? 1 : i % 3 === 1 ? 1 : 2
        return (
          <div
            key={i}
            style={{
              display: 'grid',
              gridTemplateColumns: '16px 15px 16px 1fr auto',
              alignItems: 'center',
              gap: 8,
              padding: '8px 14px',
              paddingLeft: 14 + depth * 18,
              borderBottom: i === 5 ? 'none' : `1px solid ${T.line}`,
            }}
          >
            <Skeleton width={10} height={10} radius={2} />
            <Skeleton width={15} height={15} radius={4} />
            <Skeleton width={13} height={13} radius={3} />
            <Skeleton width={`${52 - depth * 8}%`} height={11} />
            <span style={{ justifySelf: 'end', display: 'flex', gap: 10 }}>
              <Skeleton width={52} height={10} />
              <Skeleton width={40} height={10} />
            </span>
          </div>
        )
      })}
    </Card>
  )
}

function ErrorState({
  unsupported,
  reason,
  onRetry,
}: {
  unsupported: boolean
  reason: string
  onRetry: () => void
}) {
  if (unsupported) {
    return (
      <Card
        style={{
          padding: '28px 24px',
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          gap: 8,
          textAlign: 'center' as const,
        }}
      >
        <NavIcon name="folder" size={22} color={T.ink4} />
        <div style={{ fontSize: 14, fontWeight: 600, color: T.ink, marginTop: 4 }}>
          Not available in this build
        </div>
        <div style={{ fontSize: 12, color: T.ink3, lineHeight: 1.6, maxWidth: 340 }}>
          Selective sync isn't wired into this desktop build yet. It will appear here once the sync
          engine reports your vault's folder tree.
        </div>
      </Card>
    )
  }
  return (
    <Card
      style={{
        padding: '24px',
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        gap: 10,
        textAlign: 'center' as const,
      }}
    >
      <NavIcon name="folder" size={22} color={RED} />
      <div style={{ fontSize: 14, fontWeight: 600, color: T.ink, marginTop: 4 }}>
        Couldn't load your folders
      </div>
      <div
        style={{
          fontSize: 11.5,
          fontFamily: T.fontMono,
          color: T.ink3,
          lineHeight: 1.5,
          maxWidth: 380,
          wordBreak: 'break-word' as const,
        }}
      >
        {reason}
      </div>
      <div style={{ marginTop: 4 }}>
        <PrimaryBtn onClick={onRetry}>Retry</PrimaryBtn>
      </div>
    </Card>
  )
}

function EmptyState() {
  return (
    <Card
      style={{
        padding: '32px 24px',
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        gap: 8,
        textAlign: 'center' as const,
      }}
    >
      <div
        style={{
          width: 46,
          height: 46,
          borderRadius: 11,
          background: T.paper2,
          border: `1px solid ${T.line}`,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
        }}
      >
        <NavIcon name="folder" size={20} color={T.ink3} />
      </div>
      <div style={{ fontSize: 14, fontWeight: 600, color: T.ink, marginTop: 4 }}>
        No folders in your vault yet
      </div>
      <div style={{ fontSize: 12, color: T.ink3, lineHeight: 1.6, maxWidth: 340 }}>
        Once you add folders to your vault, you can choose which ones live on this PC and which stay
        online-only.
      </div>
    </Card>
  )
}

// ── Async state ───────────────────────────────────────────────────────────────

type LoadState =
  | { phase: 'loading' }
  | { phase: 'error'; unsupported: boolean; reason: string }
  | { phase: 'ready'; roots: VaultItem[]; original: Set<string> }

// ── Refresh affordance ────────────────────────────────────────────────────────

/**
 * Small neutral "Refresh" button shown in the header. Re-fetches the folder
 * tree on demand so a folder created moments ago appears without waiting for
 * the next silent poll. NEUTRAL styling (refreshing is not an encryption-state
 * signal — amber is reserved for the lock chip + Apply). Disabled while an edit
 * is pending (a refresh would discard unsaved toggles) and spins its icon while
 * a fetch is in flight.
 */
function RefreshButton({
  refreshing,
  disabled,
  onClick,
}: {
  refreshing: boolean
  disabled: boolean
  onClick: () => void
}) {
  const off = disabled && !refreshing
  return (
    <button
      onClick={onClick}
      disabled={disabled || refreshing}
      aria-label="Refresh folder list"
      title={
        disabled
          ? 'Apply or discard your changes before refreshing'
          : 'Refresh — show folders added since you opened this view'
      }
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 6,
        padding: '6px 11px',
        fontSize: 11.5,
        fontFamily: T.fontSans,
        fontWeight: 500,
        borderRadius: 6,
        border: `1px solid ${T.line2}`,
        background: T.paper,
        color: off ? T.ink4 : T.ink2,
        cursor: disabled || refreshing ? 'not-allowed' : 'pointer',
        opacity: off ? 0.6 : 1,
        whiteSpace: 'nowrap' as const,
      }}
    >
      <svg
        width="12"
        height="12"
        viewBox="0 0 16 16"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
        strokeLinejoin="round"
        style={
          refreshing
            ? { animation: 'bb-spin 0.8s linear infinite', transformOrigin: 'center' }
            : undefined
        }
      >
        <path d="M13.5 8a5.5 5.5 0 1 1-1.6-3.9" />
        <path d="M13.5 2.5 V5 H11" />
      </svg>
      {refreshing ? 'Refreshing…' : 'Refresh'}
    </button>
  )
}

// ── Root view ─────────────────────────────────────────────────────────────────

export default function SelectiveSyncView() {
  const regionLabel = useRegionLabel()
  const { showToast } = useToast()
  const [state, setState] = useState<LoadState>({ phase: 'loading' })
  // Working excluded set + which folders are expanded. Re-seeded on every load.
  const [working, setWorking] = useState<Set<string>>(new Set())
  const [expanded, setExpanded] = useState<Set<string>>(new Set())
  const [applying, setApplying] = useState(false)
  // Latched true while a silent background refresh is in flight, so the header
  // chip can show "Refreshing…" without disturbing the tree.
  const [refreshing, setRefreshing] = useState(false)

  // The current dirty/applying state, mirrored into a ref so the interval
  // callback can read it without re-subscribing the effect every render.
  const dirtyRef = useRef(false)
  const applyingRef = useRef(false)

  // Hard reload: resets to the skeleton and re-seeds working + original from the
  // server tree. Used on first mount and on the Retry path. NOT used by the
  // silent poll (which must never flash a skeleton or clobber a live edit).
  const load = () => {
    setState({ phase: 'loading' })
    let cancelled = false
    void (async () => {
      const result = await listVaultFolders()
      if (cancelled) return
      if (!result.ok) {
        setState({ phase: 'error', unsupported: result.unsupported, reason: result.reason })
        return
      }
      const roots = Array.isArray(result.value) ? result.value : []
      const original = collectExcludedIds(roots)
      setWorking(new Set(original))
      // Default-expand the top level so the tree reads as a tree, not a flat list.
      setExpanded(new Set(roots.map((r) => r.id)))
      setState({ phase: 'ready', roots, original })
    })()
    return () => {
      cancelled = true
    }
  }

  // Silent refresh: re-fetch the tree WITHOUT flashing a skeleton. This is what
  // surfaces a folder created during the session — the engine puts it in
  // state.db within one sync tick (~5s) and this re-asks for it.
  //
  // Edit-safety: if the user has an in-progress edit (dirty) or an Apply is
  // running, we MUST NOT yank the tree / re-seed `original` out from under them
  // (that would silently discard their pending toggles or move the Apply
  // baseline). In that case the refresh is skipped this round and retried on
  // the next poll once they've applied or reverted. When clean, we fully
  // re-seed `working` + `original` from the fresh tree so any new folder shows
  // up with its correct server-default exclusion state.
  //
  // Returns a no-op even on error/empty: stale data is kept (never downgrade a
  // populated tree to an error/empty on a transient poll miss).
  const refresh = useCallback(async () => {
    // Don't run a second refresh on top of one already in flight, and never
    // disturb a live edit or an in-progress Apply.
    if (refreshing || dirtyRef.current || applyingRef.current) return
    setRefreshing(true)
    try {
      const result = await listVaultFolders()
      if (!result.ok || !Array.isArray(result.value)) return // keep stale data
      // Re-check guards: the user may have started editing while we awaited.
      if (dirtyRef.current || applyingRef.current) return
      const roots = result.value
      const original = collectExcludedIds(roots)
      setState((prev) => {
        // Only replace a ready tree; never resurrect from loading/error here
        // (that path belongs to `load`).
        if (prev.phase !== 'ready') return prev
        return { phase: 'ready', roots, original }
      })
      setWorking(new Set(original))
      // Auto-expand any NEW top-level root so a freshly-created folder is
      // visible without the user hunting for a collapsed chevron.
      setExpanded((prev) => {
        const next = new Set(prev)
        for (const r of roots) if (!prev.has(r.id)) next.add(r.id)
        return next
      })
    } finally {
      setRefreshing(false)
    }
  }, [refreshing])

  useEffect(() => {
    const cancel = load()
    const id = window.setInterval(() => {
      void refresh()
    }, POLL_INTERVAL_MS)
    return () => {
      cancel?.()
      window.clearInterval(id)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const toggleExpand = (id: string) => {
    setExpanded((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }

  // Toggle a folder's sync state: excluded → synced (remove), synced → excluded
  // (add). Ancestor-excluded folders are not toggleable (handled by the row).
  const toggleSync = (node: VaultItem) => {
    setWorking((prev) => {
      const next = new Set(prev)
      if (next.has(node.id)) next.delete(node.id)
      else next.add(node.id)
      return next
    })
  }

  const apply = () => {
    if (state.phase !== 'ready') return
    setApplying(true)
    void (async () => {
      const result = await setSelectiveSync([...working])
      if (!result.ok) {
        setApplying(false)
        showToast({
          variant: 'error',
          title: 'Couldn’t apply selective sync',
          message: result.unsupported
            ? "Selective sync can't be changed in this build yet."
            : result.reason,
        })
        return
      }
      // Success — reload the tree (excluding frees disk, so aggregates change).
      setApplying(false)
      load()
    })()
  }

  const dirty = state.phase === 'ready' && !setsEqual(working, state.original)
  const totals =
    state.phase === 'ready' ? computeTotals(state.roots, working) : { onDisk: 0, onlineOnly: 0 }

  // Mirror dirty/applying into refs the silent poll reads, so a background
  // refresh never overwrites an in-progress edit or an Apply baseline.
  useEffect(() => {
    dirtyRef.current = dirty
  }, [dirty])
  useEffect(() => {
    applyingRef.current = applying
  }, [applying])

  return (
    <div style={{ overflow: 'auto', padding: '28px 36px', flex: 1 }}>
      <PageHeader
        title="Selective sync"
        subtitle="Choose what to keep on this PC. Folders you switch to online-only stay in your vault but free up local disk — they download again the moment you open them."
        aside={
          <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
            {state.phase === 'ready' && (
              <RefreshButton
                refreshing={refreshing}
                // A manual refresh while dirty would discard the user's pending
                // toggles — so it's disabled until they Apply or revert. The
                // header copy tells them why.
                disabled={dirty || applying}
                onClick={() => void refresh()}
              />
            )}
            <Chip tone="amber">
              <NavIcon name="lock" size={10} color="oklch(0.4 0.08 72)" /> {regionLabel}
            </Chip>
          </div>
        }
      />

      {state.phase === 'loading' && <LoadingState />}

      {state.phase === 'error' && (
        <ErrorState unsupported={state.unsupported} reason={state.reason} onRetry={() => load()} />
      )}

      {state.phase === 'ready' && state.roots.length === 0 && <EmptyState />}

      {state.phase === 'ready' && state.roots.length > 0 && (
        <>
          <FolderTree
            roots={state.roots}
            excluded={working}
            expanded={expanded}
            onToggleExpand={toggleExpand}
            onToggleSync={toggleSync}
          />
          <Footer
            onDisk={totals.onDisk}
            onlineOnly={totals.onlineOnly}
            dirty={dirty}
            applying={applying}
            onApply={apply}
          />
        </>
      )}
    </div>
  )
}
