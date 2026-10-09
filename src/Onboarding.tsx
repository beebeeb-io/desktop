import { useRegionLabel } from './windows/useRegion'
import { useCallback, useEffect, useRef, useState, type FormEvent, type KeyboardEvent, type ReactNode } from 'react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import OnboardingErrorBoundary from './OnboardingErrorBoundary'
import {
  command,
  commandUnavailableLabel,
  lastSignedInEmail,
  loadSyncStatus,
  type CommandResult,
  type DesktopPlatform,
  type FinderInstallState,
  type SyncStatus,
  type VaultItem,
} from './desktopApi'
import { useCapabilities } from './capabilities'
import { submitPassword, submitTotpCode, type SignInSettled } from './onboardingSignIn'
import { ACCOUNT_SWITCH_CANCEL, ACCOUNT_SWITCH_CONFIRM, ACCOUNT_SWITCH_FAILED, ACCOUNT_SWITCH_TITLE, accountSwitchBody } from './accountSwitchCopy'
import { classifyFinderInstallResult } from './finderInstallCard'
import { loadFinderSetup, useFinderSetup } from './finderSetup'
import { FINDER_RAIL_DETAIL, FINDER_RAIL_TITLE, FINDER_SETUP_TITLE } from './finderSetupCopy'
import { Wordmark } from './Logo'
import { useToast } from './windows/ui'

type Step = 'signin' | 'unlock' | 'finder' | 'pinning' | 'ready' | 'switch'
const RECOVERY_WORD_COUNT = 12

const STEPS: Array<{ id: Step; title: string; detail: string }> = [
  { id: 'signin', title: 'Sign in', detail: 'Authenticate your account.' },
  { id: 'unlock', title: 'Set up this Mac', detail: 'Restore the vault key on this device.' },
  { id: 'finder', title: 'Install Finder location', detail: 'Register Beebeeb in the Finder sidebar.' },
  { id: 'pinning', title: 'Choose offline folders', detail: 'Default is online-only; pin only what you need.' },
  { id: 'ready', title: 'Review status', detail: 'Open the control center.' },
]

export default function Onboarding({ mode = 'setup' }: { mode?: 'setup' | 'reauth' }) {
  return <OnboardingErrorBoundary><OnboardingView mode={mode} /></OnboardingErrorBoundary>
}

function OnboardingView({ mode }: { mode: 'setup' | 'reauth' }) {
  const [step, setStep] = useState<Step>('signin')
  // How many unsent changes the switch warning names; set when a sign-in turns out to be another account.
  const [pendingSwitch, setPendingSwitch] = useState(0)
  // `null` until `desktop_platform` has answered, so the Finder step never flashes the wrong
  // variant. A platform that cannot be read stays resolvable through the capability snapshot
  // (below); only when both are unknown does it become 'unknown', which takes the Windows/Linux
  // step exactly as it did before the macOS step existed (never a blank page).
  const [platform, setPlatform] = useState<DesktopPlatform | null>(null)
  // The snapshot's host OS (what main.tsx's HostOnboarding already routes on). A Mac whose
  // `desktop_platform` call fails must still be a Mac here: the Windows/Linux step runs the
  // install-era command, which no macOS code path may reach.
  const hostOs: DesktopPlatform = useCapabilities()?.host_os ?? 'unknown'
  const regionLabel = useRegionLabel(step)
  // On macOS the Finder row must not promise a manual install (nothing is installed by hand
  // there). Until the platform has answered, and everywhere else, the rail is `STEPS` as written.
  const rail = STEPS.map((item) =>
    item.id === 'finder' && platform === 'macos' ? { ...item, title: FINDER_RAIL_TITLE, detail: FINDER_RAIL_DETAIL } : item,
  )

  useEffect(() => {
    let cancelled = false

    Promise.all([loadSyncStatus(), command<DesktopPlatform>('desktop_platform')]).then(async ([status, platformResult]) => {
      if (cancelled) return
      // `desktop_platform` answers first; if it failed or said 'unknown', the snapshot decides.
      const answered: DesktopPlatform = platformResult.ok ? platformResult.value : 'unknown'
      const resolved: DesktopPlatform = answered === 'unknown' ? hostOs : answered
      setPlatform(resolved)
      // R8: "Sign in again" opens this window in reauth mode. It starts at sign-in whatever
      // sync_status says; nothing was cleared, so the status still reads signed in and unlocked.
      if (mode === 'reauth') return
      if (!status?.logged_in) return

      if (!status.vault_unlocked) {
        setStep('unlock')
        return
      }

      if (resolved === 'macos') {
        // Spec 2026-10-06 §10: the reconciler's state, not the install-era command. Anything
        // but Ready (adding, failed, turned off, unreadable) goes to the step that says which.
        const finder = await loadFinderSetup()
        if (cancelled) return
        setStep(finder.ok && finder.value.setup === 'ready' ? 'ready' : 'finder')
        return
      }
      setStep(status.sync_root ? 'ready' : 'finder')
    })

    return () => {
      cancelled = true
    }
  }, [hostOs, mode])

  // The window closes itself through the `onboarding-close` capability (core:window:allow-close,
  // this window only). The close is awaited, and a refusal falls back to the DOM close, as the
  // ReadyStep does, so a rejection is never silently dropped.
  const closeWindow = async () => {
    try {
      await getCurrentWindow().close()
    } catch {
      window.close()
    }
  }

  const afterSignIn = (settled: SignInSettled) => {
    if (settled.kind === 'account_mismatch') {
      setPendingSwitch(settled.pendingChanges)
      setStep('switch')
      return
    }
    if (settled.kind === 'reauthenticated' && settled.vaultUnlocked) {
      // The same account, its keys here: sync resumes; nothing else to set up.
      void closeWindow()
      return
    }
    setStep('unlock')
  }

  return (
    <div className="onboarding-shell">
      <aside className="onboarding-rail">
        <div>
          <div className="onboarding-brand">
            <Wordmark className="onboarding-logo" />
            <div className="brand-subtitle">Private macOS file access</div>
          </div>
          <div className="steps">
            {rail.map((item, index) => (
              <div key={item.id} className={`step-row ${step === item.id ? 'active' : ''}`}>
                <div className="step-number">{index + 1}</div>
                <div>
                  <div className="row-title">{item.title}</div>
                  <div className="row-detail">{item.detail}</div>
                </div>
              </div>
            ))}
          </div>
        </div>
        <div className="sidebar-footer">
          {regionLabel} | Zero-knowledge
        </div>
      </aside>

      <main className="onboarding-main">
        {step === 'signin' && <SignInStep onDone={afterSignIn} />}
        {step === 'switch' && (
          <AccountSwitchStep
            pendingChanges={pendingSwitch}
            onSwitched={() => setStep('signin')}
            onCancel={() => (mode === 'reauth' ? void closeWindow() : setStep('signin'))}
          />
        )}
        {step === 'unlock' && <UnlockStep onDone={() => setStep('finder')} />}
        {step === 'finder' &&
          platform !== null &&
          (platform === 'macos' ? (
            <MacFinderStep onDone={() => setStep('pinning')} />
          ) : (
            <FinderInstallStep onDone={() => setStep('pinning')} />
          ))}
        {step === 'pinning' && <PinningStep onDone={() => setStep('ready')} />}
        {step === 'ready' && <ReadyStep />}
      </main>
    </div>
  )
}

function Card({
  title,
  copy,
  children,
}: {
  title: string
  copy: string
  children: ReactNode
}) {
  return (
    <section className="auth-card">
      <div className="auth-card-header">
        <Wordmark className="auth-logo" />
      </div>
      <h1 className="page-title">{title}</h1>
      <p className="page-copy" style={{ marginBottom: 22 }}>
        {copy}
      </p>
      {children}
    </section>
  )
}

function Field({
  label,
  type,
  value,
  onChange,
  disabled,
  placeholder,
}: {
  label: string
  type: string
  value: string
  onChange: (value: string) => void
  disabled?: boolean
  placeholder?: string
}) {
  return (
    <label style={{ display: 'block', marginBottom: 14 }}>
      <span className="field-label" style={{ display: 'block', marginBottom: 6 }}>
        {label}
      </span>
      <input
        className="form-input"
        type={type}
        value={value}
        disabled={disabled}
        placeholder={placeholder}
        onChange={(event) => onChange(event.currentTarget.value)}
      />
    </label>
  )
}

/**
 * Sign-in, including the 2FA sub-step (task 1521).
 *
 * `desktop_login` (email + password) can resolve `requiresTotp: true` when
 * the account has 2FA enabled — the password was correct, but the server has
 * only issued a short-lived partial token, not a real session. This
 * component MUST show a code prompt and call `submitTotpCode` before calling
 * `onDone`; skipping that check is exactly what shipped before this fix (see
 * `onboardingSignIn.ts`'s doc comment) — sign-in silently "succeeded" with no
 * session installed, and the next step (vault unlock) failed with "Sign in
 * before unlocking the vault."
 *
 * The 2FA sub-step mirrors the web client's `TwoFactorPrompt`
 * (`repos/web/src/components/two-factor-prompt.tsx`): a 6-digit authenticator
 * code by default, with a "Use backup code" toggle for the 8-digit codes
 * issued at 2FA setup — the server's `/auth/2fa/verify` accepts either in the
 * same field (`verify_totp_or_backup`).
 */
function SignInStep({ onDone }: { onDone: (settled: SignInSettled) => void }) {
  type Mode = 'password' | 'totp' | 'backup'
  const [mode, setMode] = useState<Mode>('password')

  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [busy, setBusy] = useState(false)
  const [passwordError, setPasswordError] = useState<string | null>(null)

  // Prefill the email from the last signed-in account on this install, so a
  // sign-out / startup-401 auto sign-out lands on a password form that
  // already knows the address (backend keeps it in desktop.toml — the
  // keychain account-email credential is erased by sign-out). Prefill only:
  // never overwrites what the user is typing, and stays silent on
  // unavailability (fresh install / hand-edited config).
  useEffect(() => {
    let cancelled = false
    void lastSignedInEmail().then((result) => {
      if (cancelled || !result.ok || !result.value) return
      setEmail((current) => (current ? current : result.value as string))
    })
    return () => {
      cancelled = true
    }
  }, [])

  const [totpCode, setTotpCode] = useState('')
  const [backupCode, setBackupCode] = useState('')
  const [totpError, setTotpError] = useState<string | null>(null)
  const totpInputRef = useRef<HTMLInputElement | null>(null)

  const startTotpStep = (nextMode: 'totp' | 'backup') => {
    setMode(nextMode)
    setTotpCode('')
    setBackupCode('')
    setTotpError(null)
    requestAnimationFrame(() => totpInputRef.current?.focus())
  }

  const submitPasswordForm = async (event: FormEvent) => {
    event.preventDefault()
    setBusy(true)
    setPasswordError(null)
    const result = await submitPassword(email, password)
    setBusy(false)
    if (!result.ok) {
      setPasswordError(result.message)
      return
    }
    if (result.requiresTotp) {
      startTotpStep('totp')
      return
    }
    onDone(result.settled)
  }

  const submitTotpForm = async (event: FormEvent) => {
    event.preventDefault()
    const code = mode === 'backup' ? backupCode.trim() : totpCode.trim()
    if (!code) return
    setBusy(true)
    setTotpError(null)
    const result = await submitTotpCode(code)
    setBusy(false)
    if (!result.ok) {
      // Wrong/expired code is retryable within the partial token's ~5-minute
      // window — clear the field and keep the user on this step.
      setTotpCode('')
      setBackupCode('')
      setTotpError(result.message)
      requestAnimationFrame(() => totpInputRef.current?.focus())
      return
    }
    onDone(result.settled)
  }

  if (mode === 'totp' || mode === 'backup') {
    const isBackup = mode === 'backup'
    return (
      <Card
        title="Two-factor authentication"
        copy={
          isBackup
            ? 'Enter one of the 8-digit backup codes you saved when enabling 2FA.'
            : 'Enter the 6-digit code from your authenticator app.'
        }
      >
        {totpError && <div className="notice">{totpError}</div>}
        <form onSubmit={submitTotpForm} style={{ marginTop: 16 }}>
          {isBackup ? (
            <Field
              label="Backup code"
              type="text"
              value={backupCode}
              onChange={(value) => {
                setBackupCode(value.replace(/\D/g, '').slice(0, 8))
                setTotpError(null)
              }}
              disabled={busy}
              placeholder="12345678"
            />
          ) : (
            <label style={{ display: 'block', marginBottom: 14 }}>
              <span className="field-label" style={{ display: 'block', marginBottom: 6 }}>
                Authentication code
              </span>
              <input
                ref={totpInputRef}
                className="form-input"
                type="text"
                inputMode="numeric"
                autoComplete="one-time-code"
                maxLength={6}
                placeholder="000000"
                disabled={busy}
                value={totpCode}
                onChange={(event) => {
                  setTotpCode(event.currentTarget.value.replace(/\D/g, '').slice(0, 6))
                  setTotpError(null)
                }}
              />
            </label>
          )}
          <button
            className="button amber"
            type="submit"
            disabled={busy || (isBackup ? backupCode.trim().length === 0 : totpCode.trim().length !== 6)}
          >
            {busy ? 'Verifying…' : 'Verify'}
          </button>
        </form>
        <div className="button-row" style={{ marginTop: 12 }}>
          <button className="button" onClick={() => startTotpStep(isBackup ? 'totp' : 'backup')} disabled={busy}>
            {isBackup ? 'Use authenticator code instead' : 'Use backup code'}
          </button>
          <button
            className="button"
            onClick={() => {
              setMode('password')
              setTotpError(null)
            }}
            disabled={busy}
          >
            ← Back
          </button>
        </div>
      </Card>
    )
  }

  return (
    <Card
      title="Welcome back"
      copy="Sign in to unlock your encrypted vault."
    >
      {passwordError && <div className="notice" style={{ marginBottom: 14 }}>{passwordError}</div>}
      <form onSubmit={submitPasswordForm} style={{ marginTop: 16 }}>
        <Field label="Email" type="email" value={email} onChange={setEmail} disabled={busy} placeholder="you@example.com" />
        <Field label="Password" type="password" value={password} onChange={setPassword} disabled={busy} placeholder="Your password" />
        <button className="button amber" type="submit" disabled={!email || !password || busy}>
          {busy ? 'Signing in…' : 'Sign in'}
        </button>
      </form>
    </Card>
  )
}

/**
 * R8: another account is signing in on this Mac. Nothing has changed yet. "Sign out and switch"
 * is the full sign-out (Finder entry removed, queue and cache purged), then a fresh sign-in.
 * A failed sign-out is a toast (an action that gates nothing more than itself).
 *
 * Drawn in design/hifi/macos-settings-dialogs.html §4 (onboarding-window variant): no close, the
 * confirm is the filled destructive button, and focus is on Cancel when the step opens, so a stray
 * Enter cancels and can never confirm.
 */
function AccountSwitchStep({ pendingChanges, onSwitched, onCancel }: { pendingChanges: number; onSwitched: () => void; onCancel: () => void }) {
  const { showToast } = useToast()
  const [busy, setBusy] = useState(false)
  const cancelRef = useRef<HTMLButtonElement | null>(null)
  useEffect(() => {
    cancelRef.current?.focus()
  }, [])
  const switchAccount = async () => {
    setBusy(true)
    const result = await command<void>('clear_session')
    setBusy(false)
    if (!result.ok) {
      showToast({ variant: 'error', title: ACCOUNT_SWITCH_FAILED, message: result.unsupported ? commandUnavailableLabel('clear_session') : result.reason })
      return
    }
    onSwitched()
  }
  return (
    <Card title={ACCOUNT_SWITCH_TITLE} copy={accountSwitchBody(pendingChanges)}>
      <div className="button-row" style={{ marginTop: 16 }}>
        <button ref={cancelRef} className="button" onClick={onCancel} disabled={busy}>
          {ACCOUNT_SWITCH_CANCEL}
        </button>
        <button className="button danger filled" onClick={() => void switchAccount()} disabled={busy}>
          {ACCOUNT_SWITCH_CONFIRM}
        </button>
      </div>
    </Card>
  )
}

function UnlockStep({ onDone }: { onDone: () => void }) {
  const [recoveryWords, setRecoveryWords] = useState<string[]>(() =>
    Array.from({ length: RECOVERY_WORD_COUNT }, () => ''),
  )
  const [busy, setBusy] = useState(false)
  const [result, setResult] = useState<CommandResult<void> | null>(null)
  const wordRefs = useRef<Array<HTMLInputElement | null>>([])

  const recoveryPhrase = recoveryWords.map((word) => word.trim()).join(' ').trim()
  const filledWords = recoveryWords.filter((word) => word.trim()).length
  const canUnlock = filledWords === RECOVERY_WORD_COUNT

  const unlock = async (event: FormEvent) => {
    event.preventDefault()
    if (!canUnlock) return
    setBusy(true)
    const next = await command<void>('desktop_unlock_with_recovery_phrase', { recoveryPhrase })
    setResult(next)
    setBusy(false)
    if (next.ok) onDone()
  }

  const applyWords = (startIndex: number, rawWords: string[]) => {
    const cleanWords = rawWords.map((word) => word.trim()).filter(Boolean)
    if (cleanWords.length === 0) return
    // Match the “paste all 12 words into any box” promise.
    if (cleanWords.length === RECOVERY_WORD_COUNT) startIndex = 0

    setResult(null)
    setRecoveryWords((current) => {
      const next = [...current]
      cleanWords.slice(0, RECOVERY_WORD_COUNT - startIndex).forEach((word, offset) => {
        next[startIndex + offset] = word
      })
      return next
    })

    const nextIndex = Math.min(startIndex + cleanWords.length, RECOVERY_WORD_COUNT - 1)
    requestAnimationFrame(() => wordRefs.current[nextIndex]?.focus())
  }

  const updateWord = (index: number, value: string) => {
    if (/\s/.test(value)) {
      applyWords(index, value.split(/\s+/))
      return
    }

    setResult(null)
    setRecoveryWords((current) => current.map((word, wordIndex) => (wordIndex === index ? value : word)))
  }

  const onWordKeyDown = (event: KeyboardEvent<HTMLInputElement>, index: number) => {
    if (event.key === ' ' || event.key === 'Enter') {
      event.preventDefault()
      wordRefs.current[Math.min(index + 1, RECOVERY_WORD_COUNT - 1)]?.focus()
      return
    }
    if (event.key === 'Backspace' && !recoveryWords[index] && index > 0) {
      // Cancel deletion before focus moves: WebView2 applies it to the new input.
      event.preventDefault()
      const previous = wordRefs.current[index - 1]
      previous?.focus()
      previous?.setSelectionRange(previous.value.length, previous.value.length)
    }
  }

  return (
    <Card
      title="Set up this Mac"
      copy="This Mac does not have your encryption keys yet. Restore them to continue."
    >
      {result && !result.ok && (
        <div className="notice">
          {result.unsupported ? commandUnavailableLabel('desktop_unlock_with_recovery_phrase') : result.reason}
        </div>
      )}
      <form onSubmit={unlock} style={{ marginTop: 16 }}>
        <div className="field-label" style={{ marginBottom: 8 }}>
          Recovery phrase
        </div>
        <div className="recovery-grid">
          {recoveryWords.map((word, index) => (
            <label className="recovery-word" key={index}>
              <span className="word-index">{index + 1}</span>
              <input
                ref={(node) => {
                  wordRefs.current[index] = node
                }}
                aria-label={`Recovery word ${index + 1}`}
                autoCapitalize="none"
                autoComplete="off"
                autoCorrect="off"
                className="recovery-input"
                disabled={busy}
                spellCheck={false}
                type="text"
                value={word}
                onChange={(event) => updateWord(index, event.currentTarget.value)}
                onKeyDown={(event) => onWordKeyDown(event, index)}
                onPaste={(event) => {
                  event.preventDefault()
                  applyWords(index, event.clipboardData.getData('text').split(/\s+/))
                }}
              />
            </label>
          ))}
        </div>
        <div className="row-detail" style={{ marginTop: 10, marginBottom: 14 }}>
          Paste all 12 words into any box or type them one by one. Beebeeb stores the unlocked vault
          key in macOS Keychain for future unlocks.
        </div>
        <button className="button amber" type="submit" disabled={busy || !canUnlock}>
          {busy ? 'Unlocking…' : 'Unlock vault'}
        </button>
      </form>
      <div className="button-row" style={{ marginTop: 12 }}>
        {result && !result.ok && result.unsupported && (
          <button className="button" onClick={onDone}>
            Continue to Finder setup
          </button>
        )}
      </div>
    </Card>
  )
}

/**
 * macOS (spec 2026-10-06 §10, ruling R5): no install button. Beebeeb adds itself once the keys
 * arrive; this step shows Adding, advances by itself on Ready, and on a failure or a turned-off
 * extension shows the one notice and the one action of finderSetupCopy.ts. The notice gates the
 * step, so it is inline; a failed ACTION gates nothing, so `useFinderSetup` raises it as a toast.
 *
 * The load, the `finder-setup-changed` subscription and that toast live in `useFinderSetup`
 * (lead ruling 7b); this component only draws what the hook presents.
 */
function MacFinderStep({ onDone }: { onDone: () => void }) {
  const finder = useFinderSetup()
  const [busy, setBusy] = useState(false)
  const presentation = finder.presentation
  const ready = presentation.kind === 'ready'

  useEffect(() => {
    if (ready) onDone()
  }, [ready, onDone])

  const act = async (send: () => Promise<unknown>) => {
    setBusy(true)
    try {
      await send()
    } finally {
      setBusy(false)
    }
  }

  // Two different "Try again"s, never crossed (lead rulings 7a / c5). A failure notice asks the
  // reconciler to check again (`run`); the unreadable state has nothing to ask yet, so its one
  // action only reads the state again (`retry`).
  const notice =
    presentation.kind === 'notice'
      ? {
          tone: presentation.tone,
          sentence: presentation.sentence,
          actionLabel: presentation.actionLabel,
          send: () => finder.run(presentation.action),
        }
      : presentation.kind === 'unavailable'
        ? { tone: 'alert' as const, sentence: presentation.line, actionLabel: presentation.actionLabel, send: () => finder.retry() }
        : null

  return (
    <Card title={FINDER_SETUP_TITLE} copy="Beebeeb appears as a system-managed Finder location. Offline folders are controlled separately.">
      {notice ? (
        <div
          className={notice.tone === 'alert' ? 'notice error' : 'notice'}
          role={notice.tone === 'alert' ? 'alert' : 'status'}
          data-error-surface={notice.tone === 'alert' ? 'finder-setup' : undefined}
          style={{ marginTop: 16 }}
        >
          <div>{notice.sentence}</div>
          <div className="button-row" style={{ marginTop: 10 }}>
            <button className="button" onClick={() => void act(notice.send)} disabled={busy}>
              {notice.actionLabel}
            </button>
          </div>
        </div>
      ) : presentation.kind === 'adding' || presentation.kind === 'ready' ? (
        // Only what the reconciler said. Before its first answer, and while it says Missing, there
        // is nothing true to show yet, so nothing is shown (never "Adding" on a guess).
        <div className="panel" style={{ marginTop: 16, background: 'var(--paper-2)' }}>
          <div className="mono" style={{ fontSize: 13 }}>
            {presentation.line}
          </div>
        </div>
      ) : null}
    </Card>
  )
}

/**
 * Windows/Linux only since spec 2026-10-06 (macOS renders MacFinderStep). The macOS-only
 * branches (the turned-off card, its poll, the System Settings link, the macOS copy) are gone;
 * on these platforms they never rendered.
 */
function FinderInstallStep({ onDone }: { onDone: () => void }) {
  const [syncRoot, setSyncRoot] = useState<string | null>(null)
  const [finderPath, setFinderPath] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  // KEPT INLINE BY DESIGN — task 1255, lead ruling 2026-08-31 ("option (a)").
  //
  // DO NOT convert this to a Toast and delete the state. `message` is not decoration:
  // it GATES the "Continue without install" button in the button row below. Delete this
  // state and that button disappears from the product — a user whose Finder install
  // fails is left with no way past this onboarding step.
  //
  // All three setMessage sites (the pick_sync_root, install_finder_location and
  // continue_without_finder_location failures) feed that same gate, so they stay
  // together; splitting only some of them leaves the gate driven by a subset of its
  // causes, which is worse than either alternative.
  //
  // Same load-bearing shape as WindowsFirstRun's UnlockStep and its "Continue without
  // unlock" escape hatch. See the 1255 task file for the full ruling and the
  // `escapeHatchVisible: true` evidence.
  const [message, setMessage] = useState<string | null>(null)

  useEffect(() => {
    command<string>('default_sync_root').then((result) => {
      if (result.ok) setSyncRoot(result.value)
    })
    command<FinderInstallState>('finder_location_state').then((result) => {
      if (result.ok) setFinderPath(result.value.path ?? null)
    })
  }, [])

  const chooseFolder = useCallback(async () => {
    setBusy(true)
    setMessage(null)
    const picked = await command<string | null>('pick_sync_root')
    setBusy(false)
    if (picked.ok && picked.value) {
      setSyncRoot(picked.value)
      return
    }
    if (!picked.ok) setMessage(picked.unsupported ? commandUnavailableLabel('pick_sync_root') : picked.reason)
  }, [])

  const install = useCallback(async () => {
    setBusy(true)
    setMessage(null)
    const result = await command<FinderInstallState>('install_finder_location', { path: syncRoot })
    setBusy(false)
    const outcome = classifyFinderInstallResult(result)
    if (outcome.kind === 'installed') {
      setFinderPath(outcome.path)
      onDone()
      return
    }
    setMessage(!result.ok && result.unsupported ? commandUnavailableLabel('install_finder_location') : outcome.message)
  }, [onDone, syncRoot])

  const continueWithoutInstall = useCallback(async () => {
    setBusy(true)
    setMessage(null)
    const result = await command<void>('continue_without_finder_location', { path: syncRoot })
    setBusy(false)
    if (result.ok) {
      onDone()
      return
    }
    setMessage(result.unsupported ? commandUnavailableLabel('continue_without_finder_location') : result.reason)
  }, [onDone, syncRoot])

  return (
    <Card
      title="Install the Finder location"
      copy="Beebeeb should appear as a file-manager location. This is separate from choosing optional offline folders."
    >
      {message && <div className="notice">{message}</div>}
      <div className="panel" style={{ marginTop: 16, background: 'var(--paper-2)' }}>
        <div className="section-label">Folder path</div>
        <div className="mono" style={{ marginTop: 8, fontSize: 13 }}>
          {finderPath ?? syncRoot ?? '~/Beebeeb'}
        </div>
      </div>
      <div className="button-row" style={{ marginTop: 16 }}>
        <button className="button" onClick={chooseFolder} disabled={busy}>
          Choose location
        </button>
        <button className="button amber" onClick={install} disabled={busy}>
          {busy ? 'Installing…' : 'Install Finder location'}
        </button>
        {/* This escape hatch EXISTS ONLY while `message` is set — it is the gate described
            on the `message` state above. Removing the inline error removes this button.
            Read that comment before refactoring either one. */}
        {message && (
          <button className="button" onClick={continueWithoutInstall} disabled={busy}>
            Continue without install
          </button>
        )}
      </div>
    </Card>
  )
}

function PinningStep({ onDone }: { onDone: () => void }) {
  const [items, setItems] = useState<VaultItem[]>([])
  const [loading, setLoading] = useState(true)
  // `notice` is deliberately kept for the LOAD failure only: when `list_remote_tree`
  // fails we fall back to `list_vault_folders`, so the list on screen is degraded and
  // needs a persistent explanation. The toggle failure below is a transient ACTION
  // failure and goes to a Toast instead.
  const [notice, setNotice] = useState<string | null>(null)
  const { showToast } = useToast()

  useEffect(() => {
    let cancelled = false
    command<VaultItem[]>('list_remote_tree').then(async (tree) => {
      if (cancelled) return
      if (tree.ok) {
        setItems(tree.value)
        setLoading(false)
        return
      }
      const topLevel = await command<VaultItem[]>('list_vault_folders')
      if (cancelled) return
      if (topLevel.ok) setItems(topLevel.value)
      setNotice(
        tree.unsupported
          ? commandUnavailableLabel('list_remote_tree')
          : tree.reason,
      )
      setLoading(false)
    })
    return () => {
      cancelled = true
    }
  }, [])

  const togglePin = async (item: VaultItem) => {
    const nextPinned = !item.pinned
    const result = await command<void>('set_recursive_pin', {
      itemId: item.id,
      pinned: nextPinned,
    })
    if (!result.ok) {
      showToast({
        variant: 'error',
        title: nextPinned ? 'Couldn’t make folder offline' : 'Couldn’t make folder online-only',
        message: result.unsupported ? commandUnavailableLabel('set_recursive_pin') : result.reason,
      })
      return
    }
    setItems((current) =>
      current.map((entry) => (entry.id === item.id ? { ...entry, pinned: nextPinned } : entry)),
    )
  }

  return (
    <Card
      title="Start online-only"
      copy="No folders are pinned by default. You can make any folder recursively available offline now or later from the control center."
    >
      {notice && <div className="notice">{notice}</div>}
      {loading ? (
        <div className="empty-state" style={{ marginTop: 16 }}>
          Loading remote tree…
        </div>
      ) : items.length === 0 ? (
        <div className="empty-state" style={{ marginTop: 16 }}>
          No remote folders are available yet. Onboarding will continue with everything online-only.
        </div>
      ) : (
        <div className="tree" style={{ marginTop: 16 }}>
          {items
            .filter((item) => item.is_folder)
            .map((item) => (
              <div className="tree-row" key={item.id}>
                <div>
                  <div className="row-title">{item.name}</div>
                  <div className="row-detail">Recursive offline availability</div>
                </div>
                <button className="button" onClick={() => void togglePin(item)}>
                  {item.pinned ? 'Pinned' : 'Online-only'}
                </button>
              </div>
            ))}
        </div>
      )}
      <div className="button-row" style={{ marginTop: 18 }}>
        <button className="button amber" onClick={onDone}>
          Continue with no pinned folders
        </button>
      </div>
    </Card>
  )
}

function ReadyStep() {
  const [status, setStatus] = useState<SyncStatus | null>(null)

  useEffect(() => {
    void loadSyncStatus().then(setStatus)
  }, [])

  const finish = async () => {
    await command<void>('show_main_app_window')
    try {
      const win = getCurrentWindow()
      await win.close()
    } catch {
      window.close()
    }
  }

  return (
    <Card
      title="Control center is ready"
      copy="Use the app for sync health, lock state, offline folders, shared roots, versions, conflicts, and diagnostics. Finder remains the file surface."
    >
      <div className="grid three" style={{ marginTop: 16 }}>
        <div className="metric">
          <div className="metric-label">Engine</div>
          <div className="metric-value">{status?.engine ?? 'Unknown'}</div>
        </div>
        <div className="metric">
          <div className="metric-label">Queue</div>
          <div className="metric-value">{status?.syncing ?? 0}</div>
        </div>
        <div className="metric">
          <div className="metric-label">Conflicts</div>
          <div className="metric-value">{status?.conflicts ?? 0}</div>
        </div>
      </div>
      <button className="button amber" onClick={() => void finish()} style={{ marginTop: 18 }}>
        Open control center
      </button>
    </Card>
  )
}
