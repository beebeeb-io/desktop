/**
 * Beebeeb desktop — first-launch onboarding flow.
 *
 * Three sequential steps:
 *   1. Login   — email + password → desktop_login IPC → session stored in Rust
 *   2. Folder  — native folder picker via pick_sync_root IPC
 *   3. Sync    — progress bar polling sync_status until files are indexed
 *
 * Rendered when `?window=onboarding` is present in the URL (set by
 * `open_onboarding_window` in lib.rs when no sync_root is configured at
 * first launch). Once the user completes all three steps the window closes
 * and the main settings window becomes the primary UI.
 *
 * Design tokens: paper (#FAF8F5), amber (#FBBF24 / oklch 0.82 0.17 84),
 * amber-deep (#92400E), ink (#1C1A17). Inter for text, JetBrains Mono for
 * paths/hashes (none shown here — paths are prose).
 */

import { useState, useEffect, useCallback } from 'react'
import { invoke, isTauri } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'

// ── Types ─────────────────────────────────────────────────────────────────────

type Step = 'login' | 'folder' | 'sync'

interface SyncStatus {
  logged_in: boolean
  engine: string
  sync_root: string | null
  syncing: number
  cloud_only: number
  conflicts: number
}

// ── Design constants ──────────────────────────────────────────────────────────

const C = {
  paper: '#FAF8F5',
  paperCard: '#F4F1EB',
  paperBorder: '#E8E4DC',
  amber: '#FBBF24',
  amberDeep: '#92400E',
  amberBg: 'rgba(251,191,36,0.12)',
  ink: '#1C1A17',
  ink2: '#3D3A34',
  ink3: '#6B6860',
  ink4: '#9B9890',
  green: '#16A34A',
  red: '#DC2626',
  redBg: '#FEE2E2',
  redBorder: '#FECACA',
} as const

// ── Root component ────────────────────────────────────────────────────────────

export default function Onboarding() {
  const [step, setStep] = useState<Step>('login')

  return (
    <div
      style={{
        minHeight: '100vh',
        background: C.paper,
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        fontFamily: 'Inter, system-ui, sans-serif',
        padding: '32px 24px',
      }}
    >
      {/* Logo + wordmark */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 10,
          marginBottom: 36,
        }}
      >
        <div
          style={{
            width: 32,
            height: 32,
            borderRadius: 8,
            background: C.amber,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
          }}
        >
          <svg width="18" height="18" viewBox="0 0 18 18" fill="none">
            <rect x="3" y="3" width="5" height="5" rx="1.5" fill={C.amberDeep} />
            <rect x="10" y="3" width="5" height="5" rx="1.5" fill={C.amberDeep} />
            <rect x="3" y="10" width="5" height="5" rx="1.5" fill={C.amberDeep} />
            <rect x="10" y="10" width="5" height="5" rx="1.5" fill={C.amberDeep} opacity="0.5" />
          </svg>
        </div>
        <span style={{ fontWeight: 700, fontSize: 18, color: C.ink, letterSpacing: '-0.02em' }}>
          Beebeeb
        </span>
      </div>

      {/* Step indicator */}
      <StepIndicator current={step} />

      {/* Step content */}
      <div
        style={{
          width: '100%',
          maxWidth: 440,
          marginTop: 28,
        }}
      >
        {step === 'login' && <LoginStep onDone={() => setStep('folder')} />}
        {step === 'folder' && <FolderStep onDone={() => setStep('sync')} />}
        {step === 'sync' && <SyncStep />}
      </div>

      {/* Footer */}
      <p
        style={{
          marginTop: 40,
          fontSize: 11,
          color: C.ink4,
          textAlign: 'center',
        }}
      >
        Stored in Falkenstein. Hetzner. Encrypted before it leaves your device.
      </p>
    </div>
  )
}

// ── Step indicator ────────────────────────────────────────────────────────────

function StepIndicator({ current }: { current: Step }) {
  const steps: Array<{ id: Step; label: string }> = [
    { id: 'login', label: 'Sign in' },
    { id: 'folder', label: 'Sync folder' },
    { id: 'sync', label: 'First sync' },
  ]
  const idx = steps.findIndex((s) => s.id === current)

  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 0 }}>
      {steps.map((s, i) => {
        const done = i < idx
        const active = i === idx
        return (
          <div key={s.id} style={{ display: 'flex', alignItems: 'center' }}>
            <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 4 }}>
              <div
                style={{
                  width: 28,
                  height: 28,
                  borderRadius: '50%',
                  background: done ? C.green : active ? C.amber : C.paperCard,
                  border: `2px solid ${done ? C.green : active ? C.amber : C.paperBorder}`,
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                  transition: 'all 0.2s',
                }}
              >
                {done ? (
                  <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
                    <path d="M2 6l3 3 5-5" stroke="#fff" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
                  </svg>
                ) : (
                  <span
                    style={{
                      fontSize: 11,
                      fontWeight: 600,
                      color: active ? C.amberDeep : C.ink3,
                    }}
                  >
                    {i + 1}
                  </span>
                )}
              </div>
              <span
                style={{
                  fontSize: 10,
                  fontWeight: active ? 600 : 400,
                  color: active ? C.ink2 : C.ink4,
                  whiteSpace: 'nowrap',
                }}
              >
                {s.label}
              </span>
            </div>
            {i < steps.length - 1 && (
              <div
                style={{
                  width: 56,
                  height: 2,
                  background: done ? C.green : C.paperBorder,
                  margin: '0 4px',
                  marginBottom: 20,
                  transition: 'background 0.2s',
                }}
              />
            )}
          </div>
        )
      })}
    </div>
  )
}

// ── Card wrapper ──────────────────────────────────────────────────────────────

function Card({
  title,
  subtitle,
  children,
}: {
  title: string
  subtitle: string
  children: React.ReactNode
}) {
  return (
    <div
      style={{
        background: C.paperCard,
        border: `1px solid ${C.paperBorder}`,
        borderRadius: 12,
        padding: 28,
      }}
    >
      <h2
        style={{
          margin: '0 0 4px',
          fontSize: 20,
          fontWeight: 700,
          color: C.ink,
          letterSpacing: '-0.02em',
        }}
      >
        {title}
      </h2>
      <p style={{ margin: '0 0 24px', fontSize: 13, color: C.ink3, lineHeight: 1.5 }}>
        {subtitle}
      </p>
      {children}
    </div>
  )
}

// ── Shared UI primitives ──────────────────────────────────────────────────────

function Field({
  label,
  type,
  value,
  onChange,
  placeholder,
  disabled,
}: {
  label: string
  type: string
  value: string
  onChange: (v: string) => void
  placeholder?: string
  disabled?: boolean
}) {
  return (
    <label style={{ display: 'block', marginBottom: 16 }}>
      <span
        style={{
          display: 'block',
          fontSize: 12,
          fontWeight: 500,
          color: C.ink2,
          marginBottom: 6,
        }}
      >
        {label}
      </span>
      <input
        type={type}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        disabled={disabled}
        style={{
          width: '100%',
          boxSizing: 'border-box',
          padding: '9px 12px',
          fontSize: 14,
          color: C.ink,
          background: disabled ? C.paper : '#fff',
          border: `1px solid ${C.paperBorder}`,
          borderRadius: 8,
          outline: 'none',
          fontFamily: 'inherit',
          opacity: disabled ? 0.7 : 1,
          transition: 'border-color 0.15s',
        }}
        onFocus={(e) => {
          e.currentTarget.style.borderColor = C.amber
        }}
        onBlur={(e) => {
          e.currentTarget.style.borderColor = C.paperBorder
        }}
      />
    </label>
  )
}

function PrimaryButton({
  children,
  onClick,
  disabled,
  loading,
}: {
  children: React.ReactNode
  onClick?: () => void
  disabled?: boolean
  loading?: boolean
  type?: 'submit' | 'button'
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled || loading}
      type="submit"
      style={{
        width: '100%',
        padding: '11px 16px',
        background: disabled || loading ? C.paperBorder : C.amber,
        color: disabled || loading ? C.ink4 : C.amberDeep,
        border: 'none',
        borderRadius: 8,
        fontSize: 14,
        fontWeight: 600,
        cursor: disabled || loading ? 'not-allowed' : 'pointer',
        fontFamily: 'inherit',
        transition: 'all 0.15s',
      }}
    >
      {loading ? 'Loading…' : children}
    </button>
  )
}

function ErrorBox({ message }: { message: string }) {
  return (
    <div
      style={{
        background: C.redBg,
        border: `1px solid ${C.redBorder}`,
        borderRadius: 8,
        padding: '10px 14px',
        fontSize: 13,
        color: C.red,
        marginBottom: 16,
        lineHeight: 1.5,
      }}
    >
      {message}
    </div>
  )
}

// ── Step 1: Login ─────────────────────────────────────────────────────────────

function LoginStep({ onDone }: { onDone: () => void }) {
  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault()
    if (!email || !password) return

    setBusy(true)
    setError(null)

    try {
      await invoke('desktop_login', { email, password })
      onDone()
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err)
      // Surface friendly messages for common auth errors.
      if (msg.includes('401') || msg.includes('Unauthorized') || msg.includes('Invalid')) {
        setError('Incorrect email or password. Please try again.')
      } else if (msg.includes('network') || msg.includes('connect')) {
        setError('Could not reach Beebeeb servers. Check your internet connection.')
      } else {
        setError(msg)
      }
    } finally {
      setBusy(false)
    }
  }

  return (
    <Card
      title="Sign in to Beebeeb"
      subtitle="Your files sync automatically after you sign in. Nothing leaves your device unencrypted."
    >
      {error && <ErrorBox message={error} />}
      <form onSubmit={handleSubmit}>
        <Field
          label="Email"
          type="email"
          value={email}
          onChange={setEmail}
          placeholder="you@example.com"
          disabled={busy}
        />
        <Field
          label="Password"
          type="password"
          value={password}
          onChange={setPassword}
          placeholder=""
          disabled={busy}
        />
        <PrimaryButton disabled={!email || !password} loading={busy}>
          Sign in
        </PrimaryButton>
      </form>
      <p style={{ marginTop: 16, fontSize: 12, color: C.ink4, textAlign: 'center' }}>
        No account?{' '}
        <a
          href="https://app.beebeeb.io/signup"
          target="_blank"
          rel="noopener noreferrer"
          style={{ color: C.amberDeep, textDecoration: 'none', fontWeight: 500 }}
        >
          Create one at app.beebeeb.io
        </a>
      </p>
    </Card>
  )
}

// ── Step 2: Folder ────────────────────────────────────────────────────────────

function FolderStep({ onDone }: { onDone: () => void }) {
  const [chosenPath, setChosenPath] = useState<string | null>(null)
  const [defaultPath, setDefaultPath] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    // Pre-fill the suggested default so the user can see it before picking
    invoke<string>('default_sync_root')
      .then(setDefaultPath)
      .catch(() => {})
  }, [])

  const handleBrowse = useCallback(async () => {
    setBusy(true)
    setError(null)
    try {
      const path = await invoke<string | null>('pick_sync_root')
      if (path) {
        setChosenPath(path)
      }
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusy(false)
    }
  }, [])

  const handleContinue = useCallback(async () => {
    if (chosenPath) {
      // pick_sync_root already persisted + started the engine
      onDone()
      return
    }
    // User hasn't browsed — accept the default
    if (!defaultPath) return
    setBusy(true)
    setError(null)
    try {
      // pick_sync_root opens a dialog pointed at defaultPath; if user accepts
      // it clicks once through. We can also manually call ensure_sync_root
      // but pick_sync_root covers persistence + engine start.
      const path = await invoke<string | null>('pick_sync_root')
      if (path) {
        setChosenPath(path)
        onDone()
      }
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusy(false)
    }
  }, [chosenPath, defaultPath, onDone])

  const displayPath = chosenPath ?? defaultPath ?? '~/Beebeeb'

  return (
    <Card
      title="Choose your sync folder"
      subtitle="Files in this folder sync with your Beebeeb vault. Encrypted before they leave your device."
    >
      {error && <ErrorBox message={error} />}

      {/* Path display */}
      <div
        style={{
          background: C.paper,
          border: `1px solid ${C.paperBorder}`,
          borderRadius: 8,
          padding: '10px 14px',
          fontSize: 13,
          color: chosenPath ? C.ink : C.ink4,
          marginBottom: 12,
          wordBreak: 'break-all',
          fontFamily: chosenPath ? 'JetBrains Mono, monospace' : 'inherit',
        }}
      >
        {displayPath}
      </div>

      <div style={{ display: 'flex', gap: 8, marginBottom: 20 }}>
        <button
          onClick={handleBrowse}
          disabled={busy}
          style={{
            flex: 1,
            padding: '9px 14px',
            background: '#fff',
            color: C.ink2,
            border: `1px solid ${C.paperBorder}`,
            borderRadius: 8,
            fontSize: 13,
            fontWeight: 500,
            cursor: busy ? 'not-allowed' : 'pointer',
            fontFamily: 'inherit',
          }}
        >
          Browse…
        </button>
      </div>

      {!chosenPath && (
        <p style={{ fontSize: 12, color: C.ink4, marginBottom: 16, lineHeight: 1.5 }}>
          We'll create{' '}
          <span style={{ fontFamily: 'JetBrains Mono, monospace' }}>{displayPath}</span> if it
          doesn't exist yet. You can change this later in Settings.
        </p>
      )}

      {chosenPath && (
        <div
          style={{
            background: C.amberBg,
            border: `1px solid ${C.amber}`,
            borderRadius: 8,
            padding: '10px 14px',
            fontSize: 12,
            color: C.amberDeep,
            marginBottom: 16,
            lineHeight: 1.5,
          }}
        >
          Folder selected. Your files will sync here automatically.
        </div>
      )}

      <PrimaryButton
        onClick={handleContinue}
        disabled={busy}
        loading={busy}
      >
        {chosenPath ? 'Start syncing' : 'Use default folder'}
      </PrimaryButton>
    </Card>
  )
}

// ── Step 3: Sync ──────────────────────────────────────────────────────────────

function SyncStep() {
  const [status, setStatus] = useState<SyncStatus | null>(null)
  const [done, setDone] = useState(false)
  const [elapsed, setElapsed] = useState(0)

  // Poll sync_status every 1.5 s
  useEffect(() => {
    const poll = () =>
      invoke<SyncStatus>('sync_status')
        .then((s) => {
          setStatus(s)
          // "Done" when engine is running and nothing is syncing
          if (s.engine === 'running' && s.syncing === 0 && elapsed > 3) {
            setDone(true)
          }
        })
        .catch(console.warn)

    poll()
    const id = setInterval(poll, 1500)
    return () => clearInterval(id)
  }, [elapsed])

  // Elapsed timer for minimum display time
  useEffect(() => {
    const id = setInterval(() => setElapsed((n) => n + 1), 1000)
    return () => clearInterval(id)
  }, [])

  const engineRunning = status?.engine === 'running'
  const syncing = status?.syncing ?? 0

  // Progress is synthetic — we don't know total file count from the status endpoint.
  // Show indeterminate animation while syncing, fill to 100% when done.
  const progressPct = done ? 100 : engineRunning ? Math.min(95, elapsed * 5) : 0

  const handleFinish = async () => {
    // Close the onboarding window — settings window will handle itself
    try {
      const win = await getCurrentWindow()
      await win.close()
    } catch {
      // If window close fails (e.g. prevented_close hook), show settings anyway
      await invoke('show_settings').catch(() => {})
    }
  }

  return (
    <Card
      title={done ? 'All set.' : 'Setting up your vault…'}
      subtitle={
        done
          ? 'Your files are syncing. Beebeeb runs in your menu bar.'
          : engineRunning
            ? syncing > 0
              ? `Indexing ${syncing} file${syncing === 1 ? '' : 's'}…`
              : 'Scanning your sync folder…'
            : 'Starting sync engine…'
      }
    >
      {/* Progress bar */}
      <div
        style={{
          background: C.paperBorder,
          borderRadius: 99,
          height: 6,
          marginBottom: 20,
          overflow: 'hidden',
        }}
      >
        <div
          style={{
            height: '100%',
            background: done ? C.green : C.amber,
            borderRadius: 99,
            width: `${progressPct}%`,
            transition: done ? 'width 0.6s ease' : 'width 1.5s linear',
          }}
        />
      </div>

      {/* Status pills */}
      <div
        style={{
          display: 'flex',
          gap: 8,
          marginBottom: 20,
          flexWrap: 'wrap',
        }}
      >
        <Pill
          label="Engine"
          value={engineRunning ? 'Running' : 'Starting…'}
          ok={engineRunning}
        />
        {status?.sync_root && (
          <Pill label="Folder" value="Linked" ok />
        )}
        {done && <Pill label="Vault" value="Ready" ok />}
      </div>

      {done ? (
        <button
          onClick={handleFinish}
          style={{
            width: '100%',
            padding: '11px 16px',
            background: C.amber,
            color: C.amberDeep,
            border: 'none',
            borderRadius: 8,
            fontSize: 14,
            fontWeight: 600,
            cursor: 'pointer',
            fontFamily: 'inherit',
          }}
        >
          Open Beebeeb
        </button>
      ) : (
        <p style={{ fontSize: 12, color: C.ink4, lineHeight: 1.5, margin: 0 }}>
          This takes a few seconds on first launch. You can close this window — sync
          continues in the background.
        </p>
      )}
    </Card>
  )
}

function Pill({ label, value, ok }: { label: string; value: string; ok: boolean }) {
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 5,
        background: ok ? C.amberBg : C.paperCard,
        border: `1px solid ${ok ? C.amber : C.paperBorder}`,
        borderRadius: 99,
        padding: '3px 10px',
        fontSize: 11,
        color: ok ? C.amberDeep : C.ink4,
        fontWeight: 500,
      }}
    >
      <span
        style={{
          width: 5,
          height: 5,
          borderRadius: '50%',
          background: ok ? C.green : C.ink4,
        }}
      />
      <span style={{ color: C.ink3 }}>{label}:</span>
      <span>{value}</span>
    </div>
  )
}
