/**
 * Files: the sync-folder card, the status buckets and bars, the recent-changes strip, the backup
 * toggle and the known-folder backup card. The two window events it dispatches (re-open backup,
 * backup changed) are named here because this view owns that flow.
 */
import { useEffect, useState } from 'react'
import { command, commandUnavailableLabel, formatBytes, loadSyncStatus, desktopFileOverview, getKnownFolderBackup, setKnownFolderBackup, type KnownFolderStatus, type SyncStatus, type FileOverview, type StatusBucket, type RecentFile } from '../desktopApi'
import { setKnownFolderSource, type KnownFolderSource } from '../knownFolderSource'
import { backUpAgainLabel, type BackupStopFields } from '../backupStop'
import { pollWhileVisible } from '../windows/visiblePoll'
import { T, NavIcon, Chip, PrimaryBtn, Skeleton, PageHeader, Card, useToast } from '../windows/ui'
import { usePlatformName } from '../platform'
import { finderOpenFailedToast } from '../finderSetup'

// FILES — the per-PC SYNC-STATE dashboard. The device lens: how many files live
// on this PC vs online-only vs pinned vs conflicting, the most-recently-changed
// files, and the Open-in-Explorer card (Explorer stays the real file surface on
// Windows). COMPLEMENTS InsightsView, which is the by-TYPE / billing lens — this
// view never duplicates that by-type breakdown.
//
// Data: desktopFileOverview() (local, from the SQLite mirror). All four states
// (loading / error / empty / loaded) handled — pattern copied from InsightsView.

export const FILES_RECENT_LIMIT = 15
export const FILES_DANGER = 'oklch(0.5 0.18 25)'

// Stable display order + human labels for the sync statuses. The DTO emits a
// bucket only for statuses that have files, so the UI owns the legend and treats
// a missing status as zero.
export const STATUS_ORDER: RecentFile['status'][] = [
  'local',
  'cloud_only',
  'downloading',
  'uploading',
  'conflict',
  'error',
]
export const STATUS_LABEL: Record<string, string> = {
  local: 'On this PC',
  cloud_only: 'Online-only',
  downloading: 'Downloading',
  uploading: 'Uploading',
  conflict: 'Conflict',
  error: 'Error',
  moved_to_trash: 'Moved to trash',
}
// Status colors are NEUTRAL/GREEN/RED only — amber is reserved for encryption
// state + primary actions (file-header brand note). Synced/local reads green,
// conflict/error red, everything in-flight or cloud-only stays neutral ink.
export const STATUS_COLOR: Record<string, string> = {
  local: T.green,
  cloud_only: T.ink3,
  downloading: T.ink3,
  uploading: T.ink3,
  conflict: FILES_DANGER,
  error: FILES_DANGER,
  moved_to_trash: T.ink3,
}

export function statusLabel(status: string): string {
  return STATUS_LABEL[status] ?? status
}
export function statusColor(status: string): string {
  return STATUS_COLOR[status] ?? T.ink3
}

// basename of a forward- or back-slash path.
export function baseName(path: string): string {
  const parts = path.split(/[/\\]/)
  return parts[parts.length - 1] || path
}

// Relative time from a Unix-epoch (seconds) timestamp. Coarse on purpose —
// "just now" / "3m ago" / "2h ago" / "5d ago" / a short date for older.
export function relativeTime(epochSecs: number): string {
  if (!Number.isFinite(epochSecs) || epochSecs <= 0) return '—'
  const deltaSecs = Math.max(0, Math.floor(Date.now() / 1000 - epochSecs))
  if (deltaSecs < 45) return 'just now'
  const mins = Math.floor(deltaSecs / 60)
  if (mins < 60) return `${mins}m ago`
  const hours = Math.floor(mins / 60)
  if (hours < 24) return `${hours}h ago`
  const days = Math.floor(hours / 24)
  if (days < 7) return `${days}d ago`
  return new Date(epochSecs * 1000).toLocaleDateString('en-US', { month: 'short', day: 'numeric' })
}

// A small status pill (mirrors InsightsView's category Chip idiom, but colored
// by sync status, never amber). A leading dot carries the status color so the
// pill text can stay legible neutral ink.
export function StatusPill({ status }: { status: string }) {
  return (
    <Chip tone="neutral">
      <span style={{ display: 'inline-block', width: 6, height: 6, borderRadius: 999, background: statusColor(status) }} />
      {statusLabel(status)}
    </Chip>
  )
}

// ── Sub-card: Open-in-Explorer (kept from the previous FilesView) ─────────────

export function SyncFolderCard({ status: polledStatus }: { status: SyncStatus | null }) {
  const [status, setStatus] = useState(polledStatus)
  useEffect(() => setStatus(polledStatus), [polledStatus])
  const root = status?.sync_root ?? null
  const isMacos = usePlatformName() === 'macos'
  const [opening, setOpening] = useState(false)
  const { showToast } = useToast()

  const openInExplorer = async () => {
    setOpening(true)
    // The last poll may predate a root change or removal.
    const current = isMacos ? null : await loadSyncStatus()
    if (!isMacos) {
      setStatus(current)
      if (!current?.sync_root) {
        setOpening(false)
        return
      }
    }
    const r = await command<void>('open_finder_location', isMacos ? undefined : { path: current!.sync_root })
    setOpening(false)
    if (!r.ok) {
      // A Mac's error is a redacted bridge code, never shown: the toast is the one sentence
      // (task 17b). Windows/Linux keep the error text, which names a folder.
      showToast(
        isMacos
          ? finderOpenFailedToast()
          : {
              variant: 'error',
              title: 'Couldn’t open folder',
              message: r.unsupported ? commandUnavailableLabel('open_finder_location') : r.reason,
            },
      )
    }
  }

  return (
    <Card style={{ padding: 18, marginBottom: 14 }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 14 }}>
        <div style={{ width: 36, height: 36, borderRadius: 9, background: T.amberBg, border: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
          <NavIcon name="folder" size={16} color={T.amberDeep} />
        </div>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ fontSize: 13, fontWeight: 600, color: T.ink, marginBottom: 2 }}>Beebeeb sync folder</div>
          <div title={root ?? undefined} style={{ fontSize: 11.5, fontFamily: T.fontMono, color: T.ink3, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' as const }}>
            {status == null ? 'Sync folder unavailable' : isMacos ? 'Beebeeb in Finder' : root ?? 'Not configured on this PC yet'}
          </div>
        </div>
        <PrimaryBtn onClick={() => void openInExplorer()} disabled={opening || status == null || (!isMacos && root == null)}>
          <NavIcon name="external" size={13} color={T.paper} /> {opening ? 'Opening…' : isMacos ? 'Open in Finder' : 'Open in Explorer'}
        </PrimaryBtn>
      </div>
    </Card>
  )
}

// ── Section: metric strip ─────────────────────────────────────────────────────

export function bucketFor(buckets: StatusBucket[], status: string): StatusBucket | undefined {
  return buckets.find((b) => b.status === status)
}

export function FilesMetricStrip({ overview }: { overview: FileOverview }) {
  const local = bucketFor(overview.by_status, 'local')
  const cloud = bucketFor(overview.by_status, 'cloud_only')
  const conflict = bucketFor(overview.by_status, 'conflict')

  const metrics: { label: string; value: string; sub: string; color: string }[] = [
    {
      label: 'On this PC',
      value: (local?.count ?? 0).toLocaleString('en-US'),
      sub: formatBytes(local?.bytes ?? 0),
      color: T.green,
    },
    {
      label: 'Online-only',
      value: (cloud?.count ?? 0).toLocaleString('en-US'),
      sub: formatBytes(cloud?.bytes ?? 0),
      color: T.ink3,
    },
    {
      label: 'Pinned',
      value: overview.pinned_count.toLocaleString('en-US'),
      sub: formatBytes(overview.pinned_bytes),
      color: T.ink2,
    },
    {
      label: 'Conflicts',
      value: (conflict?.count ?? 0).toLocaleString('en-US'),
      sub: conflict && conflict.count > 0 ? 'needs attention' : 'none',
      color: conflict && conflict.count > 0 ? FILES_DANGER : T.ink3,
    },
    {
      label: 'In vault',
      value: overview.total_files.toLocaleString('en-US'),
      sub: formatBytes(overview.total_bytes),
      color: T.ink,
    },
  ]

  return (
    <Card style={{ padding: 0, marginBottom: 14 }}>
      <div style={{ display: 'grid', gridTemplateColumns: `repeat(${metrics.length}, 1fr)` }}>
        {metrics.map((m, i) => (
          <div
            key={m.label}
            style={{
              padding: '16px 18px',
              borderLeft: i === 0 ? 'none' : `1px solid ${T.line}`,
              minWidth: 0,
            }}
          >
            <div style={{ fontSize: 9.5, fontFamily: T.fontMono, textTransform: 'uppercase' as const, letterSpacing: '0.07em', color: T.ink3, marginBottom: 8 }}>
              {m.label}
            </div>
            <div style={{ fontSize: 22, fontWeight: 700, fontFamily: T.fontMono, letterSpacing: '-0.02em', color: m.color, lineHeight: 1.05 }}>
              {m.value}
            </div>
            <div style={{ fontSize: 10.5, fontFamily: T.fontMono, color: T.ink3, marginTop: 4 }}>
              {m.sub}
            </div>
          </div>
        ))}
      </div>
    </Card>
  )
}

// ── Section: by-status proportional bar ──────────────────────────────────────

export function ByStatusBar({ overview }: { overview: FileOverview }) {
  const total = overview.total_files
  // Render in stable order; only statuses with files contribute a segment.
  const segments = STATUS_ORDER.map((status) => {
    const bucket = bucketFor(overview.by_status, status)
    return { status, count: bucket?.count ?? 0 }
  }).filter((s) => s.count > 0)

  return (
    <Card style={{ padding: 22, marginBottom: 14 }}>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 14 }}>
        <div style={{ fontSize: 13, fontWeight: 600, color: T.ink }}>By sync state</div>
        <span style={{ fontSize: 11, fontFamily: T.fontMono, color: T.ink3 }}>
          {total.toLocaleString('en-US')} {total === 1 ? 'file' : 'files'}
        </span>
      </div>

      <div style={{ display: 'flex', width: '100%', height: 10, borderRadius: 999, overflow: 'hidden', background: T.paper3 }}>
        {segments.map((s) => (
          <div
            key={s.status}
            title={`${statusLabel(s.status)} · ${s.count.toLocaleString('en-US')}`}
            style={{
              width: `${total > 0 ? (s.count / total) * 100 : 0}%`,
              height: '100%',
              background: statusColor(s.status),
              transition: 'width 200ms ease',
            }}
          />
        ))}
      </div>

      <div style={{ display: 'flex', flexWrap: 'wrap' as const, gap: '8px 16px', marginTop: 14 }}>
        {segments.map((s) => (
          <div key={s.status} style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
            <span style={{ display: 'inline-block', width: 8, height: 8, borderRadius: 999, background: statusColor(s.status) }} />
            <span style={{ fontSize: 11.5, color: T.ink2 }}>{statusLabel(s.status)}</span>
            <span style={{ fontSize: 11, fontFamily: T.fontMono, color: T.ink3 }}>
              {s.count.toLocaleString('en-US')}
            </span>
          </div>
        ))}
      </div>
    </Card>
  )
}

// ── Section: recently changed ─────────────────────────────────────────────────

export function RecentlyChanged({ files }: { files: RecentFile[] }) {
  return (
    <Card style={{ padding: 0 }}>
      <div style={{ padding: '16px 22px 12px', display: 'flex', alignItems: 'center', justifyContent: 'space-between', borderBottom: `1px solid ${T.line}` }}>
        <div style={{ fontSize: 13, fontWeight: 600, color: T.ink }}>Recently changed</div>
        <span style={{ fontSize: 11, fontFamily: T.fontMono, color: T.ink3 }}>last {files.length}</span>
      </div>
      <div>
        {files.map((f, i) => {
          const activity = f.activity_type === 'moved_to_trash' || f.status === 'moved_to_trash'
          return (
          <div
            key={`${f.status}:${f.path || i}:${f.modified_at}`}
            style={{
              display: 'grid',
              gridTemplateColumns: '1fr auto auto auto',
              gap: 14,
              alignItems: 'center',
              padding: '11px 22px',
              borderBottom: i === files.length - 1 ? 'none' : `1px solid ${T.line}`,
            }}
          >
            <span
              title={f.path}
              style={{
                fontSize: 12,
                fontFamily: T.fontMono,
                color: T.ink2,
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                whiteSpace: 'nowrap' as const,
                minWidth: 0,
                display: 'inline-flex',
                alignItems: 'center',
                gap: 7,
              }}
            >
              {f.pinned && <NavIcon name="lock" size={11} color={T.ink3} />}
              {activity && <NavIcon name="trash" size={11} color={T.ink3} />}
              {baseName(f.path)}
            </span>
            <StatusPill status={f.status} />
            <span style={{ fontSize: 11, fontFamily: T.fontMono, color: T.ink3, textAlign: 'right' as const, minWidth: 56 }}>
              {relativeTime(f.modified_at)}
            </span>
            <span style={{ fontSize: 12, fontFamily: T.fontMono, fontWeight: 600, color: T.ink, textAlign: 'right' as const, minWidth: 56 }}>
              {activity ? '—' : formatBytes(f.size_bytes)}
            </span>
          </div>
        )})}
      </div>
    </Card>
  )
}

// ── States: loading / error / empty (pattern from InsightsView) ───────────────

export function FilesLoadingState() {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
      <Card style={{ padding: 18 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 14 }}>
          <Skeleton width={36} height={36} radius={9} />
          <div style={{ flex: 1 }}>
            <Skeleton width={130} height={12} />
            <div style={{ marginTop: 8 }}><Skeleton width="60%" height={11} /></div>
          </div>
          <Skeleton width={130} height={32} radius={6} />
        </div>
      </Card>
      <Card style={{ padding: 0 }}>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)' }}>
          {[0, 1, 2, 3].map((i) => (
            <div key={i} style={{ padding: '16px 18px', borderLeft: i === 0 ? 'none' : `1px solid ${T.line}` }}>
              <Skeleton width={70} height={10} />
              <div style={{ marginTop: 10 }}><Skeleton width={48} height={20} /></div>
              <div style={{ marginTop: 6 }}><Skeleton width={60} height={10} /></div>
            </div>
          ))}
        </div>
      </Card>
      <Card style={{ padding: 22 }}>
        <div style={{ marginBottom: 14 }}><Skeleton width={100} height={12} /></div>
        <Skeleton height={10} radius={999} />
      </Card>
      <Card style={{ padding: 0 }}>
        <div style={{ padding: '16px 22px 12px', borderBottom: `1px solid ${T.line}` }}>
          <Skeleton width={120} height={12} />
        </div>
        {[0, 1, 2, 3].map((i) => (
          <div
            key={i}
            style={{
              display: 'grid',
              gridTemplateColumns: '1fr auto auto auto',
              gap: 14,
              alignItems: 'center',
              padding: '11px 22px',
              borderBottom: i === 3 ? 'none' : `1px solid ${T.line}`,
            }}
          >
            <Skeleton width={`${64 - i * 8}%`} height={12} />
            <Skeleton width={72} height={16} radius={999} />
            <Skeleton width={44} height={12} />
            <Skeleton width={50} height={12} />
          </div>
        ))}
      </Card>
    </div>
  )
}

export function FilesErrorState({ err, onRetry }: { err: { reason: string; unsupported: boolean }; onRetry: () => void }) {
  if (err.unsupported) {
    return (
      <>
        <Card style={{ padding: 22 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
            <div style={{ width: 36, height: 36, borderRadius: 9, background: T.paper2, border: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
              <NavIcon name="cloud" size={16} color={T.ink3} />
            </div>
            <div style={{ minWidth: 0 }}>
              <div style={{ fontSize: 13, fontWeight: 600, color: T.ink, marginBottom: 2 }}>Sync details not available in this build</div>
              <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.5 }}>
                The per-file sync state needs the desktop sync engine, which this build doesn’t ship yet. Explorer is still your file surface.
              </div>
            </div>
          </div>
        </Card>
      </>
    )
  }
  return (
    <>
      <Card style={{ padding: 22 }}>
        <div style={{ display: 'flex', alignItems: 'flex-start', gap: 12 }}>
          <div style={{ width: 36, height: 36, borderRadius: 9, background: T.paper2, border: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
            <NavIcon name="cloud" size={16} color={FILES_DANGER} />
          </div>
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ fontSize: 13, fontWeight: 600, color: T.ink, marginBottom: 2 }}>Couldn’t load your sync state</div>
            <div style={{ fontSize: 11.5, fontFamily: T.fontMono, color: T.ink3, lineHeight: 1.5, wordBreak: 'break-word' as const }}>
              {err.reason}
            </div>
            <div style={{ marginTop: 14 }}>
              <PrimaryBtn onClick={onRetry}>Retry</PrimaryBtn>
            </div>
          </div>
        </div>
      </Card>
    </>
  )
}

export function FilesEmptyState() {
  return (
    <>
      <Card style={{ padding: 0 }}>
        <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', padding: '48px 36px', gap: 10 }}>
          <NavIcon name="cloud" size={26} color={T.ink4} />
          <div style={{ fontSize: 14, fontWeight: 600, color: T.ink, marginTop: 4 }}>No files on this PC yet</div>
          <div style={{ fontSize: 12, color: T.ink3, textAlign: 'center' as const, lineHeight: 1.6, maxWidth: 360 }}>
            Once files sync to this PC, you’ll see what’s local, what’s online-only, what’s pinned, and anything that needs attention here.
          </div>
        </div>
      </Card>
    </>
  )
}

// ── Known-folder backup onboarding re-open bus (task 0804) ────────────────────
//
// The first-run "Back up these folders?" prompt is mounted ONCE at the app root
// (so its overlay covers the whole window). The Manage-backup panel — rendered
// several layers deep inside FilesView — needs to RE-OPEN that same prompt when
// the user clicks "Set up backup" after having skipped it. Rather than thread a
// callback down through FilesView → ManageBackupCard, we fire a tiny window
// CustomEvent the root listens for. Show-once still governs the AUTOMATIC
// first-run appearance; this manual path bypasses it on explicit request.
export const KF_ONBOARDING_REOPEN_EVENT = 'bb:kf-onboarding-reopen'
// Fired after the onboarding prompt enables one or more folders, so the
// Manage-backup panel (if mounted) re-reads the now-updated backup state.
export const KF_BACKUP_CHANGED_EVENT = 'bb:kf-backup-changed'

export function requestKnownFolderOnboarding() {
  window.dispatchEvent(new CustomEvent(KF_ONBOARDING_REOPEN_EVENT))
}

// ── Section: Manage backup (known-folder backup, task 0797) ───────────────────
//
// OneDrive-style "Manage backup": one-way mirror of the user's Windows known
// folders (Desktop / Documents / Pictures / Music / Videos / Downloads) into the
// vault. Model 2 — the real folders are NOT redirected; we copy their contents
// into the sync root and the engine uploads them. Honest copy, no emojis. The
// toggle is NEUTRAL (ink), not amber — amber is reserved for encryption state.

/** A small on/off switch matching the Windows app's neutral palette. */
export function BackupToggle({
  on,
  busy,
  onChange,
  label,
}: {
  on: boolean
  busy: boolean
  onChange: () => void
  label: string
}) {
  return (
    <button
      role="switch"
      aria-checked={on}
      aria-label={label}
      disabled={busy}
      onClick={onChange}
      style={{
        position: 'relative' as const,
        width: 36,
        height: 20,
        flexShrink: 0,
        borderRadius: 999,
        border: `1px solid ${on ? T.ink : T.line2}`,
        background: on ? T.ink : T.paper2,
        cursor: busy ? 'wait' : 'pointer',
        opacity: busy ? 0.6 : 1,
        padding: 0,
        transition: 'background 120ms ease, border-color 120ms ease',
      }}
    >
      <span
        style={{
          position: 'absolute' as const,
          top: 2,
          left: on ? 18 : 2,
          width: 14,
          height: 14,
          borderRadius: 999,
          background: T.paper,
          boxShadow: T.knobShadow2,
          transition: 'left 120ms ease',
        }}
      />
    </button>
  )
}

/** Status line under each folder name: Backed up / Off / Syncing / unavailable. */
export function backupStatus(f: KnownFolderStatus & Partial<BackupStopFields>, pending: boolean): { text: string; tone: 'on' | 'off' | 'busy' | 'na' } {
  if (f.source_path == null) return { text: 'Not available on this platform', tone: 'na' }
  if (pending) return { text: f.enabled ? 'Starting backup…' : 'Updating…', tone: 'busy' }
  // LD-0139: core's words for a backup that stopped by itself, or for a notices check that failed.
  if (f.enabled && f.check_note) return { text: f.check_note, tone: 'busy' }
  if (!f.enabled && f.stop_words) return { text: f.stop_words, tone: 'off' }
  // LD-0113 Amendment 9: OneDrive moved this folder and the person has not chosen which one to back up yet.
  if (f.enabled && f.question) return { text: 'Waiting for your answer: not backed up yet', tone: 'busy' }
  if (f.enabled) return { text: 'Backed up to your vault', tone: 'on' }
  return { text: 'Not backed up', tone: 'off' }
}

export function ManageBackupCard() {
  const [folders, setFolders] = useState<(KnownFolderStatus & Partial<BackupStopFields>)[] | null>(null)
  const [unsupported, setUnsupported] = useState(false)
  // Persistent load failure (get_known_folder_backup) keeps its inline notice —
  // the card is stuck on skeletons until it loads, so a transient toast would
  // leave no explanation. Per-toggle (set_known_folder_backup) failures toast.
  const [error, setError] = useState<string | null>(null)
  const { showToast } = useToast()
  // Keys with an in-flight set_known_folder_backup call.
  const [pending, setPending] = useState<Set<string>>(new Set())

  const load = async () => {
    const res = await getKnownFolderBackup()
    if (!res.ok) {
      if (res.unsupported) setUnsupported(true)
      else setError(res.reason)
      return
    }
    setFolders(res.value)
  }

  useEffect(() => {
    let cancelled = false
    void (async () => {
      const res = await getKnownFolderBackup()
      if (cancelled) return
      if (!res.ok) {
        if (res.unsupported) setUnsupported(true)
        else setError(res.reason)
        return
      }
      setFolders(res.value)
    })()
    return () => { cancelled = true }
  }, [])

  // Re-read backup state when the onboarding prompt enables folders (task 0804)
  // so the panel reflects the change without a manual refresh.
  useEffect(() => {
    const onChanged = () => { void load() }
    window.addEventListener(KF_BACKUP_CHANGED_EVENT, onChanged)
    return () => window.removeEventListener(KF_BACKUP_CHANGED_EVENT, onChanged)
  }, [])

  // LD-0139: a pass can turn a backup off (core's words, "Back up again") or fail its notice check while the card is
  // open, so it reads again every minute while the window is on screen, and at once when it is shown again after a
  // skipped read (task 2243's poll; the effect above does the first load).
  useEffect(() => pollWhileVisible(() => load(), 60_000, { leading: false }), [])

  const toggle = async (f: KnownFolderStatus) => {
    if (f.source_path == null) return
    const next = !f.enabled
    setPending((p) => new Set(p).add(f.key))
    // Optimistic flip.
    setFolders((prev) => prev?.map((x) => (x.key === f.key ? { ...x, enabled: next } : x)) ?? prev)
    const res = await setKnownFolderBackup(f.key, next)
    setPending((p) => {
      const n = new Set(p)
      n.delete(f.key)
      return n
    })
    if (!res.ok) {
      // Roll back the optimistic flip and surface the error.
      setFolders((prev) => prev?.map((x) => (x.key === f.key ? { ...x, enabled: f.enabled } : x)) ?? prev)
      showToast({
        variant: 'error',
        title: next ? 'Couldn’t start backup' : 'Couldn’t stop backup',
        message: res.unsupported ? commandUnavailableLabel('set_known_folder_backup') : res.reason,
      })
      return
    }
    // Refresh to pick up updated item counts after a mirror pass.
    void load()
  }

  // LD-0113 Amendment 9: the one answer for a folder OneDrive moved.
  const choose = async (f: KnownFolderStatus, choice: KnownFolderSource) => {
    setPending((p) => new Set(p).add(f.key))
    const res = await setKnownFolderSource(f.key, choice)
    setPending((p) => {
      const n = new Set(p)
      n.delete(f.key)
      return n
    })
    if (!res.ok) {
      showToast({
        variant: 'error',
        title: 'Couldn’t save your choice',
        message: res.unsupported ? commandUnavailableLabel('set_known_folder_source') : res.reason,
      })
      return
    }
    void load()
  }

  // Windows-only feature — render nothing on macOS/Linux rather than an empty card.
  if (unsupported) return null
  if (folders != null && folders.every((f) => f.source_path == null)) return null

  // No folders backed up yet → offer the guided "Set up backup" prompt (task
  // 0804). This is how a user who dismissed the first-run prompt with "Not now"
  // can return to it: the affordance re-opens the SAME prompt (pre-checked
  // Desktop/Documents/Pictures). Only shown when at least one folder resolves
  // here AND none are currently backed up.
  const noneBackedUp =
    folders != null && folders.some((f) => f.source_path != null) && folders.every((f) => !f.enabled)

  return (
    <Card style={{ padding: 0, marginBottom: 14 }}>
      <div style={{ padding: '16px 18px 12px', borderBottom: `1px solid ${T.line}` }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12 }}>
          <div style={{ fontSize: 13, fontWeight: 600, color: T.ink }}>Manage backup</div>
          {noneBackedUp ? (
            <button
              type="button"
              onClick={() => requestKnownFolderOnboarding()}
              style={{
                padding: '5px 11px',
                fontSize: 11.5,
                fontFamily: T.fontSans,
                fontWeight: 600,
                borderRadius: 6,
                border: `1px solid ${T.amberDeep}`,
                background: T.amber,
                color: T.ink,
                cursor: 'pointer',
                whiteSpace: 'nowrap' as const,
                letterSpacing: '-0.005em',
              }}
            >
              Set up backup
            </button>
          ) : (
            <Chip tone="neutral">One-way</Chip>
          )}
        </div>
        <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.55, marginTop: 4, maxWidth: 520 }}>
          Back up your Windows folders to your encrypted vault. Files are copied in — your
          original folders stay exactly where they are, so this works alongside OneDrive. Turning a
          folder off stops new backups; what’s already in your vault stays.
        </div>
      </div>

      {folders == null ? (
        <div style={{ padding: 18, display: 'grid', gap: 12 }}>
          <Skeleton height={40} />
          <Skeleton height={40} />
          <Skeleton height={40} />
        </div>
      ) : (
        <div>
          {folders.filter((f) => f.source_path != null).map((f, i) => {
            const isPending = pending.has(f.key)
            const status = backupStatus(f, isPending)
            const statusColor =
              status.tone === 'on' ? T.green : status.tone === 'busy' ? T.ink2 : T.ink3
            const disabled = f.source_path == null
            return (
              <div
                key={f.key}
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 14,
                  padding: '12px 18px',
                  borderTop: i === 0 ? 'none' : `1px solid ${T.line}`,
                  opacity: disabled ? 0.55 : 1,
                }}
              >
                <div style={{ width: 30, height: 30, borderRadius: 8, background: T.paper2, border: `1px solid ${T.line}`, display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
                  <NavIcon name="folder" size={14} color={T.ink3} />
                </div>
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ display: 'flex', alignItems: 'baseline', gap: 8 }}>
                    <span style={{ fontSize: 12.5, fontWeight: 600, color: T.ink }}>{f.display_name}</span>
                    {f.item_count != null && f.item_count > 0 && (
                      <span style={{ fontSize: 10.5, fontFamily: T.fontMono, color: T.ink4 }}>
                        {f.item_count.toLocaleString('en-US')}
                        {f.item_count >= 10000 ? '+' : ''} {f.item_count === 1 ? 'file' : 'files'}
                      </span>
                    )}
                  </div>
                  {f.source_path && (
                    <div
                      title={f.source_path}
                      style={{ fontSize: 11, fontFamily: T.fontMono, color: T.ink4, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' as const, marginTop: 1 }}
                    >
                      {f.source_path}
                    </div>
                  )}
                  <div style={{ fontSize: 11, color: statusColor, marginTop: 2 }}>{status.text}</div>
                  {f.enabled && f.question && (
                    <div style={{ marginTop: 6 }}>
                      <div style={{ fontSize: 11.5, color: T.ink2, lineHeight: 1.5, maxWidth: 520 }}>{f.question}</div>
                      <div style={{ display: 'flex', gap: 8, marginTop: 6 }}>
                        <PrimaryBtn disabled={isPending} onClick={() => void choose(f, 'onedrive')}>
                          {f.moved_by_onedrive ? 'Back up OneDrive’s folder' : 'Back up the moved folder'}
                        </PrimaryBtn>
                        <button
                          type="button"
                          disabled={isPending}
                          onClick={() => void choose(f, 'default')}
                          style={{ padding: '4px 10px', fontSize: 11.5, fontFamily: T.fontSans, fontWeight: 500, borderRadius: 6, border: `1px solid ${T.line}`, background: T.paper, color: T.ink, cursor: 'pointer' }}
                        >
                          Back up the default folder
                        </button>
                      </div>
                    </div>
                  )}
                  {f.note && <div style={{ fontSize: 11, color: T.ink3, marginTop: 2 }}>{f.note}</div>}
                  {backUpAgainLabel(f) && (
                    <div style={{ marginTop: 6 }}>
                      <PrimaryBtn disabled={isPending} onClick={() => void toggle(f)}>{backUpAgainLabel(f)}</PrimaryBtn>
                    </div>
                  )}
                </div>
                <BackupToggle
                  on={f.enabled}
                  busy={isPending}
                  onChange={() => void toggle(f)}
                  label={`${f.enabled ? 'Stop backing up' : 'Back up'} ${f.display_name}`}
                />
              </div>
            )
          })}
        </div>
      )}

      {error && <div style={{ padding: '0 18px 14px', fontSize: 11.5, color: FILES_DANGER }}>{error}</div>}
    </Card>
  )
}

// ── Root: FilesView ───────────────────────────────────────────────────────────

export function FilesView({ status }: { status: SyncStatus | null }) {
  const [loading, setLoading] = useState(true)
  const [err, setErr] = useState<{ reason: string; unsupported: boolean } | null>(null)
  const [overview, setOverview] = useState<FileOverview | null>(null)
  const [reloadKey, setReloadKey] = useState(0)

  useEffect(() => {
    let cancelled = false
    setLoading(true)
    setErr(null)

    void (async () => {
      const res = await desktopFileOverview(FILES_RECENT_LIMIT)
      if (cancelled) return
      if (!res.ok) {
        setErr({ reason: res.reason, unsupported: res.unsupported })
        setLoading(false)
        return
      }
      setOverview(res.value)
      setLoading(false)
    })()

    return () => { cancelled = true }
  }, [reloadKey])

  const retry = () => setReloadKey((k) => k + 1)
  const isEmpty = overview != null && overview.total_files === 0 && overview.total_bytes === 0

  return (
    <div style={{ overflow: 'auto', padding: '28px 36px', flex: 1 }}>
      <PageHeader
        title="Files"
        subtitle="What’s synced to this PC — local vs online-only, anything pinned or in conflict, and your most recent changes. Explorer stays your file surface; this is the device view."
      />

      <SyncFolderCard status={status} />

      {loading ? (
        <FilesLoadingState />
      ) : err ? (
        <FilesErrorState err={err} onRetry={retry} />
      ) : isEmpty ? (
        <>
          {/* Manage backup is useful even before any files exist — it's how the
              user opts their Windows folders INTO the vault in the first place. */}
          <ManageBackupCard />
          <FilesEmptyState />
        </>
      ) : overview ? (
        <>
          <ManageBackupCard />
          <FilesMetricStrip overview={overview} />
          <ByStatusBar overview={overview} />
          {overview.recent.length > 0 && <RecentlyChanged files={overview.recent} />}
        </>
      ) : (
        // Defensive fallback — never white-screen.
        <FilesEmptyState />
      )}
    </div>
  )
}
