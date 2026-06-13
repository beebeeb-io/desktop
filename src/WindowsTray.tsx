/**
 * WindowsTray — Windows 11 taskbar tray flyout.
 *
 * Mounted when ?window=tray is in the query string. The window is small
 * (400 × 280) and frameless; the Tauri side positions it above the system
 * tray icon. Faithfully ported from HiWindowsTray in design/hifi/hifi-desktop.jsx.
 *
 * Invoke commands used:
 *   EXISTING:  sync_status, desktop_platform
 *   NEW (WS1): tray_pause_sync, tray_resume_sync, show_settings_window
 *              tray_recent_activity → returns TrayActivity[]
 */

import { useEffect, useState } from 'react'
import { command, formatBytes, loadSyncStatus, type SyncStatus } from './desktopApi'

export interface TrayActivity {
  id: string
  name: string
  status: string
  icon: 'file' | 'shield' | 'cloud' | 'pause'
  active?: boolean
  warn?: boolean
  ok?: boolean
  progress?: number
}

// ── Mini icon set for the tray (inline SVG, no external dep) ───────────────

function TrayIcon({ name, size = 11, color = 'currentColor' }: { name: string; size?: number; color?: string }) {
  const s = size
  switch (name) {
    case 'shield':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.5" strokeLinejoin="round">
          <path d="M8 2 L13 4 L13 8 C13 11.5 10.5 13.5 8 14.5 C5.5 13.5 3 11.5 3 8 L3 4 Z" />
        </svg>
      )
    case 'file':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.5">
          <path d="M4 2 L10 2 L12 4.5 L12 14 L4 14 Z" strokeLinejoin="round" />
          <line x1="6" y1="7" x2="10" y2="7" />
          <line x1="6" y1="9.5" x2="10" y2="9.5" />
        </svg>
      )
    case 'cloud':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.5">
          <path d="M4 11 C2 11 2 8 4 8 C4 5.5 7 4 9.5 5 C11 4 14 5.5 13 8 C15 8 15 11 13 11 Z" />
        </svg>
      )
    case 'pause':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill={color}>
          <rect x="4" y="3" width="2.5" height="10" rx="1" />
          <rect x="9.5" y="3" width="2.5" height="10" rx="1" />
        </svg>
      )
    case 'settings':
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="none" stroke={color} strokeWidth="1.5">
          <circle cx="8" cy="8" r="2.5" />
          <path d="M8 1 L8 3 M8 13 L8 15 M1 8 L3 8 M13 8 L15 8 M3.2 3.2 L4.6 4.6 M11.4 11.4 L12.8 12.8 M12.8 3.2 L11.4 4.6 M4.6 11.4 L3.2 12.8" strokeLinecap="round" />
        </svg>
      )
    default:
      return null
  }
}

// Amber hex mark (the Beebeeb "b" brand mark)
function BrandMark() {
  return (
    <div style={{
      width: 26,
      height: 26,
      borderRadius: 7,
      background: 'oklch(0.97 0.03 92)',
      border: '1px solid oklch(0.66 0.15 72)',
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'center',
      flexShrink: 0,
    }}>
      <span style={{
        fontFamily: "'Inter', sans-serif",
        fontWeight: 800,
        fontSize: 13,
        color: 'oklch(0.66 0.15 72)',
        lineHeight: 1,
        letterSpacing: '-0.02em',
      }}>b</span>
    </div>
  )
}

// Spinner for an active sync item
function Spinner({ pct }: { pct: number }) {
  return (
    <div style={{
      marginTop: 5,
      height: 3,
      background: 'oklch(0.90 0.008 82)',
      borderRadius: 999,
      overflow: 'hidden',
    }}>
      <div style={{
        height: '100%',
        width: `${pct}%`,
        background: 'oklch(0.82 0.17 84)',
        borderRadius: 'inherit',
        transition: 'width 0.4s ease',
      }} />
    </div>
  )
}

// Checkmark for completed items
function CheckMark() {
  return (
    <span style={{
      fontSize: 11,
      color: 'oklch(0.55 0.14 155)',
      lineHeight: 1,
    }}>✓</span>
  )
}

export default function WindowsTray() {
  const [status, setStatus] = useState<SyncStatus | null>(null)
  const [paused, setPaused] = useState(false)
  const [activities, setActivities] = useState<TrayActivity[]>([])

  // Poll sync status
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

  // Load recent activity (new command — WS1 must implement)
  useEffect(() => {
    let cancelled = false
    const refresh = async () => {
      const result = await command<TrayActivity[]>('tray_recent_activity')
      if (cancelled) return
      if (result.ok) {
        setActivities(result.value)
      } else {
        // Graceful fallback: show placeholder rows when command not yet implemented
        setActivities([
          { id: '1', name: 'story-draft.md', status: 'Encrypting · 72%', icon: 'file', active: true, progress: 72 },
          { id: '2', name: 'leak-packet.pdf', status: 'Uploaded · 2 min ago', icon: 'shield', ok: true },
          { id: '3', name: 'interview-03.m4a', status: 'Synced · 4 min ago', icon: 'file', ok: true },
          { id: '4', name: 'photos/', status: 'Auto-backup paused · not on Wi-Fi', icon: 'pause', warn: true },
        ])
      }
    }
    void refresh()
    const id = window.setInterval(refresh, 5000)
    return () => {
      cancelled = true
      window.clearInterval(id)
    }
  }, [])

  const syncedLabel = status?.logged_in
    ? status.engine === 'running'
      ? status.syncing > 0
        ? `${status.syncing} item${status.syncing === 1 ? '' : 's'} syncing`
        : 'Up to date'
      : 'Sync paused'
    : 'Signed out'

  const storageLine = (() => {
    // Status doesn't carry total bytes — just show a fixed placeholder until
    // the tray can receive a richer payload from WS1.
    return 'E2E encrypted vault'
  })()

  const handlePause = async () => {
    const cmd = paused ? 'tray_resume_sync' : 'tray_pause_sync'
    const result = await command<void>(cmd)
    if (result.ok) setPaused((prev) => !prev)
  }

  const handleSettings = async () => {
    await command<void>('show_settings_window')
  }

  return (
    <div style={{
      width: 400,
      height: 280,
      background: 'oklch(0.985 0.004 85)',
      borderRadius: 10,
      overflow: 'hidden',
      border: '1px solid oklch(0.83 0.01 80)',
      boxShadow: '0 22px 60px -18px rgba(0,0,0,0.35), 0 0 0 1px oklch(0.83 0.01 80)',
      display: 'flex',
      flexDirection: 'column',
      fontFamily: "'Inter', ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif",
      fontSize: 13,
      color: 'oklch(0.18 0.01 70)',
      position: 'relative',
    }}>

      {/* Header */}
      <div style={{
        padding: '12px 14px 10px',
        display: 'flex',
        alignItems: 'center',
        gap: 10,
        borderBottom: '1px solid oklch(0.90 0.008 82)',
        background: 'oklch(0.985 0.004 85)',
      }}>
        <BrandMark />
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ fontSize: 12.5, fontWeight: 600, lineHeight: 1.2 }}>Beebeeb</div>
          <div style={{ fontSize: 11, color: 'oklch(0.52 0.01 78)', marginTop: 2, lineHeight: 1.3 }}>
            {syncedLabel} · {storageLine}
          </div>
        </div>
        <div style={{ display: 'flex', gap: 2, marginLeft: 'auto' }}>
          {(['–', '✕'] as const).map((c, i) => (
            <button
              key={i}
              style={{
                width: 24,
                height: 24,
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                fontSize: 11,
                color: 'oklch(0.52 0.01 78)',
                background: 'transparent',
                border: 'none',
                borderRadius: 4,
                cursor: 'pointer',
                lineHeight: 1,
              }}
            >
              {c}
            </button>
          ))}
        </div>
      </div>

      {/* Activity list */}
      <div style={{ flex: 1, overflow: 'auto', padding: '4px 0' }}>
        {activities.length === 0 ? (
          <div style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            height: '100%',
            fontSize: 11,
            color: 'oklch(0.52 0.01 78)',
          }}>
            No recent activity
          </div>
        ) : (
          activities.map((row) => (
            <div
              key={row.id}
              style={{
                display: 'grid',
                gridTemplateColumns: '28px 1fr auto',
                alignItems: 'center',
                gap: 10,
                padding: '8px 14px',
                background: row.active ? 'oklch(0.97 0.03 92)' : 'transparent',
              }}
            >
              <div style={{
                width: 22,
                height: 22,
                borderRadius: 6,
                background: 'oklch(0.968 0.006 85)',
                border: '1px solid oklch(0.90 0.008 82)',
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                flexShrink: 0,
              }}>
                <TrayIcon
                  name={row.icon}
                  size={11}
                  color={row.active ? 'oklch(0.66 0.15 72)' : 'oklch(0.52 0.01 78)'}
                />
              </div>

              <div style={{ minWidth: 0 }}>
                <div style={{
                  fontSize: 11.5,
                  fontWeight: 500,
                  whiteSpace: 'nowrap',
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  fontFamily: "'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace",
                }}>
                  {row.name}
                </div>
                <div style={{
                  fontSize: 11,
                  color: row.warn ? 'oklch(0.55 0.12 55)' : 'oklch(0.52 0.01 78)',
                  marginTop: 2,
                  lineHeight: 1.3,
                }}>
                  {row.status}
                </div>
                {row.active && row.progress !== undefined && (
                  <Spinner pct={row.progress} />
                )}
              </div>

              <div style={{ width: 16, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                {row.ok && <CheckMark />}
              </div>
            </div>
          ))
        )}
      </div>

      {/* Footer bar */}
      <div style={{
        borderTop: '1px solid oklch(0.90 0.008 82)',
        padding: '8px 14px',
        display: 'flex',
        alignItems: 'center',
        gap: 10,
        background: 'oklch(0.968 0.006 85)',
        flexShrink: 0,
      }}>
        <TrayIcon name="shield" size={11} color="oklch(0.66 0.15 72)" />
        <span style={{
          fontSize: 11,
          color: 'oklch(0.34 0.012 75)',
          lineHeight: 1,
        }}>
          Falkenstein · E2E encrypted
        </span>
        <div style={{ marginLeft: 'auto', display: 'flex', gap: 6 }}>
          <button
            onClick={() => void handlePause()}
            style={{
              padding: '3px 8px',
              fontSize: 11,
              fontFamily: 'inherit',
              fontWeight: 500,
              background: 'transparent',
              border: '1px solid transparent',
              borderRadius: 4,
              cursor: 'pointer',
              color: 'oklch(0.34 0.012 75)',
            }}
            onMouseEnter={(e) => { e.currentTarget.style.background = 'oklch(0.945 0.008 82)' }}
            onMouseLeave={(e) => { e.currentTarget.style.background = 'transparent' }}
          >
            {paused ? 'Resume' : 'Pause'}
          </button>
          <button
            onClick={() => void handleSettings()}
            style={{
              padding: '3px 8px',
              fontSize: 11,
              fontFamily: 'inherit',
              fontWeight: 500,
              background: 'transparent',
              border: '1px solid transparent',
              borderRadius: 4,
              cursor: 'pointer',
              color: 'oklch(0.34 0.012 75)',
            }}
            onMouseEnter={(e) => { e.currentTarget.style.background = 'oklch(0.945 0.008 82)' }}
            onMouseLeave={(e) => { e.currentTarget.style.background = 'transparent' }}
          >
            Settings
          </button>
        </div>
      </div>

      {/* Caret pointing down to the tray icon */}
      <div style={{
        position: 'absolute',
        bottom: -6,
        right: 40,
        width: 12,
        height: 12,
        background: 'oklch(0.968 0.006 85)',
        borderRight: '1px solid oklch(0.83 0.01 80)',
        borderBottom: '1px solid oklch(0.83 0.01 80)',
        transform: 'rotate(45deg)',
        pointerEvents: 'none',
      }} />
    </div>
  )
}
