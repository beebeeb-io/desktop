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
 *   • Status   — green check + a sync-state line derived from `sync_status`.
 *   • Recent   — scrollable list of recently-changed files (desktopFileOverview).
 *   • Actions  — Open folder · View online · Recycle bin.
 *
 * Data sources:
 *   sync_status              → SyncStatus (logged_in / engine / syncing / conflicts)
 *   desktop_file_overview(50) → FileOverview.recent[] (path, size_bytes, status, modified_at)
 *   show_main_app_settings   → opens the main app and selects Settings
 *   open_finder_location     → opens the sync root in Explorer
 *   openUrl                  → app.beebeeb.io / app.beebeeb.io/trash
 */

import { useEffect, useState } from 'react'
import { emit } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'
import {
  command,
  desktopFileOverview,
  loadSyncStatus,
  openUrl,
  revealAndOpenFile,
  type RecentFile,
  type SyncStatus,
} from './desktopApi'
import { T } from './windows/ui'

// ── File-type classification (ported from repos/web file-icon.tsx getFileType,
//    pure, no @beebeeb/shared import) ─────────────────────────────────────────

type TrayFileType = 'pdf' | 'image' | 'video' | 'audio' | 'code' | 'archive' | 'default'

function getFileType(name: string): TrayFileType {
  const ext = name.split('.').pop()?.toLowerCase() ?? ''

  if (ext === 'pdf') return 'pdf'

  if (
    ['jpg', 'jpeg', 'png', 'gif', 'heic', 'heif', 'webp', 'svg', 'ico', 'bmp', 'tiff', 'tif', 'avif', 'dng', 'cr2', 'cr3', 'nef', 'arw', 'orf', 'rw2', 'raf'].includes(ext)
  )
    return 'image'
  if (['mp4', 'mov', 'avi', 'mkv', 'webm', 'flv', 'wmv', 'm4v', 'hevc'].includes(ext)) return 'video'
  if (['mp3', 'wav', 'flac', 'aac', 'ogg', 'wma', 'm4a', 'aiff', 'opus'].includes(ext)) return 'audio'

  if (
    ['js', 'jsx', 'ts', 'tsx', 'py', 'rs', 'go', 'rb', 'java', 'kt', 'swift', 'c', 'cpp', 'h', 'hpp', 'cs', 'php', 'sh', 'bash', 'zsh', 'lua', 'r', 'scala', 'zig', 'asm', 'sql', 'graphql', 'proto'].includes(ext)
  )
    return 'code'
  if (['html', 'htm', 'css', 'scss', 'sass', 'less', 'vue', 'svelte', 'astro'].includes(ext)) return 'code'

  if (['zip', 'tar', 'gz', 'rar', '7z', 'bz2', 'xz', 'zst', 'lz', 'dmg', 'iso'].includes(ext)) return 'archive'

  return 'default'
}

// Per-type tint for the file glyph. Neutral by default; amber is reserved for
// encryption state only (brand rule), so these are non-amber category hues.
const FILE_COLOR: Record<TrayFileType, string> = {
  pdf: '#dc2626',
  image: '#0d9488',
  video: '#9333ea',
  audio: '#db2777',
  code: '#0f766e',
  archive: '#92400e',
  default: T.ink3,
}

// ── Inline SVG file glyphs (no external dep) ─────────────────────────────────

function FileGlyph({ type, size = 13, color }: { type: TrayFileType; size?: number; color: string }) {
  const s = size
  const common = { width: s, height: s, viewBox: '0 0 16 16', fill: 'none', stroke: color, strokeWidth: 1.4 }
  switch (type) {
    case 'pdf':
      return (
        <svg {...common}>
          <path d="M4 2 L10 2 L12 4.5 L12 14 L4 14 Z" strokeLinejoin="round" />
          <path d="M6 8 L10 8 M6 10.3 L9 10.3" strokeLinecap="round" />
        </svg>
      )
    case 'image':
      return (
        <svg {...common}>
          <rect x="2.5" y="3" width="11" height="10" rx="1.5" />
          <circle cx="6" cy="6.5" r="1.1" fill={color} stroke="none" />
          <path d="M3 12 L6.5 8.5 L9 11 L11 9 L13.5 12" strokeLinejoin="round" />
        </svg>
      )
    case 'video':
      return (
        <svg {...common}>
          <rect x="2" y="4" width="9" height="8" rx="1.5" />
          <path d="M11 7 L14 5 L14 11 L11 9 Z" strokeLinejoin="round" />
        </svg>
      )
    case 'audio':
      return (
        <svg {...common}>
          <path d="M6 11 L6 4 L12 2.5 L12 9.5" strokeLinejoin="round" />
          <circle cx="4.5" cy="11.5" r="1.6" />
          <circle cx="10.5" cy="9.8" r="1.6" />
        </svg>
      )
    case 'code':
      return (
        <svg {...common} strokeLinecap="round" strokeLinejoin="round">
          <path d="M6 5 L3 8 L6 11" />
          <path d="M10 5 L13 8 L10 11" />
        </svg>
      )
    case 'archive':
      return (
        <svg {...common}>
          <path d="M4 2 L12 2 L12 14 L4 14 Z" strokeLinejoin="round" />
          <path d="M8 2 L8 4 M8 5.5 L8 7.5" strokeLinecap="round" />
          <rect x="7" y="8" width="2" height="2.4" rx="0.4" />
        </svg>
      )
    default:
      return (
        <svg {...common}>
          <path d="M4 2 L10 2 L12 4.5 L12 14 L4 14 Z" strokeLinejoin="round" />
          <path d="M6 7.5 L10 7.5 M6 10 L10 10" strokeLinecap="round" />
        </svg>
      )
  }
}

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

/** Honest per-status subline. `<F>` is the parent folder. */
function statusSubline(status: string, folder: string): string {
  switch (status) {
    case 'cloud_only':
      return `Online-only in ${folder}`
    case 'downloading':
      return `Downloading to ${folder}`
    case 'conflict':
      return `Needs review — ${folder}`
    case 'error':
      return `Sync error — ${folder}`
    case 'local':
    case 'uploading':
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
const WINDOWS_APP_NAV_EVENT = 'windows-app:navigate'
const WINDOWS_APP_NAV_STORAGE_KEY = 'bb.windowsApp.nav'

// ── Component ────────────────────────────────────────────────────────────────

export default function WindowsTray() {
  const [status, setStatus] = useState<SyncStatus | null>(null)
  const [recent, setRecent] = useState<RecentFile[]>([])
  const [loaded, setLoaded] = useState(false)
  const [signedOut, setSignedOut] = useState(false)

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

  // Poll sync status (~3s).
  useEffect(() => {
    let cancelled = false
    const refresh = async () => {
      const next = await loadSyncStatus()
      if (!cancelled) setStatus(next)
    }
    void refresh()
    const id = window.setInterval(refresh, 3000)
    return () => {
      cancelled = true
      window.clearInterval(id)
    }
  }, [])

  // Poll recent files (~5s). A failed overview (e.g. vault locked) → empty
  // "Sign in" state, not an error banner.
  useEffect(() => {
    let cancelled = false
    const refresh = async () => {
      const result = await desktopFileOverview(TRAY_RECENT_LIMIT)
      if (cancelled) return
      if (result.ok) {
        setRecent(result.value.recent ?? [])
        setSignedOut(false)
      } else {
        setRecent([])
        setSignedOut(true)
      }
      setLoaded(true)
    }
    void refresh()
    const id = window.setInterval(refresh, 5000)
    return () => {
      cancelled = true
      window.clearInterval(id)
    }
  }, [])

  // Status line — green for "synced"; honest phrasing for in-flight / paused /
  // conflicts / signed out. Amber is encryption-only, so this never uses amber.
  const statusLine: string = (() => {
    if (!status?.logged_in) return 'Sign in to start syncing'
    if (status.conflicts > 0) return `${status.conflicts} conflict${status.conflicts === 1 ? '' : 's'} to review`
    if (status.syncing > 0) return `${status.syncing} file${status.syncing === 1 ? '' : 's'} syncing`
    if (status.engine !== 'running') return 'Sync paused'
    return 'Your files are synced'
  })()
  const statusIsGood = !!status?.logged_in && status.conflicts === 0 && status.engine === 'running'

  const openSettings = async () => {
    await command<void>('show_main_app_settings')
    void getCurrentWindow().hide()
  }

  const openFolder = async () => {
    await command<void>('open_finder_location')
    void getCurrentWindow().hide()
  }

  const viewOnline = async () => {
    await openUrl('https://app.beebeeb.io')
    void getCurrentWindow().hide()
  }

  const openRecycleBin = async () => {
    await openUrl('https://app.beebeeb.io/trash')
    void getCurrentWindow().hide()
  }

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

  return (
    <div
      style={{
        // Fill the opaque frameless window with the intentional Beebeeb surface.
        position: 'absolute',
        inset: 0,
        background: T.paper,
        border: `1px solid ${T.line}`,
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

      {/* Status line */}
      <div
        style={{
          padding: '10px 14px',
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          borderBottom: `1px solid ${T.line}`,
        }}
      >
        <span
          aria-hidden
          style={{
            width: 16,
            height: 16,
            borderRadius: 999,
            background: statusIsGood ? T.green : T.paper3,
            color: statusIsGood ? 'white' : T.ink3,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            flexShrink: 0,
          }}
        >
          {statusIsGood ? (
            <svg width="10" height="10" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
              <path d="M3.5 8.5 L6.5 11.5 L12.5 4.5" />
            </svg>
          ) : (
            <svg width="9" height="9" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
              <path d="M8 4 L8 9" />
              <circle cx="8" cy="12" r="0.6" fill="currentColor" stroke="none" />
            </svg>
          )}
        </span>
        <span style={{ fontSize: 12.5, fontWeight: 500, color: T.ink2 }}>{statusLine}</span>
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
            const warn = WARN_STATUSES.has(file.status)
            return (
              <button
                key={file.path}
                title={`Open ${name}`}
                onClick={() => void revealAndOpenFile(file.path)}
                style={{
                  display: 'grid',
                  gridTemplateColumns: '30px 1fr auto',
                  alignItems: 'center',
                  gap: 10,
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
                  <FileGlyph type={type} size={14} color={FILE_COLOR[type]} />
                </div>

                <div style={{ minWidth: 0 }}>
                  <div
                    style={{
                      fontSize: 11.5,
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
                      color: warn ? 'oklch(0.55 0.12 55)' : T.ink3,
                      marginTop: 2,
                      lineHeight: 1.3,
                      whiteSpace: 'nowrap',
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                    }}
                  >
                    {statusSubline(file.status, folder)}
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
          display: 'grid',
          gridTemplateColumns: '1fr 1fr 1fr',
          background: T.paper2,
          flexShrink: 0,
        }}
      >
        <ActionButton icon="folder" label="Open folder" onClick={() => void openFolder()} />
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
}: {
  icon: string
  label: string
  onClick: () => void
  borderLeft?: boolean
}) {
  return (
    <button
      onClick={onClick}
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
        cursor: 'pointer',
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
