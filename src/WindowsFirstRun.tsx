/**
 * WindowsFirstRun — Windows-specific first-run setup wizard.
 *
 * Mounted when ?window=onboarding&platform=windows (or when the Rust side
 * determines the platform is Windows at launch). This component takes the
 * existing Onboarding.tsx flow and re-skins it with the two-column layout,
 * amber gradient left rail, and step-progress list from HiFirstRun in
 * design/hifi/hifi-desktop.jsx.
 *
 * Steps:
 *   1. Sign in          — desktop_login
 *   2. Unlock vault     — desktop_unlock_with_recovery_phrase
 *   3. Pick sync folder — pick_sync_root, set_sync_mode (NEW — WS1)
 *   4. Explorer integration — install_windows_shell_integration
 *   ready               — show_main_app_window
 *
 * Invoke commands used:
 *   EXISTING:  desktop_login, desktop_unlock_with_recovery_phrase,
 *              desktop_platform, pick_sync_root, default_sync_root,
 *              list_vault_folders, set_recursive_pin, show_main_app_window
 *   NEW (WS1): install_windows_shell_integration, windows_shell_integration_state
 *   NEW (WS1): set_sync_mode (arg: mode: 'everything' | 'smart' | 'custom' | 'online_only')
 */

import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type FormEvent,
  type KeyboardEvent,
} from 'react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { listen } from '@tauri-apps/api/event'
import {
  command,
  commandUnavailableLabel,
  desktopLogin,
  desktopLogin2fa,
  loadSyncStatus,
  type DesktopPlatform,
  type FinderInstallState,
  type SyncStatus,
  type VaultItem,
} from './desktopApi'
import { useToast } from './windows/ui'

// Progress event emitted by the Rust `start_browser_login` handoff.
// `phase` keys the UI state machine; the other fields are populated on
// `waiting` (the device code) and `done` (the signed-in email).
type BrowserLoginEvent = {
  phase: 'connecting' | 'waiting' | 'authorized' | 'done' | 'error'
  user_code?: string
  verification_uri?: string
  expires_in?: number
  email?: string
  message?: string
}

// Windows shell integration uses the same shape as the macOS FinderInstallState
type ShellIntegrationState = FinderInstallState

type Step = 'signin' | 'unlock' | 'sync-mode' | 'explorer' | 'ready'
const RECOVERY_WORD_COUNT = 12

const STEPS: Array<{ id: Step; title: string; detail: string }> = [
  { id: 'signin', title: 'Sign in', detail: 'Authenticate your Beebeeb account.' },
  { id: 'unlock', title: 'Unlock vault', detail: 'Restore encryption keys on this PC.' },
  { id: 'sync-mode', title: 'Sync mode', detail: 'Pick what lives on this PC.' },
  { id: 'explorer', title: 'Explorer integration', detail: 'Add Beebeeb to File Explorer.' },
  { id: 'ready', title: 'Ready', detail: 'Open the control center.' },
]

type SyncMode = 'everything' | 'smart' | 'custom' | 'online_only'

const SYNC_OPTIONS: Array<{ mode: SyncMode; title: string; desc: string; recommended?: boolean }> = [
  { mode: 'everything', title: 'Everything', desc: 'Full vault · fastest, uses most disk' },
  { mode: 'smart', title: 'Smart', desc: 'Recent + starred + shared · smaller footprint', recommended: true },
  { mode: 'custom', title: 'Custom', desc: "I'll pick folders after setup" },
  { mode: 'online_only', title: 'Online-only', desc: 'Placeholders only · almost no disk use' },
]

// ── Design tokens (inline, no Tailwind dependency) ────────────────────────

const T = {
  paper: 'oklch(0.985 0.004 85)',
  paper2: 'oklch(0.968 0.006 85)',
  paper3: 'oklch(0.945 0.008 82)',
  line: 'oklch(0.90 0.008 82)',
  line2: 'oklch(0.83 0.01 80)',
  ink: 'oklch(0.18 0.01 70)',
  ink2: 'oklch(0.34 0.012 75)',
  ink3: 'oklch(0.52 0.01 78)',
  ink4: 'oklch(0.68 0.008 80)',
  amber: 'oklch(0.82 0.17 84)',
  amberDeep: 'oklch(0.66 0.15 72)',
  amberBg: 'oklch(0.97 0.03 92)',
  amberSoft: 'oklch(0.94 0.06 90)',
  green: 'oklch(0.72 0.16 155)',
  fontSans: "'Inter', 'Segoe UI', system-ui, ui-sans-serif, sans-serif",
  fontMono: "'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace",
} as const

// ── Small reusable pieces ─────────────────────────────────────────────────

function Btn({
  children,
  variant = 'ghost',
  disabled,
  onClick,
  type = 'button',
  wide,
}: {
  children: React.ReactNode
  variant?: 'primary' | 'ghost' | 'outline'
  disabled?: boolean
  onClick?: () => void
  type?: 'button' | 'submit'
  wide?: boolean
}) {
  const base: React.CSSProperties = {
    display: 'inline-flex',
    alignItems: 'center',
    justifyContent: 'center',
    gap: 6,
    padding: '7px 14px',
    fontSize: 13,
    fontFamily: T.fontSans,
    fontWeight: 500,
    borderRadius: 6,
    cursor: disabled ? 'not-allowed' : 'pointer',
    opacity: disabled ? 0.55 : 1,
    transition: 'all 100ms ease',
    border: '1px solid',
    width: wide ? '100%' : undefined,
    letterSpacing: '-0.005em',
    whiteSpace: 'nowrap',
  }
  const styles: Record<string, React.CSSProperties> = {
    primary: { background: T.ink, color: T.paper, borderColor: T.ink },
    ghost: { background: 'transparent', color: T.ink2, borderColor: 'transparent' },
    outline: { background: T.paper, color: T.ink2, borderColor: T.line2 },
  }
  return (
    <button
      type={type}
      disabled={disabled}
      onClick={onClick}
      style={{ ...base, ...styles[variant] }}
    >
      {children}
    </button>
  )
}

function Notice({ children, kind = 'warn' }: { children: React.ReactNode; kind?: 'warn' | 'error' }) {
  return (
    <div style={{
      padding: '10px 12px',
      borderRadius: 6,
      border: `1px solid ${kind === 'error' ? 'oklch(0.88 0.05 25)' : 'oklch(0.87 0.08 84)'}`,
      background: kind === 'error' ? 'oklch(0.98 0.02 25)' : T.amberBg,
      color: kind === 'error' ? 'oklch(0.42 0.15 25)' : T.amberDeep,
      fontSize: 12,
      lineHeight: 1.5,
      marginBottom: 14,
    }}>
      {children}
    </div>
  )
}

// ── Left rail ─────────────────────────────────────────────────────────────

function LeftRail({ currentStep }: { currentStep: Step }) {
  const currentIdx = STEPS.findIndex((s) => s.id === currentStep)
  return (
    <div style={{
      background: `linear-gradient(160deg, ${T.amberBg}, ${T.paper2})`,
      borderRight: `1px solid ${T.line}`,
      padding: '32px 24px',
      display: 'flex',
      flexDirection: 'column',
    }}>
      {/* Logo mark */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
        <div style={{
          width: 30,
          height: 30,
          borderRadius: 8,
          background: T.amber,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
        }}>
          <span style={{
            fontFamily: T.fontSans,
            fontWeight: 800,
            fontSize: 15,
            color: T.ink,
            lineHeight: 1,
          }}>b</span>
        </div>
        <span style={{
          fontFamily: T.fontSans,
          fontWeight: 700,
          fontSize: 15,
          color: T.ink,
          letterSpacing: '-0.02em',
        }}>Beebeeb</span>
      </div>

      <div style={{
        marginTop: 28,
        fontSize: 22,
        fontWeight: 700,
        letterSpacing: '-0.025em',
        lineHeight: 1.15,
        color: T.ink,
      }}>
        Welcome to your vault.
      </div>
      <div style={{
        marginTop: 10,
        fontSize: 12,
        color: T.ink3,
        lineHeight: 1.6,
        maxWidth: 200,
      }}>
        Four quick steps. No data leaves this PC without encryption.
      </div>

      {/* Step list */}
      <div style={{ marginTop: 32, display: 'flex', flexDirection: 'column', gap: 14 }}>
        {STEPS.filter((s) => s.id !== 'ready').map((step, i) => {
          const stepIdx = STEPS.indexOf(step)
          const done = stepIdx < currentIdx
          const current = step.id === currentStep
          return (
            <div key={step.id} style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
              <div style={{
                width: 22,
                height: 22,
                borderRadius: '50%',
                background: done ? T.ink : current ? T.amberDeep : T.paper,
                color: (done || current) ? T.paper : T.ink4,
                border: (done || current) ? 'none' : `1.5px solid ${T.line2}`,
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                fontSize: 11,
                fontWeight: 600,
                flexShrink: 0,
              }}>
                {done ? '✓' : i + 1}
              </div>
              <span style={{
                fontSize: 12.5,
                fontWeight: current ? 600 : done ? 400 : 400,
                color: current ? T.ink : done ? T.ink3 : T.ink4,
              }}>
                {step.title}
              </span>
            </div>
          )
        })}
      </div>

      <div style={{ marginTop: 'auto', paddingTop: 24 }}>
        <div style={{
          display: 'inline-flex',
          alignItems: 'center',
          gap: 6,
          fontSize: 10.5,
          fontFamily: T.fontMono,
          color: T.ink3,
          letterSpacing: '0.04em',
          textTransform: 'uppercase' as const,
        }}>
          Falkenstein · E2E encrypted
        </div>
      </div>
    </div>
  )
}

// ── Step components ───────────────────────────────────────────────────────

/**
 * Browser-based sign-in. Invokes the Rust `start_browser_login` handoff, which
 * opens the system browser to the device-code verification URL and waits for
 * the (already signed-in) browser to ECDH-encrypt the session token + master
 * key back. On success the Rust side has already persisted the session AND the
 * master key (the handoff returns both), so the vault is unlocked — `onDone`
 * receives `vaultUnlocked: true` so the wizard can skip the recovery-phrase step.
 *
 * Also handles the email+password path (including 2FA/TOTP) as an alternative
 * to the browser handoff. When `desktop_login` returns `requires_2fa: true` the
 * component advances to a TOTP sub-step; `desktop_login_2fa` completes it.
 */
function SignInStep({ onDone }: { onDone: (info: { vaultUnlocked: boolean }) => void }) {
  // ── Browser-flow state ──────────────────────────────────────────────────
  type BrowserPhase = 'idle' | 'connecting' | 'waiting' | 'authorized' | 'error'
  const [browserPhase, setBrowserPhase] = useState<BrowserPhase>('idle')
  const [userCode, setUserCode] = useState<string | null>(null)
  const [verificationUri, setVerificationUri] = useState<string | null>(null)
  const browserDoneRef = useRef(false)

  const browserBusy =
    browserPhase === 'connecting' ||
    browserPhase === 'waiting' ||
    browserPhase === 'authorized'

  // ── Email/password + TOTP state ─────────────────────────────────────────
  // 'browser'   — showing the browser-handoff form (default)
  // 'password'  — email + password form
  // 'totp'      — 6-digit TOTP code entry (reached after password succeeds with requires_2fa)
  type SignInMode = 'browser' | 'password' | 'totp'
  const [mode, setMode] = useState<SignInMode>('browser')

  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [pwBusy, setPwBusy] = useState(false)
  // Form-blocking: the user re-reads and corrects the password in place, so the error must stay next
  // to the input. A toast would scroll away mid-retry.
  // eslint-disable-next-line beebeeb/no-ad-hoc-error-surface
  const [pwError, setPwError] = useState<string | null>(null)

  const [totpCode, setTotpCode] = useState('')
  const [totpBusy, setTotpBusy] = useState(false)
  // Form-blocking: same as pwError — the 2FA code is corrected in place while the error is visible.
  // eslint-disable-next-line beebeeb/no-ad-hoc-error-surface
  const [totpError, setTotpError] = useState<string | null>(null)
  const totpInputRef = useRef<HTMLInputElement | null>(null)

  // Shared error for browser flow
  const [browserError, setBrowserError] = useState<string | null>(null)

  // ── Browser-flow handler ────────────────────────────────────────────────
  const startBrowserLogin = useCallback(async () => {
    setBrowserError(null)
    setUserCode(null)
    setVerificationUri(null)
    setBrowserPhase('connecting')
    browserDoneRef.current = false

    // Subscribe to handoff progress before invoking so we never miss an event.
    const unlisten = await listen<BrowserLoginEvent>('browser-login', (event) => {
      const p = event.payload
      switch (p.phase) {
        case 'connecting':
          setBrowserPhase('connecting')
          break
        case 'waiting':
          setBrowserPhase('waiting')
          if (p.user_code) setUserCode(p.user_code)
          if (p.verification_uri) setVerificationUri(p.verification_uri)
          break
        case 'authorized':
          setBrowserPhase('authorized')
          break
        case 'done':
          // Advance exactly once — guard with browserDoneRef so a late command
          // resolve can't double-fire onDone if the event arrived first.
          if (!browserDoneRef.current) {
            browserDoneRef.current = true
            onDone({ vaultUnlocked: true })
          }
          break
        case 'error':
          setBrowserPhase('error')
          setBrowserError(p.message ?? 'Sign-in failed. Please try again.')
          break
      }
    })

    const result = await command<void>('start_browser_login')
    unlisten()

    // If the 'done' event already fired (and advanced the wizard) before or
    // during the await, browserDoneRef is true — nothing left to do here.
    if (browserDoneRef.current) return

    if (result.ok) {
      // Command resolved OK but the 'done' event was either missed (arrived
      // after unlisten) or not yet emitted. Advance now — this is the same
      // success path: handoff persisted the session AND the master key, so the
      // vault is already unlocked — no recovery phrase needed on this path.
      browserDoneRef.current = true
      onDone({ vaultUnlocked: true })
      return
    }

    // Any non-success outcome MUST leave the busy/"Connecting" state and show a
    // visible, actionable error. The Rust handoff now bounds the WebSocket
    // connect with a 15s timeout, so this branch is always reached on a failure
    // (previously a stalled TLS handshake never returned and the UI hung on
    // "Connecting" forever with no error). We always fall back to phase 'error'
    // here — never leave the user staring at a spinner.
    const message = result.unsupported
      ? commandUnavailableLabel('start_browser_login')
      : result.reason
    setBrowserPhase('error')
    setBrowserError(message)
  }, [onDone])

  // ── Email + password submit ─────────────────────────────────────────────
  const submitPassword = useCallback(
    async (e: FormEvent) => {
      e.preventDefault()
      if (!email.trim() || !password) return
      setPwBusy(true)
      setPwError(null)
      const result = await desktopLogin(email.trim(), password)
      setPwBusy(false)
      if (!result.ok) {
        setPwError(
          result.unsupported ? commandUnavailableLabel('desktop_login') : result.reason,
        )
        return
      }
      if (result.value.requires_2fa) {
        // Advance to the TOTP sub-step; do NOT call onDone yet.
        setMode('totp')
        setTotpCode('')
        setTotpError(null)
        // Autofocus the TOTP input on the next paint.
        requestAnimationFrame(() => totpInputRef.current?.focus())
        return
      }
      // No 2FA — login is complete. The password path doesn't return the master
      // key (unlike the browser handoff), so vaultUnlocked is false — the user
      // will proceed to the recovery-phrase unlock step.
      onDone({ vaultUnlocked: false })
    },
    [email, password, onDone],
  )

  // ── TOTP submit ─────────────────────────────────────────────────────────
  const submitTotp = useCallback(
    async (e: FormEvent) => {
      e.preventDefault()
      const code = totpCode.trim()
      if (code.length !== 6) return
      setTotpBusy(true)
      setTotpError(null)
      const result = await desktopLogin2fa(code)
      setTotpBusy(false)
      if (!result.ok) {
        // Wrong code is retryable — clear the field and keep them on this step.
        setTotpCode('')
        setTotpError(
          result.unsupported ? commandUnavailableLabel('desktop_login_2fa') : result.reason,
        )
        requestAnimationFrame(() => totpInputRef.current?.focus())
        return
      }
      // 2FA complete — session installed by the Rust side. Same as password path:
      // vault key was not handed back, so the recovery-phrase step is next.
      onDone({ vaultUnlocked: false })
    },
    [totpCode, onDone],
  )

  // ── TOTP sub-step UI ────────────────────────────────────────────────────
  if (mode === 'totp') {
    return (
      <div>
        <div style={{
          fontSize: 11,
          fontFamily: T.fontMono,
          textTransform: 'uppercase' as const,
          letterSpacing: '0.07em',
          color: T.ink3,
          marginBottom: 4,
        }}>
          Step 1 of 4
        </div>
        <h1 style={{ margin: '4px 0 6px', fontSize: 22, fontWeight: 700, letterSpacing: '-0.02em', color: T.ink, lineHeight: 1.2 }}>
          Two-factor authentication
        </h1>
        <p style={{ margin: '0 0 22px', fontSize: 12, color: T.ink3, lineHeight: 1.6 }}>
          Enter the 6-digit code from your authenticator app.
        </p>
        {totpError && <Notice kind="error">{totpError}</Notice>}
        <form onSubmit={submitTotp}>
          <div style={{ marginBottom: 6, fontSize: 11, fontFamily: T.fontMono, textTransform: 'uppercase' as const, letterSpacing: '0.07em', color: T.ink3 }}>
            Authenticator code
          </div>
          <input
            ref={totpInputRef}
            type="text"
            inputMode="numeric"
            autoComplete="one-time-code"
            maxLength={6}
            placeholder="000000"
            disabled={totpBusy}
            value={totpCode}
            onChange={(e) => {
              // Strip non-digits, enforce max 6 characters.
              setTotpCode(e.currentTarget.value.replace(/\D/g, '').slice(0, 6))
            }}
            style={{
              display: 'block',
              width: '100%',
              padding: '10px 12px',
              fontSize: 24,
              fontFamily: T.fontMono,
              fontWeight: 600,
              letterSpacing: '0.25em',
              color: T.ink,
              background: T.paper,
              border: `1.5px solid ${totpError ? 'oklch(0.72 0.18 25)' : T.line2}`,
              borderRadius: 8,
              outline: 'none',
              boxSizing: 'border-box' as const,
              marginBottom: 16,
              transition: 'border-color 120ms',
            }}
          />
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
            <Btn
              variant="ghost"
              disabled={totpBusy}
              onClick={() => {
                setMode('password')
                setTotpCode('')
                setTotpError(null)
              }}
            >
              ← Back
            </Btn>
            <Btn
              type="submit"
              variant="primary"
              disabled={totpBusy || totpCode.trim().length !== 6}
            >
              {totpBusy ? 'Verifying…' : 'Verify →'}
            </Btn>
          </div>
        </form>
      </div>
    )
  }

  // ── Email + password sub-step UI ─────────────────────────────────────────
  if (mode === 'password') {
    return (
      <div>
        <div style={{
          fontSize: 11,
          fontFamily: T.fontMono,
          textTransform: 'uppercase' as const,
          letterSpacing: '0.07em',
          color: T.ink3,
          marginBottom: 4,
        }}>
          Step 1 of 4
        </div>
        <h1 style={{ margin: '4px 0 6px', fontSize: 22, fontWeight: 700, letterSpacing: '-0.02em', color: T.ink, lineHeight: 1.2 }}>
          Sign in to Beebeeb
        </h1>
        <p style={{ margin: '0 0 22px', fontSize: 12, color: T.ink3, lineHeight: 1.6 }}>
          Enter your email and password.
        </p>
        {pwError && <Notice kind="error">{pwError}</Notice>}
        <form onSubmit={submitPassword}>
          <label style={{ display: 'block', marginBottom: 14 }}>
            <div style={{ fontSize: 11, fontFamily: T.fontMono, textTransform: 'uppercase' as const, letterSpacing: '0.07em', color: T.ink3, marginBottom: 5 }}>
              Email
            </div>
            <input
              type="email"
              autoComplete="email"
              disabled={pwBusy}
              value={email}
              onChange={(e) => setEmail(e.currentTarget.value)}
              style={{
                display: 'block',
                width: '100%',
                padding: '8px 10px',
                fontSize: 13,
                fontFamily: T.fontSans,
                color: T.ink,
                background: T.paper,
                border: `1px solid ${T.line2}`,
                borderRadius: 6,
                outline: 'none',
                boxSizing: 'border-box' as const,
              }}
            />
          </label>
          <label style={{ display: 'block', marginBottom: 18 }}>
            <div style={{ fontSize: 11, fontFamily: T.fontMono, textTransform: 'uppercase' as const, letterSpacing: '0.07em', color: T.ink3, marginBottom: 5 }}>
              Password
            </div>
            <input
              type="password"
              autoComplete="current-password"
              disabled={pwBusy}
              value={password}
              onChange={(e) => setPassword(e.currentTarget.value)}
              style={{
                display: 'block',
                width: '100%',
                padding: '8px 10px',
                fontSize: 13,
                fontFamily: T.fontSans,
                color: T.ink,
                background: T.paper,
                border: `1px solid ${T.line2}`,
                borderRadius: 6,
                outline: 'none',
                boxSizing: 'border-box' as const,
              }}
            />
          </label>
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
            <Btn
              variant="ghost"
              disabled={pwBusy}
              onClick={() => {
                setMode('browser')
                setPwError(null)
              }}
            >
              ← Back
            </Btn>
            <Btn
              type="submit"
              variant="primary"
              disabled={pwBusy || !email.trim() || !password}
            >
              {pwBusy ? 'Signing in…' : 'Sign in →'}
            </Btn>
          </div>
        </form>
      </div>
    )
  }

  // ── Browser-handoff UI (default mode) ───────────────────────────────────
  return (
    <div>
      <div style={{ fontSize: 11, fontFamily: T.fontMono, textTransform: 'uppercase' as const, letterSpacing: '0.07em', color: T.ink3, marginBottom: 4 }}>
        Step 1 of 4
      </div>
      <h1 style={{ margin: '4px 0 6px', fontSize: 22, fontWeight: 700, letterSpacing: '-0.02em', color: T.ink, lineHeight: 1.2 }}>
        Sign in to Beebeeb
      </h1>
      <p style={{ margin: '0 0 22px', fontSize: 12, color: T.ink3, lineHeight: 1.6 }}>
        Sign in through your browser. Beebeeb opens a Beebeeb tab, you confirm
        the code below, and your access &amp; refresh credentials are handed back
        encrypted — end-to-end, never touching this app in the clear.
      </p>
      {browserError && <Notice kind="error">{browserError}</Notice>}

      {browserPhase === 'waiting' || browserPhase === 'authorized' ? (
        <div style={{
          padding: '16px 18px',
          background: T.amberBg,
          border: `1px solid ${T.amberDeep}`,
          borderRadius: 10,
          marginBottom: 18,
        }}>
          <div style={{ fontSize: 11, fontFamily: T.fontMono, textTransform: 'uppercase' as const, letterSpacing: '0.07em', color: T.amberDeep, marginBottom: 8 }}>
            {browserPhase === 'authorized' ? 'Authorized — finishing…' : 'We opened your browser'}
          </div>
          <p style={{ margin: '0 0 12px', fontSize: 12, color: T.ink2, lineHeight: 1.6 }}>
            {browserPhase === 'authorized'
              ? 'Decrypting your credentials and starting sync…'
              : 'Confirm the sign-in in the browser tab we just opened. Check that the code there matches:'}
          </p>
          {userCode && (
            <div style={{
              fontSize: 26,
              fontWeight: 700,
              letterSpacing: '0.12em',
              fontFamily: T.fontMono,
              color: T.ink,
              marginBottom: 10,
            }}>
              {userCode}
            </div>
          )}
          {verificationUri && browserPhase === 'waiting' && (
            <p style={{ margin: 0, fontSize: 11, color: T.ink3, lineHeight: 1.5, wordBreak: 'break-all' as const }}>
              Browser didn&apos;t open? Visit{' '}
              <span style={{ fontFamily: T.fontMono, color: T.amberDeep }}>{verificationUri}</span>
              {' '}in any signed-in browser.
            </p>
          )}
        </div>
      ) : null}

      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginTop: 8 }}>
        {!browserBusy && (
          <Btn variant="ghost" onClick={() => setMode('password')}>
            Sign in with email →
          </Btn>
        )}
        <div style={{ marginLeft: 'auto' }}>
          <Btn variant="primary" disabled={browserBusy} onClick={() => void startBrowserLogin()}>
            {browserPhase === 'connecting' && 'Connecting…'}
            {browserPhase === 'waiting' && 'Waiting for browser…'}
            {browserPhase === 'authorized' && 'Finishing…'}
            {(browserPhase === 'idle' || browserPhase === 'error') && (browserPhase === 'error' ? 'Try again' : 'Sign in with browser →')}
          </Btn>
        </div>
      </div>
    </div>
  )
}

function UnlockStep({ onDone }: { onDone: () => void }) {
  const [words, setWords] = useState<string[]>(() => Array.from({ length: RECOVERY_WORD_COUNT }, () => ''))
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const refs = useRef<Array<HTMLInputElement | null>>([])

  const phrase = words.map((w) => w.trim()).join(' ').trim()
  const filled = words.filter((w) => w.trim()).length
  const canUnlock = filled === RECOVERY_WORD_COUNT

  const applyWords = (start: number, raw: string[]) => {
    const clean = raw.map((w) => w.trim()).filter(Boolean)
    if (!clean.length) return
    setError(null)
    setWords((prev) => {
      const next = [...prev]
      clean.slice(0, RECOVERY_WORD_COUNT - start).forEach((w, i) => { next[start + i] = w })
      return next
    })
    requestAnimationFrame(() => refs.current[Math.min(start + clean.length, RECOVERY_WORD_COUNT - 1)]?.focus())
  }

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    if (!canUnlock) return
    setBusy(true)
    setError(null)
    const result = await command<void>('desktop_unlock_with_recovery_phrase', { recoveryPhrase: phrase })
    setBusy(false)
    if (result.ok) { onDone(); return }
    setError(result.unsupported ? commandUnavailableLabel('desktop_unlock_with_recovery_phrase') : result.reason)
  }

  return (
    <div>
      <div style={{ fontSize: 11, fontFamily: T.fontMono, textTransform: 'uppercase' as const, letterSpacing: '0.07em', color: T.ink3, marginBottom: 4 }}>
        Step 2 of 4
      </div>
      <h1 style={{ margin: '4px 0 6px', fontSize: 22, fontWeight: 700, letterSpacing: '-0.02em', color: T.ink, lineHeight: 1.2 }}>
        Unlock your vault
      </h1>
      <p style={{ margin: '0 0 18px', fontSize: 12, color: T.ink3, lineHeight: 1.6 }}>
        This PC does not have your encryption keys yet. Enter your 12-word recovery phrase to restore them.
      </p>
      {error && <Notice kind="error">{error}</Notice>}
      <form onSubmit={submit}>
        <div style={{ marginBottom: 6, fontSize: 11, fontFamily: T.fontMono, textTransform: 'uppercase' as const, letterSpacing: '0.07em', color: T.ink3 }}>
          Recovery phrase
        </div>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, minmax(0, 1fr))', gap: 6, marginBottom: 12 }}>
          {words.map((word, idx) => (
            <label
              key={idx}
              style={{
                display: 'grid',
                gridTemplateColumns: '20px minmax(0, 1fr)',
                alignItems: 'center',
                border: `1px solid ${T.line2}`,
                borderRadius: 6,
                background: T.paper,
                overflow: 'hidden',
                minHeight: 36,
              }}
            >
              <span style={{ fontSize: 10, fontWeight: 700, color: T.ink4, textAlign: 'center', fontFamily: T.fontMono }}>{idx + 1}</span>
              <input
                ref={(n) => { refs.current[idx] = n }}
                aria-label={`Recovery word ${idx + 1}`}
                autoCapitalize="none"
                autoComplete="off"
                autoCorrect="off"
                spellCheck={false}
                type="text"
                value={word}
                disabled={busy}
                style={{
                  width: '100%',
                  height: 34,
                  border: 'none',
                  borderLeft: `1px solid ${T.line}`,
                  borderRadius: 0,
                  background: 'transparent',
                  fontSize: 12,
                  fontFamily: T.fontMono,
                  color: T.ink,
                  padding: '6px 7px',
                  outline: 'none',
                }}
                onChange={(e) => {
                  if (/\s/.test(e.currentTarget.value)) { applyWords(idx, e.currentTarget.value.split(/\s+/)); return }
                  setError(null)
                  setWords((prev) => prev.map((w, i) => i === idx ? e.currentTarget.value : w))
                }}
                onKeyDown={(e: KeyboardEvent<HTMLInputElement>) => {
                  if (e.key === ' ' || e.key === 'Enter') { e.preventDefault(); refs.current[Math.min(idx + 1, RECOVERY_WORD_COUNT - 1)]?.focus(); return }
                  if (e.key === 'Backspace' && !word && idx > 0) refs.current[idx - 1]?.focus()
                }}
                onPaste={(e) => { e.preventDefault(); applyWords(idx, e.clipboardData.getData('text').split(/\s+/)) }}
              />
            </label>
          ))}
        </div>
        <p style={{ margin: '0 0 14px', fontSize: 11, color: T.ink3, lineHeight: 1.5 }}>
          Paste all 12 words into any box, or type them one by one.
          Beebeeb stores the unlocked key in the Windows Credential Manager.
        </p>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
          {error && (
            <Btn onClick={onDone}>Continue without unlock</Btn>
          )}
          <div style={{ marginLeft: 'auto' }}>
            <Btn type="submit" variant="primary" disabled={busy || !canUnlock}>
              {busy ? 'Unlocking…' : 'Unlock vault →'}
            </Btn>
          </div>
        </div>
      </form>
    </div>
  )
}

function SyncModeStep({ onDone }: { onDone: () => void }) {
  const [selected, setSelected] = useState<SyncMode>('smart')
  const [busy, setBusy] = useState(false)
  const { showToast } = useToast()

  const proceed = async () => {
    setBusy(true)
    const result = await command<void>('set_sync_mode', { mode: selected })
    setBusy(false)
    if (result.ok) {
      onDone()
      return
    }
    showToast({ variant: 'error', title: 'Couldn’t set sync mode', message: result.reason })
  }

  return (
    <div>
      <div style={{ fontSize: 11, fontFamily: T.fontMono, textTransform: 'uppercase' as const, letterSpacing: '0.07em', color: T.ink3, marginBottom: 4 }}>
        Step 3 of 4
      </div>
      <h1 style={{ margin: '4px 0 6px', fontSize: 22, fontWeight: 700, letterSpacing: '-0.02em', color: T.ink, lineHeight: 1.2 }}>
        Pick what lives on this PC
      </h1>
      <p style={{ margin: '0 0 18px', fontSize: 12, color: T.ink3, lineHeight: 1.6 }}>
        Keep everything, or only what you need. Online-only folders appear in File Explorer as placeholders — open them to download on demand.
      </p>
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 10, marginBottom: 20 }}>
        {SYNC_OPTIONS.map((opt) => {
          const active = selected === opt.mode
          return (
            <button
              key={opt.mode}
              onClick={() => setSelected(opt.mode)}
              style={{
                padding: 14,
                borderRadius: 10,
                border: `1.5px solid ${active ? T.amberDeep : T.line}`,
                background: active ? T.amberBg : T.paper,
                cursor: 'pointer',
                textAlign: 'left' as const,
                position: 'relative' as const,
                fontFamily: T.fontSans,
                transition: 'border-color 120ms, background 120ms',
              }}
            >
              {opt.recommended && (
                <div style={{
                  position: 'absolute',
                  top: -8,
                  right: 10,
                  fontSize: 9,
                  padding: '2px 7px',
                  background: T.amberDeep,
                  color: T.paper,
                  borderRadius: 6,
                  fontWeight: 600,
                  letterSpacing: 0.3,
                  fontFamily: T.fontSans,
                  textTransform: 'uppercase' as const,
                }}>
                  Recommended
                </div>
              )}
              <div style={{ fontSize: 13.5, fontWeight: 600, color: T.ink, marginBottom: 4 }}>{opt.title}</div>
              <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.4 }}>{opt.desc}</div>
            </button>
          )
        })}
      </div>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
        <span style={{ fontSize: 11, color: T.ink4 }}>You can change this any time in Settings.</span>
        <Btn variant="primary" disabled={busy} onClick={() => void proceed()}>
          Continue →
        </Btn>
      </div>
    </div>
  )
}

function ExplorerStep({ onDone }: { onDone: () => void }) {
  const [syncRoot, setSyncRoot] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const { showToast } = useToast()

  useEffect(() => {
    command<string>('default_sync_root').then((r) => { if (r.ok) setSyncRoot(r.value) })
  }, [])

  const chooseFolder = useCallback(async () => {
    setBusy(true)
    const picked = await command<string | null>('pick_sync_root')
    setBusy(false)
    if (picked.ok && picked.value) { setSyncRoot(picked.value); return }
    if (!picked.ok) {
      showToast({
        variant: 'error',
        title: 'Couldn’t open the folder picker',
        message: picked.unsupported ? commandUnavailableLabel('pick_sync_root') : picked.reason,
      })
    }
  }, [showToast])

  const install = useCallback(async () => {
    setBusy(true)
    const result = await command<ShellIntegrationState>('install_windows_shell_integration', { path: syncRoot })
    setBusy(false)
    if (result.ok) { onDone(); return }
    showToast({
      variant: 'error',
      title: 'Couldn’t install File Explorer integration',
      message: result.unsupported ? commandUnavailableLabel('install_windows_shell_integration') : result.reason,
    })
  }, [onDone, syncRoot, showToast])

  const skip = useCallback(() => {
    // No dedicated skip command on Windows — proceed directly
    onDone()
  }, [onDone])

  return (
    <div>
      <div style={{ fontSize: 11, fontFamily: T.fontMono, textTransform: 'uppercase' as const, letterSpacing: '0.07em', color: T.ink3, marginBottom: 4 }}>
        Step 4 of 4
      </div>
      <h1 style={{ margin: '4px 0 6px', fontSize: 22, fontWeight: 700, letterSpacing: '-0.02em', color: T.ink, lineHeight: 1.2 }}>
        File Explorer integration
      </h1>
      <p style={{ margin: '0 0 18px', fontSize: 12, color: T.ink3, lineHeight: 1.6 }}>
        Beebeeb will appear as a location in File Explorer. This is separate from the sync folder you'll use every day.
      </p>
      <div style={{
        padding: '14px 16px',
        background: T.paper2,
        border: `1px solid ${T.line}`,
        borderRadius: 8,
        marginBottom: 18,
      }}>
        <div style={{ fontSize: 10.5, fontFamily: T.fontMono, textTransform: 'uppercase' as const, letterSpacing: '0.07em', color: T.ink3, marginBottom: 6 }}>
          Sync folder
        </div>
        <div style={{
          fontSize: 13,
          fontFamily: T.fontMono,
          color: T.ink2,
          wordBreak: 'break-all' as const,
        }}>
          {syncRoot ?? 'C:\\Users\\…\\Beebeeb'}
        </div>
      </div>
      <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
        <Btn variant="outline" onClick={() => void chooseFolder()} disabled={busy}>
          Choose folder
        </Btn>
        <Btn variant="ghost" onClick={skip} disabled={busy}>
          Skip
        </Btn>
        <Btn variant="primary" onClick={() => void install()} disabled={busy}>
          {busy ? 'Installing…' : 'Install →'}
        </Btn>
      </div>
    </div>
  )
}

function ReadyStep() {
  const [status, setStatus] = useState<SyncStatus | null>(null)

  // Poll sync_status so the engine badge reflects reality instead of a
  // one-shot snapshot taken before the engine has fully started.
  useEffect(() => {
    let cancelled = false
    const refresh = async () => {
      const next = await loadSyncStatus()
      if (!cancelled) setStatus(next)
    }
    void refresh()
    const id = window.setInterval(refresh, 2000)
    return () => { cancelled = true; window.clearInterval(id) }
  }, [])

  const finish = async () => {
    // "Open control center" lands in the main app window (the full sidebar +
    // content shell). Settings is reachable from a nav item inside the app.
    await command<void>('show_main_app_window')
    try { await getCurrentWindow().close() } catch { window.close() }
  }

  return (
    <div>
      <h1 style={{ margin: '0 0 6px', fontSize: 22, fontWeight: 700, letterSpacing: '-0.02em', color: T.ink, lineHeight: 1.2 }}>
        All set.
      </h1>
      <p style={{ margin: '0 0 22px', fontSize: 12, color: T.ink3, lineHeight: 1.6 }}>
        Beebeeb is running on this PC. File Explorer is your file surface — the Beebeeb app handles everything else.
      </p>
      <div style={{
        display: 'grid',
        gridTemplateColumns: 'repeat(3, minmax(0, 1fr))',
        gap: 10,
        marginBottom: 22,
      }}>
        {[
          { label: 'Engine', value: status?.engine ?? '…' },
          { label: 'Queue', value: String(status?.syncing ?? 0) },
          { label: 'Conflicts', value: String(status?.conflicts ?? 0) },
        ].map(({ label, value }) => (
          <div key={label} style={{
            padding: '14px 16px',
            background: T.paper,
            border: `1px solid ${T.line}`,
            borderRadius: 8,
          }}>
            <div style={{ fontSize: 10, fontFamily: T.fontMono, textTransform: 'uppercase' as const, letterSpacing: '0.07em', color: T.ink3, marginBottom: 6 }}>{label}</div>
            <div style={{ fontSize: 22, fontWeight: 700, fontFamily: T.fontMono, color: T.ink }}>{value}</div>
          </div>
        ))}
      </div>
      <Btn variant="primary" wide onClick={() => void finish()}>
        Open control center →
      </Btn>
    </div>
  )
}

// ── Root component ─────────────────────────────────────────────────────────

export default function WindowsFirstRun() {
  const [step, setStep] = useState<Step>('signin')
  const [loggedIn, setLoggedIn] = useState<boolean>(true)

  // Watch for sign-out: if the backend clears the session while we are on
  // the "ready" screen, reset the wizard back to step 1 so the user sees
  // the sign-in form rather than a stale "All set." screen.
  // Only update loggedIn on a NON-NULL status — a null result means the IPC
  // errored / Tauri wasn't ready / timed out, which must never be mistaken
  // for a logout (a transient null would otherwise trip the reset).
  useEffect(() => {
    let cancelled = false
    const checkSession = async () => {
      const status = await loadSyncStatus()
      if (cancelled || !status) return
      setLoggedIn(status.logged_in)
    }
    void checkSession()
    const id = window.setInterval(checkSession, 3000)
    return () => { cancelled = true; window.clearInterval(id) }
  }, [])

  // The only stale-view scenario is a lingering "All set." screen after a
  // sign-out, so reset to step 1 ONLY from the final 'ready' step. Resetting
  // from unlock/sync-mode/explorer would discard a normally-signed-in user's
  // mid-onboarding progress on a transient glitch.
  useEffect(() => {
    if (!loggedIn && step === 'ready') {
      setStep('signin')
    }
  }, [loggedIn, step])

  // Fast-forward past completed steps on mount.
  // sync-mode is ONLY skipped if a mode was already persisted (re-open scenario);
  // on first run it always shows so the user makes an explicit choice.
  useEffect(() => {
    let cancelled = false

    // The Rust side auto-unlocks the vault on launch when the OS credential
    // store still holds the master key (see `restore_session_on_startup`).
    // That runs concurrently with this window opening, so a status read taken
    // the instant the wizard mounts can briefly see `logged_in: true` but
    // `vault_unlocked: false` even though the key IS present and is about to
    // load. Re-poll for a short window before falling back to the
    // recovery-phrase step, so we never show "this PC doesn't have your keys"
    // to a user whose keys are simply still loading.
    async function resolveVaultUnlocked(): Promise<boolean | null> {
      for (let attempt = 0; attempt < 6; attempt++) {
        const status = await loadSyncStatus()
        if (cancelled) return null
        if (!status?.logged_in) return null
        if (status.vault_unlocked) return true
        // Not unlocked yet — wait for the startup auto-unlock to land.
        await new Promise((r) => setTimeout(r, 250))
        if (cancelled) return null
      }
      // Genuinely still locked after retrying → the key is absent; the
      // recovery-phrase step is correct.
      return false
    }

    Promise.all([
      resolveVaultUnlocked(),
      command<DesktopPlatform>('desktop_platform'),
      command<ShellIntegrationState>('windows_shell_integration_state'),
      command<{ mode: string } | null>('desktop_config'),
    ]).then(([unlocked, _platform, shellState, config]) => {
      if (cancelled || unlocked === null) return
      if (!unlocked) { setStep('unlock'); return }
      // Only skip sync-mode if a mode has already been persisted (re-open, not first run).
      // desktop_config returns null / unsupported on first run.
      const syncModePersisted = config.ok && config.value !== null
      if (!syncModePersisted) { setStep('sync-mode'); return }
      if (!shellState.ok || !shellState.value.installed) { setStep('explorer'); return }
      setStep('ready')
    })
    return () => { cancelled = true }
  }, [])

  return (
    <div style={{
      minHeight: '100vh',
      background: T.paper,
      display: 'grid',
      gridTemplateColumns: '260px 1fr',
      fontFamily: T.fontSans,
      color: T.ink,
    }}>
      <LeftRail currentStep={step} />

      <div style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        padding: '40px 36px',
      }}>
        <div style={{ width: '100%', maxWidth: 420 }}>
          {step === 'signin' && (
            <SignInStep
              onDone={({ vaultUnlocked }) =>
                // The browser handoff returns the master key, so the vault is
                // already unlocked — skip the recovery-phrase step in that case.
                setStep(vaultUnlocked ? 'sync-mode' : 'unlock')
              }
            />
          )}
          {step === 'unlock' && <UnlockStep onDone={() => setStep('sync-mode')} />}
          {step === 'sync-mode' && <SyncModeStep onDone={() => setStep('explorer')} />}
          {step === 'explorer' && <ExplorerStep onDone={() => setStep('ready')} />}
          {step === 'ready' && <ReadyStep />}
        </div>
      </div>
    </div>
  )
}
