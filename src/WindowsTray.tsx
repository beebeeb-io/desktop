/**
 * WindowsTray — Windows 11 taskbar tray flyout (OneDrive-class).
 *
 * Mounted when ?window=tray is in the query string. The window is a frameless,
 * solid-surface, always-on-top, skip-taskbar flyout (see tauri.conf.json) that
 * the Tauri side positions above the system-tray icon (lib.rs on_tray_icon_event).
 *
 * Native behaviour:
 *   • Hides on blur (click-outside dismiss), like OneDrive — getCurrentWindow()
 *     .onFocusChanged → hide() when focus is lost.
 *   • No window-chrome buttons — a native flyout has no titlebar.
 *   • The single visible surface is an on-brand document background; the window
 *     is deliberately opaque so Windows never shows an unstyled transparent
 *     rectangle behind the flyout.
 *
 * Layout (top → bottom):
 *   • Header   — BrandMark + "Beebeeb" + settings gear (→ main app Settings).
 *   • Status   — what sync is doing, from `sync_status` + `sync_activity_summary` (0.8.15): moving, checking,
 *                paused, waiting, on hold, or up to date — never "synced" while the queue holds work.
 *   • Recent   — scrollable list of recently-changed files (desktopRecentFiles).
 *   • Actions  — Open folder · View online · Recycle bin.
 *
 * Data sources:
 *   sync_status              → SyncStatus (logged_in / engine / syncing / conflicts)
 *   sync_activity_summary    → pause flag, engine state, transfers, queue counts, startup check (counts only)
 *   desktop_recent_files(50) → RecentFile[] (path, size_bytes, status, modified_at)
 *   show_main_app_settings   → opens the main app and selects Settings
 *   open_finder_location     → opens the sync root in Explorer
 *   openUrl                  → app.beebeeb.io / app.beebeeb.io/trash
 *
 * Task 2243: the flyout stays loaded while hidden, so its polls read only while it is on screen
 * (pollWhileVisible) and read at once when it is shown again.
 */

import { useEffect, useState } from 'react'
import { emit } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'
import {
  command,
  desktopRecentFiles,
  loadActiveTransfers,
  loadSyncStatus,
  openUrl,
  revealAndOpenFile,
  type ActiveTransferRow,
  type RecentFile,
  type SyncStatus,
} from './desktopApi'
import { traySubline, progressFraction, trayHeaderOverride, buildTransferMap } from './windows/downloadProgressModel'
import { FileGlyph, getFileType, FILE_COLOR } from './windows/fileGlyph'
import { T, useToast } from './windows/ui'
import { syncActivityView } from './windows/syncActivityModel'
import { SyncActivityIcon, SyncCheckProgress, useSyncActivitySummary } from './windows/syncActivityUi'
import { pollWhileVisible } from './windows/visiblePoll'

// FileGlyph, getFileType, FILE_COLOR imported from ./windows/fileGlyph (shared with ActivityView)

// ── Small action-bar / header icons ──────────────────────────────────────────

function ActionIcon({ name, size = 13, color = 'currentColor' }: { name: string; size?: number; color?: string }) {
  const s = size
  const common = { width: s, height: s, viewBox: '0 0 16 16', fill: 'none', stroke: color, strokeWidth: 1.4 }
  switch (name) {
    case 'cog':
      return (
        <svg {...common}>
          <circle cx="8" cy="8" r="2.5" />
          <path
            d="M8 1.5 L8 3.5 M8 12.5 L8 14.5 M1.5 8 L3.5 8 M12.5 8 L14.5 8 M3.5 3.5 L5 5 M11 11 L12.5 12.5 M12.5 3.5 L11 5 M5 11 L3.5 12.5"
            strokeLinecap="round"
          />
        </svg>
      )
    case 'folder':
      return (
        <svg {...common}>
          <path d="M2 5 L2 13 L14 13 L14 6 L7 6 L5.5 4 L2 4 Z" strokeLinejoin="round" />
        </svg>
      )
    case 'external':
      return (
        <svg {...common} strokeLinecap="round" strokeLinejoin="round">
          <path d="M9 2 L14 2 L14 7 M14 2 L7.5 8.5" />
          <path d="M12 9.5 L12 13 L3 13 L3 4 L6.5 4" />
        </svg>
      )
    case 'trash':
      return (
        <svg {...common} strokeLinecap="round" strokeLinejoin="round">
          <path d="M3 4.5 L13 4.5" />
          <path d="M5.5 4.5 L5.5 3 L10.5 3 L10.5 4.5" />
          <path d="M4.2 4.5 L4.8 13.5 L11.2 13.5 L11.8 4.5" />
        </svg>
      )
    default:
      return null
  }
}

// Amber hex mark (the Beebeeb "b" brand mark).
function BrandMark() {
  return (
    <div
      style={{
        width: 26,
        height: 26,
        borderRadius: 7,
        background: 'oklch(0.97 0.03 92)',
        border: '1px solid oklch(0.66 0.15 72)',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        flexShrink: 0,
      }}
    >
      <span
        style={{
          fontFamily: T.fontSans,
          fontWeight: 800,
          fontSize: 13,
          color: 'oklch(0.66 0.15 72)',
          lineHeight: 1,
          letterSpacing: '-0.02em',
        }}
      >
        b
      </span>
    </div>
  )
}

// ── Pure helpers ─────────────────────────────────────────────────────────────

/** Last path segment (handles both `/` and `\` separators). */
function basename(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean)
  return parts.length ? parts[parts.length - 1] : path
}

/** The folder a file lives in — the path segment before the basename. */
function parentFolder(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean)
  return parts.length >= 2 ? parts[parts.length - 2] : 'Beebeeb'
}

/**
 * Honest per-status subline. `<F>` is the parent folder. A file still uploading is not in Beebeeb yet, so it is never
 * shown as synced (spec 1888/2226 D13).
 */
function statusSubline(status: string, folder: string): string {
  switch (status) {
    case 'cloud_only':
      return `Online-only in ${folder}`
    case 'downloading':
      return `Downloading to ${folder}`
    case 'uploading':
      return `Not uploaded yet — ${folder}`
    case 'conflict':
      return `Needs review — ${folder}`
    case 'error':
      return `Sync error — ${folder}`
    case 'moved_to_trash':
      return 'Moved to trash'
    case 'local':
    default:
      return `Synced to ${folder}`
  }
}

/** Relative time from a Unix-epoch-seconds timestamp, e.g. "12 hr ago". */
function relativeTime(epochSeconds: number): string {
  if (!Number.isFinite(epochSeconds) || epochSeconds <= 0) return ''
  const deltaMs = Date.now() - epochSeconds * 1000
  const sec = Math.max(0, Math.round(deltaMs / 1000))
  if (sec < 45) return 'just now'
  const min = Math.round(sec / 60)
  if (min < 60) return `${min} min ago`
  const hr = Math.round(min / 60)
  if (hr < 24) return `${hr} hr ago`
  const day = Math.round(hr / 24)
  if (day < 7) return `${day} day${day === 1 ? '' : 's'} ago`
  const wk = Math.round(day / 7)
  if (wk < 5) return `${wk} wk${wk === 1 ? '' : 's'} ago`
  const mo = Math.round(day / 30)
  if (mo < 12) return `${mo} mo ago`
  const yr = Math.round(day / 365)
  return `${yr} yr${yr === 1 ? '' : 's'} ago`
}

// Sublines that warn (need attention) get a warm/neutral non-amber tint.
const WARN_STATUSES = new Set(['conflict', 'error'])
const TRAY_RECENT_LIMIT = 50
const TRAY_VIEW_ALL_THRESHOLD = 8
const TRAY_RECENT_ROW_HEIGHT = 48
const WINDOWS_APP_NAV_EVENT = 'windows-app:navigate'
const WINDOWS_APP_NAV_STORAGE_KEY = 'bb.windowsApp.nav'
/** `NOT_SENT_TARGET` in `WindowsApp/nav.tsx`: the route and section the main window opens. */
const NOT_SENT_TARGET = 'activity#not-sent'

// ── Component ────────────────────────────────────────────────────────────────

export default function WindowsTray() {
  const { showToast } = useToast()
  const [opening, setOpening] = useState(false)
  const [status, setStatus] = useState<SyncStatus | null>(null)
  const [recent, setRecent] = useState<RecentFile[]>([])
  const [loaded, setLoaded] = useState(false)
  const [signedOut, setSignedOut] = useState(false)
  const [activeTransfers, setActiveTransfers] = useState<ActiveTransferRow[]>([])

  // Hide on blur (click-outside dismiss), like OneDrive. Set up once on mount.
  useEffect(() => {
    const win = getCurrentWindow()
    let unlisten: (() => void) | undefined
    void win
      .onFocusChanged(({ payload: focused }) => {
        if (!focused) void win.hide()
      })
      .then((fn) => {
        unlisten = fn
      })
    return () => {
      unlisten?.()
    }
  }, [])

  // Poll sync status (~3s) while the flyout is on screen.
  useEffect(() => {
    let cancelled = false
    const refresh = async () => {
      const next = await loadSyncStatus()
      if (!cancelled) setStatus(next)
    }
    const stop = pollWhileVisible(refresh, 3000)
    return () => {
      cancelled = true
      stop()
    }
  }, [])

  // Poll active transfers (~3s) for the progress overlay on downloading / streaming rows (LD-0105 U1).
  useEffect(() => {
    let cancelled = false
    const refresh = async () => {
      const result = await loadActiveTransfers()
      if (!cancelled && result.ok) setActiveTransfers(result.value)
    }
    const stop = pollWhileVisible(refresh, 3000)
    return () => {
      cancelled = true
      stop()
    }
  }, [])

  // Poll recent files (~5s) while the flyout is on screen. Task 2243: only the newest rows are read, not the
  // whole overview. A failed read (e.g. vault locked) → empty "Sign in" state, not an error banner.
  useEffect(() => {
    let cancelled = false
    const refresh = async () => {
      const result = await desktopRecentFiles(TRAY_RECENT_LIMIT)
      if (cancelled) return
      if (result.ok) {
        setRecent(result.value)
        setSignedOut(false)
      } else {
        setRecent([])
        setSignedOut(true)
      }
      setLoaded(true)
    }
    const stop = pollWhileVisible(refresh, 5000)
    return () => {
      cancelled = true
      stop()
    }
  }, [])

  // What sync is doing (0.8.15). On Guus's PC 0.8.14 said "Your files are synced" here while 2,924 changes were on
  // hold, 43 had given up and the startup check was walking 37,169 files: it read only rows mid-transfer. The model
  // reads the queue and the startup check too, and is green only when nothing is pending. Never amber.
  const { summary } = useSyncActivitySummary()
  const activity = syncActivityView(status, summary, { device: 'this PC' })

  const openSettings = async () => {
    await command<void>('show_main_app_settings')
    void getCurrentWindow().hide()
  }

  const openFolder = async () => {
    setOpening(true)
    const current = await loadSyncStatus()
    setStatus(current)
    if (!current?.sync_root) {
      setOpening(false)
      return
    }
    const result = await command<void>('open_finder_location', { path: current.sync_root })
    setOpening(false)
    if (result.ok) void getCurrentWindow().hide()
    else showToast({ variant: 'error', title: 'Couldn’t open folder', message: result.reason })
  }

  const viewOnline = async () => {
    await openUrl('https://app.beebeeb.io')
    void getCurrentWindow().hide()
  }

  const openRecycleBin = async () => {
    try {
      window.localStorage.setItem(WINDOWS_APP_NAV_STORAGE_KEY, 'trash')
    } catch {
      // Best effort only; already-open main windows still receive the Tauri
      // event below.
    }
    await command<void>('show_main_app_window')
    await emit(WINDOWS_APP_NAV_EVENT, 'trash').catch(() => undefined)
    void getCurrentWindow().hide()
  }

  // Task 2409: a waiting, held or given-up line opens Activity › Not sent, where those changes are listed one by one.
  const openNotSent = async () => {
    try {
      window.localStorage.setItem(WINDOWS_APP_NAV_STORAGE_KEY, NOT_SENT_TARGET)
    } catch {
      // Best effort only; already-open main windows still receive the Tauri event below.
    }
    await command<void>('show_main_app_window')
    await emit(WINDOWS_APP_NAV_EVENT, NOT_SENT_TARGET).catch(() => undefined)
    void getCurrentWindow().hide()
  }
  /** A status line: a link to Activity › Not sent when it counts changes not sent, plain text otherwise. */
  const statusLine = (line: string) =>
    activity.notSentLines.includes(line) ? (
      <a
        href="#not-sent"
        title="See which changes"
        onClick={(event) => {
          event.preventDefault()
          void openNotSent()
        }}
        style={{ color: 'inherit', textDecoration: 'underline', textUnderlineOffset: 2, textDecorationColor: T.ink4 }}
      >
        {line}
      </a>
    ) : (
      line
    )

  const openActivity = async () => {
    try {
      window.localStorage.setItem(WINDOWS_APP_NAV_STORAGE_KEY, 'activity')
    } catch {
      // Best effort only; already-open main windows still receive the Tauri
      // event below.
    }
    await command<void>('show_main_app_window')
    await emit(WINDOWS_APP_NAV_EVENT, 'activity').catch(() => undefined)
    void getCurrentWindow().hide()
  }

  const showViewAll = recent.length > TRAY_VIEW_ALL_THRESHOLD
  // Map full vault path → active transfer DTO so two files with the same name in
  // different folders each get the right row (P1 fix, LD-0105 U1 review 2026-11-06).
  const transferByPath = buildTransferMap(activeTransfers)
  // When any transfer is in a terminal error state, suppress the "synced" headline
  // so the header doesn't contradict the visible warn row (LD-0105 U1, lead ruling 2026-11-06).
  const headerWarn = trayHeaderOverride(activeTransfers)

  return (
    <div
      style={{
        // Fill the opaque frameless window with the intentional Beebeeb surface.
        position: 'absolute',
        inset: 0,
        background: T.paper,
        border: `1px solid ${T.line}`,
        boxSizing: 'border-box',
        overflow: 'hidden',
        display: 'flex',
        flexDirection: 'column',
        fontFamily: T.fontSans,
        fontSize: 13,
        color: T.ink,
      }}
    >
      {/* Header — brand + settings gear (no window-chrome buttons) */}
      <div
        style={{
          padding: '12px 12px 12px 14px',
          height: 53,
          boxSizing: 'border-box',
          display: 'flex',
          alignItems: 'center',
          gap: 10,
          borderBottom: `1px solid ${T.line}`,
        }}
      >
        <BrandMark />
        <div style={{ flex: 1, minWidth: 0, fontSize: 13.5, fontWeight: 600, letterSpacing: '-0.01em' }}>
          Beebeeb
        </div>
        <button
          title="Settings"
          aria-label="Open settings"
          onClick={() => void openSettings()}
          style={{
            width: 28,
            height: 28,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            color: T.ink3,
            background: 'transparent',
            border: 'none',
            borderRadius: 6,
            cursor: 'pointer',
          }}
          onMouseEnter={(e) => {
            e.currentTarget.style.background = T.paper3
          }}
          onMouseLeave={(e) => {
            e.currentTarget.style.background = 'transparent'
          }}
        >
          <ActionIcon name="cog" size={15} />
        </button>
      </div>

      {/* Status: one headline, then every other pending count, and the startup check's progress while it runs.
          When any transfer is failed or needs attention, `headerWarn` overrides the headline so the header
          never claims "synced" while a warn row is visible (honest over reassuring, LD-0105 U1). */}
      <div
        role="status"
        aria-live="polite"
        style={{
          padding: '10px 14px',
          minHeight: 37,
          boxSizing: 'border-box',
          display: 'flex',
          alignItems: 'flex-start',
          gap: 8,
          borderBottom: `1px solid ${T.line}`,
          flexShrink: 0,
        }}
      >
        {headerWarn ? (
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ fontSize: 12.5, fontWeight: 500, color: 'var(--warn-text)', lineHeight: 1.35 }}>
              {headerWarn}
            </div>
          </div>
        ) : (
          <>
            <span style={{ display: 'flex', paddingTop: 0.5 }}>
              <SyncActivityIcon view={activity} />
            </span>
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ fontSize: 12.5, fontWeight: 500, color: T.ink2, lineHeight: 1.35 }}>{statusLine(activity.headline)}</div>
              {activity.details.map((line) => (
                <div key={line} style={{ fontSize: 11, color: T.ink3, marginTop: 3, lineHeight: 1.35 }}>
                  {statusLine(line)}
                </div>
              ))}
              {activity.progress && <SyncCheckProgress check={activity.progress} compact />}
            </div>
          </>
        )}
      </div>

      <div title={status?.sync_root ?? undefined} style={{ padding: '6px 14px', fontSize: 11, fontFamily: T.fontMono, color: T.ink3, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', flexShrink: 0 }}>
        {status == null ? 'Sync folder unavailable' : status.sync_root ?? 'Not configured on this PC yet'}
      </div>

      {/* Recent files list */}
      <div style={{ flex: 1, overflow: 'auto', minHeight: 0 }}>
        {!loaded ? null : signedOut || recent.length === 0 ? (
          <div
            style={{
              display: 'flex',
              flexDirection: 'column',
              alignItems: 'center',
              justifyContent: 'center',
              height: '100%',
              gap: 8,
              padding: 20,
              textAlign: 'center',
            }}
          >
            <FileGlyph type="default" size={22} color={T.ink4} />
            <span style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.4 }}>
              {signedOut ? 'Sign in to see your recent files' : 'No recent files yet'}
            </span>
          </div>
        ) : (
          recent.map((file) => {
            const name = basename(file.path)
            const folder = parentFolder(file.path)
            const type = getFileType(name)
            const deleted = file.activity_type === 'moved_to_trash' || file.status === 'moved_to_trash'
            const warn = WARN_STATUSES.has(file.status)
            // LD-0105 U1: overlay live progress from the transfer board.
            // Strip any leading slash before lookup (vault paths are always slash-free on
            // the DTO side after parent_folder strips them in Rust).
            const transfer = !deleted ? transferByPath.get(file.path.replace(/^\/+/, '')) : undefined
            const subline = transfer
              ? traySubline(transfer)
              : statusSubline(file.status, folder)
            // needs_attention: failed=false but still shows the warn colour (Amendment 4)
            const sublineWarn = transfer ? (transfer.failed || transfer.attention === 'changed_on_this_device') : warn
            const fraction = transfer ? progressFraction(transfer) : null
            return (
              <button
                key={`${file.status}:${file.path}:${file.modified_at}`}
                title={deleted ? `${name} moved to trash` : `Open ${name}`}
                onClick={() => void (deleted ? openRecycleBin() : revealAndOpenFile(file.path))}
                style={{
                  position: 'relative',
                  display: 'grid',
                  gridTemplateColumns: '30px 1fr auto',
                  alignItems: 'center',
                  gap: 10,
                  height: TRAY_RECENT_ROW_HEIGHT,
                  boxSizing: 'border-box',
                  padding: '9px 14px',
                  width: '100%',
                  background: 'transparent',
                  border: 'none',
                  textAlign: 'left',
                  cursor: 'pointer',
                }}
                onMouseEnter={(e) => {
                  e.currentTarget.style.background = T.paper2
                }}
                onMouseLeave={(e) => {
                  e.currentTarget.style.background = 'transparent'
                }}
              >
                <div
                  style={{
                    width: 28,
                    height: 28,
                    borderRadius: 6,
                    background: T.paper2,
                    border: `1px solid ${T.line}`,
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'center',
                    flexShrink: 0,
                  }}
                >
                  {deleted ? (
                    <ActionIcon name="trash" size={14} color={T.ink3} />
                  ) : (
                    <FileGlyph type={type} size={14} color={FILE_COLOR[type]} />
                  )}
                </div>

                <div style={{ minWidth: 0 }}>
                  <div
                    style={{
                      fontSize: 11.5,
                      lineHeight: 1.15,
                      fontWeight: 500,
                      whiteSpace: 'nowrap',
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                      fontFamily: T.fontMono,
                      color: T.ink,
                    }}
                  >
                    {name}
                  </div>
                  <div
                    style={{
                      fontSize: 11,
                      color: sublineWarn ? 'var(--warn-text)' : T.ink3,
                      marginTop: 2,
                      lineHeight: 1.2,
                      whiteSpace: 'nowrap',
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                    }}
                  >
                    {subline}
                  </div>
                </div>

                <div
                  style={{
                    fontSize: 10.5,
                    color: T.ink3,
                    fontFamily: T.fontMono,
                    whiteSpace: 'nowrap',
                    flexShrink: 0,
                  }}
                >
                  {relativeTime(file.modified_at)}
                </div>

                {/* Progress bar — 3px, sits on the bottom edge of the 48px row (LD-0105 U1).
                    Amber is reserved for encryption state; use neutral ink-3 on paper-3. */}
                {fraction != null && (
                  <div
                    aria-hidden
                    style={{
                      position: 'absolute',
                      bottom: 0,
                      left: 0,
                      right: 0,
                      height: 3,
                      background: T.paper3,
                      overflow: 'hidden',
                    }}
                  >
                    <div
                      style={{
                        width: `${Math.round(fraction * 100)}%`,
                        height: '100%',
                        background: T.ink3,
                        transition: 'width 0.5s linear',
                      }}
                    />
                  </div>
                )}
              </button>
            )
          })
        )}
      </div>

      {showViewAll && (
        <button
          onClick={() => void openActivity()}
          style={{
            flexShrink: 0,
            height: 31,
            boxSizing: 'border-box',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            gap: 10,
            width: '100%',
            padding: '8px 14px',
            border: 'none',
            borderTop: `1px solid ${T.line}`,
            background: T.amberBg,
            color: T.ink2,
            cursor: 'pointer',
            fontFamily: T.fontSans,
            fontSize: 11.5,
            fontWeight: 600,
            textAlign: 'left',
          }}
          onMouseEnter={(e) => {
            e.currentTarget.style.background = 'oklch(0.94 0.06 90)'
          }}
          onMouseLeave={(e) => {
            e.currentTarget.style.background = T.amberBg
          }}
        >
          <span>View all in Beebeeb</span>
          <ActionIcon name="external" size={13} color={T.amberDeep} />
        </button>
      )}

      {/* Action bar — 3 equal ghost buttons */}
      <div
        style={{
          borderTop: `1px solid ${T.line}`,
          height: 52,
          boxSizing: 'border-box',
          display: 'grid',
          gridTemplateColumns: '1fr 1fr 1fr',
          background: T.paper2,
          flexShrink: 0,
        }}
      >
        <ActionButton icon="folder" label="Open folder" onClick={() => void openFolder()} disabled={opening || !status?.sync_root} />
        <ActionButton icon="external" label="View online" onClick={() => void viewOnline()} borderLeft />
        <ActionButton icon="trash" label="Recycle bin" onClick={() => void openRecycleBin()} borderLeft />
      </div>
    </div>
  )
}

function ActionButton({
  icon,
  label,
  onClick,
  borderLeft,
  disabled = false,
}: {
  icon: string
  label: string
  onClick: () => void
  borderLeft?: boolean
  disabled?: boolean
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      style={{
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        gap: 5,
        padding: '10px 4px',
        fontSize: 11,
        fontFamily: T.fontSans,
        fontWeight: 500,
        color: T.ink2,
        background: 'transparent',
        border: 'none',
        borderLeft: borderLeft ? `1px solid ${T.line}` : 'none',
        cursor: disabled ? 'default' : 'pointer',
        opacity: disabled ? 0.5 : 1,
        lineHeight: 1,
      }}
      onMouseEnter={(e) => {
        e.currentTarget.style.background = T.paper3
      }}
      onMouseLeave={(e) => {
        e.currentTarget.style.background = 'transparent'
      }}
    >
      <ActionIcon name={icon} size={15} color={T.ink2} />
      {label}
    </button>
  )
}
