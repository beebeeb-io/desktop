/**
 * The signed-out content of the Beebeeb window on a Mac (task 1683 slice 2; spec
 * `docs/specs/2026-10-24-macos-two-surface-app.md` §7.4, §7.5, §8; spec C `…-first-run-sign-in.md` §7.1 and §7.2; hi-fi
 * `design/hifi/macos-two-surface.html` section 2).
 *
 * One centred card at a time, no sidebar (the window shows its sidebar only when there is a session to show). The cards:
 * sign in (browser first, password second), the browser code, the email form and the second factor, Sign in again (R8),
 * the account switch (1887, A0s to A4s), set up this Mac (the recovery phrase), and the Finder status. After the Finder
 * status the window moves on to Home; nothing here closes it. `SignInWindow` picks the card from `signInModel`'s
 * screen, and every card reports what it settled; `MacApp` owns the flow (it lives in a store, because a sign-in
 * remounts the window).
 *
 * Presentation is the hi-fi's `.scard`. The logic is what `Onboarding.tsx`'s steps do, and Linux keeps using those, so
 * the modules they share stay shared: `onboardingSignIn.ts` (password and second factor), `signOutCopy.ts` and
 * `accountSwitchCopy.ts` (1887), `browserLoginCopy.ts` and `browserSignIn.ts` (the handoff), `finderSetup.ts` (spec A).
 *
 * The components are plain function declarations that read only their props, hooks and the bindings imported here, so
 * `tests/macSignIn.test.tsx` can execute them with the component harness.
 */
import { useEffect, useRef, useState, type FormEvent, type KeyboardEvent, type ReactNode } from 'react'
import {
  ACCOUNT_SWITCH_CANCEL,
  ACCOUNT_SWITCH_CONFIRM,
  ACCOUNT_SWITCH_FAILED,
  ACCOUNT_SWITCH_TITLE,
  KEY_REPLACED_RECOVERY_COPY,
  accountSwitchBody,
} from './accountSwitchCopy'
import {
  BACKUP_COPY,
  BACKUP_LABEL,
  BACK_BUTTON,
  BROWSER_CONNECTING_BUTTON,
  BROWSER_DID_NOT_OPEN,
  BROWSER_FINISHING_BUTTON,
  BROWSER_WAITING_BUTTON,
  COPIED_LINK,
  COPY_LINK,
  CREATE_ACCOUNT,
  EMAIL_LABEL,
  EMAIL_PLACEHOLDER,
  FINDER_CARD_COPY,
  FINDER_CARD_TITLE,
  NEW_TO_BEEBEEB,
  OFFLINE_LINE,
  PASSWORD_COPY,
  PASSWORD_LABEL,
  PASSWORD_PLACEHOLDER,
  PASSWORD_TITLE,
  QUIT_BEEBEEB,
  RECOVERY_PHRASE_LABEL,
  SET_UP_CONTINUE,
  SET_UP_COPY,
  SET_UP_HINT,
  SET_UP_TITLE,
  SIGN_IN_AGAIN_COPY,
  SIGN_IN_AGAIN_TITLE,
  SIGN_IN_BUTTON,
  SIGN_IN_CODE_LABEL,
  SIGN_IN_TITLE,
  SIGN_IN_WITH_BROWSER,
  SIGN_IN_WITH_BROWSER_INSTEAD,
  SIGNING_IN_BUTTON,
  TOTP_COPY,
  TOTP_LABEL,
  TOTP_TITLE,
  TRY_AGAIN,
  UNLOCK_VAULT_BUTTON,
  UNLOCKING_BUTTON,
  USE_AUTHENTICATOR_CODE,
  USE_BACKUP_CODE,
  USE_EMAIL_AND_PASSWORD,
  VERIFYING_BUTTON,
  VERIFY_BUTTON,
  signedInBeforeAs,
} from './accountViewCopy'
import { BROWSER_DID_NOT_OPEN_HINT, browserWaitingInstruction } from './browserLoginCopy'
import { useBrowserSignIn } from './browserSignIn'
import {
  CREATE_ACCOUNT_URL,
  clearSession,
  command,
  commandUnavailableLabel,
  lastSignedInEmail,
  openUrl,
  type CommandResult,
} from './desktopApi'
import { finderActionButtonLabel, FINDER_COPIED_MS } from './finderSetupCopy'
import { useFinderSetup, writeTextToPasteboard } from './finderSetup'
import { Btn, Note } from './macSettingsParts'
import { submitPassword, submitTotpCode, type SignInSettled } from './onboardingSignIn'
import {
  afterFinderReady,
  afterSignIn,
  afterSwitchCancelled,
  afterSwitched,
  afterUnlocked,
  type SignInFlow,
  type SwitchInfo,
  type WindowScreen,
} from './signInModel'
import { A0S_TITLE, A1S_LINE, askLines, stripCode, type AskCounts, type SignOutCount } from './signOutCopy'
import { setMainWindowTitle } from './windowTitle'
import { useToast } from './windows/ui'

const RECOVERY_WORD_COUNT = 12

// ── The card and the look of its buttons ─────────────────────────────────────

/** The window's one button look: the Settings buttons (`ms-btn`), amber for the one primary action, red to confirm. */
export function btnClass(variant: 'plain' | 'primary' | 'danger', full = false): string {
  const base = variant === 'primary' ? 'ms-btn ms-btn--primary' : variant === 'danger' ? 'ms-btn ms-btn--danger' : 'ms-btn'
  return full ? `${base} mw-btn--full` : base
}

/** The device-code page's address as a person reads it: without the scheme (hi-fi figure 2). "Copy link" copies it whole. */
export function displayAddress(address: string): string {
  return address.replace(/^https?:\/\//, '')
}

/** The window's title follows the heading of the card shown (spec C C-W8). */
export function useWindowTitle(title: string) {
  useEffect(() => {
    void setMainWindowTitle(title)
  }, [title])
}

/** The hi-fi's `.scard`: the mark, the heading, the words, then the card's own body. */
export function SignInCard({
  title,
  copy,
  size = 'base',
  children,
}: {
  title: string
  copy?: string | null
  size?: 'base' | 'wide' | 'switch'
  children?: ReactNode
}) {
  return (
    <section className={`mw-scard mw-scard--${size}`} aria-labelledby="mw-title">
      <div className="mw-mark" aria-hidden="true">b</div>
      <h1 id="mw-title" className="mw-scard-title">{title}</h1>
      {copy ? <p className="mw-scard-copy">{copy}</p> : null}
      {children}
    </section>
  )
}

// ── The router ───────────────────────────────────────────────────────────────

export function SignInWindow({
  screen,
  email,
  flow,
  note,
  offline,
  onFlow,
}: {
  screen: WindowScreen
  /** The account on this Mac, for “Signed in before as …”. */
  email: string | null
  flow: SignInFlow
  /** A sign-out's held sentence (spec §8.1), shown above the card. */
  note: string | null
  /** Spec C C-R7 (task 1747's account view): the sign-in screen says it cannot reach Beebeeb. Not set until then. */
  offline?: boolean
  onFlow: (next: SignInFlow) => void
}) {
  if (screen === 'settling') {
    return (
      <div className="mw-solo-body">
        <div className="ms-loading" role="status">Loading…</div>
      </div>
    )
  }
  let card: ReactNode = null
  if (screen === 'signin' || screen === 'reauth') {
    const reauth = screen === 'reauth'
    card = (
      <SignInScreen
        reauth={reauth}
        email={email}
        prefillEmail={flow.email}
        offline={offline}
        onSettled={(settled, typedEmail) => onFlow(afterSignIn(settled, typedEmail, reauth))}
      />
    )
  } else if (screen === 'switch') {
    card = (
      <SwitchScreen
        info={flow.switch ?? { pendingChanges: 0, unsent: null, unsentError: null }}
        onSwitched={() => onFlow(afterSwitched(flow))}
        onCancel={() => onFlow(afterSwitchCancelled())}
      />
    )
  } else if (screen === 'unlock') {
    card = <SetUpScreen keyReplaced={flow.keyReplaced} onDone={() => onFlow(afterUnlocked())} />
  } else if (screen === 'finder') {
    card = <FinderScreen onReady={() => onFlow(afterFinderReady())} />
  }
  return (
    <div className="mw-solo-body">
      {note ? (
        <div className="mw-solo-note">
          <Note kind="status">{note}</Note>
        </div>
      ) : null}
      {card}
    </div>
  )
}

/** Browser first, password second (C2). An address typed before an account switch opens the form it was typed in. */
export function SignInScreen({
  reauth,
  email,
  prefillEmail,
  offline = false,
  onSettled,
}: {
  reauth: boolean
  email: string | null
  prefillEmail: string | undefined
  offline?: boolean
  onSettled: (settled: SignInSettled, typedEmail: string) => void
}) {
  const [method, setMethod] = useState<'browser' | 'password'>(() => (prefillEmail ? 'password' : 'browser'))
  if (method === 'password') {
    return <PasswordPanel prefillEmail={prefillEmail} onSettled={onSettled} onUseBrowser={() => setMethod('browser')} />
  }
  return (
    <BrowserPanel
      reauth={reauth}
      email={email}
      offline={offline}
      onSettled={onSettled}
      onUsePassword={() => setMethod('password')}
    />
  )
}

// ── Sign in with the browser (task 1734; spec C §7.1) ────────────────────────

/** Spec A's “running from the disk image” sentence and action, before sign-in (spec C §7.1). Nothing else of Finder. */
export function DiskImageNotice() {
  const finder = useFinderSetup()
  const presentation = finder.presentation
  if (presentation.kind !== 'notice' || presentation.reason !== 'not_in_applications') return null
  const action = presentation.action
  return (
    <Note
      kind={presentation.tone}
      surface="finder-setup"
      actions={
        action ? (
          <Btn onClick={() => void finder.run(action)}>{finderActionButtonLabel(action, presentation.actionLabel, finder.copied)}</Btn>
        ) : null
      }
    >
      {presentation.sentence}
    </Note>
  )
}

export function BrowserPanel({
  reauth,
  email,
  offline,
  onSettled,
  onUsePassword,
}: {
  reauth: boolean
  email: string | null
  offline: boolean
  onSettled: (settled: SignInSettled, typedEmail: string) => void
  onUsePassword: () => void
}) {
  const { state, start } = useBrowserSignIn({ onSettled: (settled) => onSettled(settled, '') })
  const [copied, setCopied] = useState(false)
  const [createFailed, setCreateFailed] = useState(false)
  const copiedTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  useWindowTitle(reauth ? SIGN_IN_AGAIN_TITLE : SIGN_IN_TITLE)
  useEffect(
    () => () => {
      if (copiedTimer.current !== null) clearTimeout(copiedTimer.current)
    },
    [],
  )

  const busy = state.phase === 'connecting' || state.phase === 'waiting' || state.phase === 'authorized'
  const showCode = state.phase === 'waiting' || state.phase === 'authorized'
  const primaryLabel =
    state.phase === 'connecting'
      ? BROWSER_CONNECTING_BUTTON
      : state.phase === 'waiting'
        ? BROWSER_WAITING_BUTTON
        : state.phase === 'authorized'
          ? BROWSER_FINISHING_BUTTON
          : state.phase === 'error'
            ? TRY_AGAIN
            : SIGN_IN_WITH_BROWSER

  const copyLink = async (url: string) => {
    try {
      await writeTextToPasteboard(Promise.resolve(url))
    } catch {
      // The address stays on screen to select; a refused pasteboard needs no second message.
      return
    }
    setCopied(true)
    if (copiedTimer.current !== null) clearTimeout(copiedTimer.current)
    copiedTimer.current = setTimeout(() => setCopied(false), FINDER_COPIED_MS)
  }
  const copyLabel = copied ? COPIED_LINK : COPY_LINK

  const openCreateAccount = async (event: { preventDefault: () => void }) => {
    event.preventDefault()
    const opened = await openUrl(CREATE_ACCOUNT_URL)
    if (!opened.ok) setCreateFailed(true)
  }

  return (
    <SignInCard title={reauth ? SIGN_IN_AGAIN_TITLE : SIGN_IN_TITLE} copy={reauth && !showCode ? SIGN_IN_AGAIN_COPY : null}>
      {!busy ? <DiskImageNotice /> : null}
      {showCode ? (
        <>
          <p className="mw-scard-copy">{browserWaitingInstruction(state.phase === 'authorized' ? 'authorized' : 'waiting')}</p>
          {state.userCode ? (
            <div className="mw-code" aria-label={SIGN_IN_CODE_LABEL}>{state.userCode}</div>
          ) : null}
          {state.verificationUri && state.phase === 'waiting' ? (
            <p className="mw-scard-note">
              {BROWSER_DID_NOT_OPEN} <span className="mw-mono">{displayAddress(state.verificationUri)}</span> {BROWSER_DID_NOT_OPEN_HINT}{' '}
              <button type="button" className="mw-link" onClick={() => void copyLink(state.verificationUri!)}>{copyLabel}</button>
            </p>
          ) : null}
        </>
      ) : null}
      {state.phase === 'error' && state.error ? <Note kind="alert" surface="sign-in">{state.error}</Note> : null}
      {offline ? (
        <>
          <p className="mw-scard-copy">{OFFLINE_LINE}</p>
          <button type="button" className={btnClass('plain', true)} onClick={() => void start()}>{TRY_AGAIN}</button>
        </>
      ) : (
        <>
          {/* Amber is the one primary action: while the handoff runs nothing can be pressed, so the button is plain. */}
          <button type="button" className={btnClass(busy ? 'plain' : 'primary', true)} disabled={busy} onClick={() => void start()}>{primaryLabel}</button>
          <div className="mw-center">
            <button type="button" className="mw-link" onClick={onUsePassword}>{USE_EMAIL_AND_PASSWORD}</button>
          </div>
        </>
      )}
      {!showCode ? (
        <div className="mw-scard-foot">
          {reauth ? (
            email ? <span>{signedInBeforeAs(email)}</span> : <span />
          ) : (
            <span>
              {NEW_TO_BEEBEEB}{' '}
              <a className="mw-link" href={CREATE_ACCOUNT_URL} onClick={(event) => void openCreateAccount(event)}>{CREATE_ACCOUNT}</a>
            </span>
          )}
          <button type="button" className="mw-link mw-link--quiet" onClick={() => void command<void>('quit_beebeeb')}>{QUIT_BEEBEEB}</button>
        </div>
      ) : null}
      {createFailed ? (
        <p className="mw-scard-note">
          <span className="mw-mono">{CREATE_ACCOUNT_URL}</span>{' '}
          <button type="button" className="mw-link" onClick={() => void copyLink(CREATE_ACCOUNT_URL)}>{copyLabel}</button>
        </p>
      ) : null}
    </SignInCard>
  )
}

// ── Email and password, then the second factor (task 1521) ──────────────────

/**
 * Today's `SignInStep`, in the window's card. `submitPassword` can resolve `requiresTotp`: the password was right but
 * only a short-lived partial token exists, so the code is asked for in the same card and `submitTotpCode` completes the
 * sign-in; calling `onSettled` before that is the bug task 1521 fixed. The code is six digits, or eight for a backup code.
 */
export function PasswordPanel({
  prefillEmail,
  onSettled,
  onUseBrowser,
}: {
  prefillEmail: string | undefined
  onSettled: (settled: SignInSettled, typedEmail: string) => void
  onUseBrowser: () => void
}) {
  const [mode, setMode] = useState<'password' | 'totp' | 'backup'>('password')
  // An address typed before a switch (FT-I4) is the first value, so it beats the prefill, which only fills an empty field.
  const [email, setEmail] = useState(prefillEmail ?? '')
  const [password, setPassword] = useState('')
  const [busy, setBusy] = useState(false)
  const [passwordError, setPasswordError] = useState<string | null>(null)
  const [totpCode, setTotpCode] = useState('')
  const [backupCode, setBackupCode] = useState('')
  const [totpError, setTotpError] = useState<string | null>(null)
  const codeRef = useRef<HTMLInputElement | null>(null)
  useWindowTitle(mode === 'password' ? PASSWORD_TITLE : TOTP_TITLE)

  // The last account's address, kept in desktop.toml (the Keychain copy is erased by a sign-out). Prefill only: it
  // never overwrites what is being typed, and says nothing when there is none.
  useEffect(() => {
    let cancelled = false
    void lastSignedInEmail().then((result: CommandResult<string | null>) => {
      if (cancelled || !result.ok || !result.value) return
      setEmail((current) => (current ? current : (result.value as string)))
    })
    return () => {
      cancelled = true
    }
  }, [])

  const startSecondFactor = (next: 'totp' | 'backup') => {
    setMode(next)
    setTotpCode('')
    setBackupCode('')
    setTotpError(null)
    requestAnimationFrame(() => codeRef.current?.focus())
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
      startSecondFactor('totp')
      return
    }
    onSettled(result.settled, email)
  }

  const submitCodeForm = async (event: FormEvent) => {
    event.preventDefault()
    const code = mode === 'backup' ? backupCode.trim() : totpCode.trim()
    if (!code) return
    setBusy(true)
    setTotpError(null)
    const result = await submitTotpCode(code)
    setBusy(false)
    if (!result.ok) {
      // A wrong or expired code is retryable within the partial token's window: clear the field, stay on this card.
      setTotpCode('')
      setBackupCode('')
      setTotpError(result.message)
      requestAnimationFrame(() => codeRef.current?.focus())
      return
    }
    onSettled(result.settled, email)
  }

  if (mode === 'totp' || mode === 'backup') {
    const isBackup = mode === 'backup'
    return (
      <SignInCard title={TOTP_TITLE} copy={isBackup ? BACKUP_COPY : TOTP_COPY}>
        {totpError ? <Note kind="alert" surface="sign-in">{totpError}</Note> : null}
        <form className="mw-form" onSubmit={(event) => void submitCodeForm(event)}>
          <label className="mw-field">
            <span className="mw-field-label">{isBackup ? BACKUP_LABEL : TOTP_LABEL}</span>
            <input
              ref={codeRef}
              className="mw-input mw-input--code"
              name="code"
              type="text"
              inputMode="numeric"
              autoComplete={isBackup ? 'off' : 'one-time-code'}
              maxLength={isBackup ? 8 : 6}
              placeholder={isBackup ? '12345678' : '000000'}
              disabled={busy}
              value={isBackup ? backupCode : totpCode}
              onChange={(event) => {
                const digits = event.currentTarget.value.replace(/\D/g, '')
                if (isBackup) setBackupCode(digits.slice(0, 8))
                else setTotpCode(digits.slice(0, 6))
                setTotpError(null)
              }}
            />
          </label>
          <button
            type="submit"
            className={btnClass('primary', true)}
            disabled={busy || (isBackup ? backupCode.trim().length === 0 : totpCode.trim().length !== 6)}
          >
            {busy ? VERIFYING_BUTTON : VERIFY_BUTTON}
          </button>
        </form>
        <div className="mw-scard-links">
          <button type="button" className="mw-link" disabled={busy} onClick={() => startSecondFactor(isBackup ? 'totp' : 'backup')}>
            {isBackup ? USE_AUTHENTICATOR_CODE : USE_BACKUP_CODE}
          </button>
          <button
            type="button"
            className="mw-link"
            disabled={busy}
            onClick={() => {
              setMode('password')
              setTotpError(null)
            }}
          >
            {BACK_BUTTON}
          </button>
        </div>
      </SignInCard>
    )
  }

  return (
    <SignInCard title={PASSWORD_TITLE} copy={PASSWORD_COPY}>
      {passwordError ? <Note kind="alert" surface="sign-in">{passwordError}</Note> : null}
      <form className="mw-form" onSubmit={(event) => void submitPasswordForm(event)}>
        <label className="mw-field">
          <span className="mw-field-label">{EMAIL_LABEL}</span>
          <input
            className="mw-input"
            name="email"
            type="email"
            autoComplete="email"
            placeholder={EMAIL_PLACEHOLDER}
            disabled={busy}
            value={email}
            onChange={(event) => setEmail(event.currentTarget.value)}
          />
        </label>
        <label className="mw-field">
          <span className="mw-field-label">{PASSWORD_LABEL}</span>
          <input
            className="mw-input"
            name="password"
            type="password"
            autoComplete="current-password"
            placeholder={PASSWORD_PLACEHOLDER}
            disabled={busy}
            value={password}
            onChange={(event) => setPassword(event.currentTarget.value)}
          />
        </label>
        <button type="submit" className={btnClass('primary', true)} disabled={!email || !password || busy}>
          {busy ? SIGNING_IN_BUTTON : SIGN_IN_BUTTON}
        </button>
      </form>
      <div className="mw-center">
        <button type="button" className="mw-link" disabled={busy} onClick={onUseBrowser}>{SIGN_IN_WITH_BROWSER_INSTEAD}</button>
      </div>
    </SignInCard>
  )
}

// ── The account switch (task 1887, spec 2026-10-10 W11 and §5.3 row 1) ───────

/**
 * Another account is signing in on this Mac. Nothing has changed yet. “Sign out and switch” is the full sign-out
 * (Finder entry removed, queue and cache purged), then a fresh sign-in; it also forgets the previous account's address
 * (`forgetEmail`, FB-I2). An `Err` means the sign-out did not happen (including `SIGN_OUT_EMAIL_NOT_FORGOTTEN`): its
 * sentence is shown verbatim as a toast and the step stays. Any `Ok` continues, and an `Ok` with a warning (FB-24) shows
 * the warning's sentence as a neutral note.
 *
 * On a Mac the step is the sign-out ask in the switch's words, from the snapshot the sign-in carried: “Sign out and
 * switch” keeps the waiting saves with that snapshot's `askId`; a refusal (`unsent_changes:`) counts again and
 * re-renders the ask, with no toast; a count that cannot be read (E3) is a line in the step, never a toast. A code
 * prefix is never shown. With no snapshot (the browser path without its event, or Linux) the step is spec A's.
 *
 * Drawn in the hi-fi (section 2, “Account switch”): the confirm is the red button, and focus is on Cancel when the step
 * opens, so a stray Enter cancels and can never confirm.
 */
export function SwitchScreen({
  info,
  onSwitched,
  onCancel,
}: {
  info: SwitchInfo
  onSwitched: () => void
  onCancel: () => void
}) {
  const { showToast } = useToast()
  const [busy, setBusy] = useState(false)
  const [ask, setAsk] = useState<AskCounts | null>(info.unsent)
  // Spec 2026-10-10 §5.3 row 1: E3, a line in this step while the count could not be read. Deliberately inline, never a
  // toast: it persists because “Sign out and switch” cannot keep anything until a count succeeds, and a fresh count
  // replaces it with the ask.
  // eslint-disable-next-line beebeeb/no-ad-hoc-error-surface
  const [countError, setCountError] = useState<string | null>(info.unsentError)
  const cancelRef = useRef<HTMLButtonElement | null>(null)
  const waiting = ask !== null && (ask.files > 0 || ask.others > 0)
  useWindowTitle(waiting ? A0S_TITLE : ACCOUNT_SWITCH_TITLE)
  useEffect(() => {
    cancelRef.current?.focus()
  }, [])

  const switchAccount = async () => {
    setBusy(true)
    const result = await clearSession({ forgetEmail: true, keepUnsent: ask ? { askId: ask.askId } : undefined })
    setBusy(false)
    if (!result.ok) {
      // W11: a refusal re-renders the ask from a fresh count, with no toast.
      if (result.reason.startsWith('unsent_changes: ')) {
        const fresh = await command<SignOutCount>('sign_out_count')
        if (fresh.ok) {
          setAsk(fresh.value)
          setCountError(null)
          return
        }
        // E3 again: the line, never a toast. The ask it replaces is stale, and the line is hidden behind an ask, so the
        // ask goes (the shipped step kept it and said nothing); the next confirm carries no snapshot and Rust counts.
        setAsk(null)
        setCountError(fresh.reason)
        return
      }
      // The gate's own read failed: E3 as the line; Rust holds it pending, so it is acknowledged once shown (F12).
      if (result.reason.startsWith('unsent_count_failed: ')) {
        setAsk(null)
        setCountError(result.reason)
        void command<void>('sign_out_result_ack')
        return
      }
      showToast({
        variant: 'error',
        title: ACCOUNT_SWITCH_FAILED,
        message: result.unsupported ? commandUnavailableLabel('clear_session') : stripCode(result.reason),
      })
      void command<void>('sign_out_result_ack')
      return
    }
    // FB-24: the sign-out happened. A step it could not confirm is said neutrally (never under “Couldn’t sign out”),
    // and the note stays until dismissed: this step unmounts as sign-in opens.
    if (result.value.warning) {
      showToast({ variant: 'info', message: result.value.warning.sentence, durationMs: null })
      void command<void>('sign_out_result_ack')
    }
    onSwitched()
  }

  const lines = waiting && ask ? askLines(ask, null, true) : null
  // With no count (E3), the step says only what is known: the switch signs the other account out.
  const copyLine = waiting || countError !== null ? A1S_LINE : accountSwitchBody(ask ? 0 : info.pendingChanges)
  return (
    <SignInCard title={waiting ? A0S_TITLE : ACCOUNT_SWITCH_TITLE} copy={copyLine} size="switch">
      {countError !== null && !waiting ? <Note kind="alert" surface="account-switch">{stripCode(countError)}</Note> : null}
      {lines ? lines.paragraphs.slice(0, lines.foldersAfter).map((line) => <p key={line} className="mw-scard-copy">{line}</p>) : null}
      {lines?.foldersLine ? <p className="mw-scard-copy">{lines.foldersLine}</p> : null}
      {lines ? lines.folders.map((folder) => <p key={folder} className="mw-scard-copy mw-mono">{folder}</p>) : null}
      {lines ? lines.paragraphs.slice(lines.foldersAfter).map((line) => <p key={line} className="mw-scard-copy">{line}</p>) : null}
      <div className="mw-scard-actions">
        <button ref={cancelRef} type="button" className={btnClass('plain')} onClick={onCancel} disabled={busy}>
          {ACCOUNT_SWITCH_CANCEL}
        </button>
        <button type="button" className={btnClass('danger')} onClick={() => void switchAccount()} disabled={busy}>
          {ACCOUNT_SWITCH_CONFIRM}
        </button>
      </div>
    </SignInCard>
  )
}

// ── Set up this Mac: the recovery phrase ─────────────────────────────────────

/**
 * Only when this Mac has no key. Twelve words; a whole phrase pasted into any box fills the grid, and a word with a space
 * in it spreads over the boxes from there. The key is stored in the macOS Keychain for later unlocks.
 */
export function SetUpScreen({ keyReplaced, onDone }: { keyReplaced: boolean; onDone: () => void }) {
  const [recoveryWords, setRecoveryWords] = useState<string[]>(() => Array.from({ length: RECOVERY_WORD_COUNT }, () => ''))
  const [busy, setBusy] = useState(false)
  const [result, setResult] = useState<CommandResult<void> | null>(null)
  const wordRefs = useRef<Array<HTMLInputElement | null>>([])
  useWindowTitle(SET_UP_TITLE)
  useEffect(() => {
    wordRefs.current[0]?.focus()
  }, [])

  const recoveryPhrase = recoveryWords.map((word) => word.trim()).join(' ').trim()
  const canUnlock = recoveryWords.filter((word) => word.trim()).length === RECOVERY_WORD_COUNT

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
    // “Paste all 12 words into any box”: a whole phrase belongs to the whole grid.
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
      // Cancel the deletion before focus moves, or it is applied to the box focus lands in.
      event.preventDefault()
      const previous = wordRefs.current[index - 1]
      previous?.focus()
      previous?.setSelectionRange(previous.value.length, previous.value.length)
    }
  }

  return (
    <SignInCard title={SET_UP_TITLE} copy={keyReplaced ? KEY_REPLACED_RECOVERY_COPY : SET_UP_COPY} size="wide">
      {result && !result.ok ? (
        <Note kind="alert" surface="set-up">
          {result.unsupported ? commandUnavailableLabel('desktop_unlock_with_recovery_phrase') : result.reason}
        </Note>
      ) : null}
      <form className="mw-form" onSubmit={(event) => void unlock(event)}>
        <div className="mw-field">
          <span className="mw-field-label">{RECOVERY_PHRASE_LABEL}</span>
          <div className="mw-words">
            {recoveryWords.map((word, index) => (
              <label className="mw-word" key={index}>
                <span className="mw-word-index" aria-hidden="true">{index + 1}</span>
                <input
                  ref={(node) => {
                    wordRefs.current[index] = node
                  }}
                  aria-label={`Recovery word ${index + 1}`}
                  autoCapitalize="none"
                  autoComplete="off"
                  autoCorrect="off"
                  className="mw-word-input"
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
        </div>
        <p className="mw-scard-note">{SET_UP_HINT}</p>
        <button type="submit" className={btnClass('primary', true)} disabled={busy || !canUnlock}>
          {busy ? UNLOCKING_BUTTON : UNLOCK_VAULT_BUTTON}
        </button>
      </form>
      {result && !result.ok && result.unsupported ? (
        <div className="mw-center">
          <button type="button" className="mw-link" onClick={onDone}>{SET_UP_CONTINUE}</button>
        </div>
      ) : null}
    </SignInCard>
  )
}

// ── Finder (spec A's reconciler; spec §7.5) ──────────────────────────────────

/**
 * A status screen, not a step you complete. Beebeeb adds itself to Finder once the keys arrive; this shows what the
 * reconciler says, moves the window on to Home by itself on Ready, and on a failure or a turned-off extension shows
 * spec A's one sentence and one action inline. There is no “Continue”: Home shows the Finder row's state. Nothing is
 * claimed before the reconciler speaks, and a loaded Missing claims nothing either (never “Adding” on a guess).
 *
 * The load, the `finder-setup-changed` subscription and the failed-action toast live in `useFinderSetup` (lead ruling
 * 7b); this component only draws what the hook presents. A failed ACTION gates nothing, so the hook raises it as a toast;
 * the notice gates the card, so it is inline.
 */
export function FinderScreen({ onReady }: { onReady: () => void }) {
  const finder = useFinderSetup()
  const [busy, setBusy] = useState(false)
  const presentation = finder.presentation
  const ready = presentation.kind === 'ready'
  const movedOn = useRef(false)
  useWindowTitle(FINDER_CARD_TITLE)

  useEffect(() => {
    if (!ready || movedOn.current) return
    movedOn.current = true
    onReady()
  }, [ready, onReady])

  const act = async (send: () => Promise<unknown>) => {
    setBusy(true)
    try {
      await send()
    } finally {
      setBusy(false)
    }
  }

  // Two different “Try again”s, never crossed (lead rulings 7a, c5). A failure notice asks the reconciler to check again
  // (`run`); the unreadable state has nothing to ask yet, so its one action only reads the state again (`retry`). A
  // notice with no action (an unconfirmed engine stop: only a relaunch helps) is its sentence alone.
  const noticeAction = presentation.kind === 'notice' ? presentation.action : null
  const notice =
    presentation.kind === 'notice'
      ? {
          // Spec §7.5 and the hi-fi: a failure and a turned-off extension are both ink on the error wash (the notice's own
          // tone, neutral for "turned off", is what Home's Finder row and Settings › Sync draw; this screen follows the design).
          tone: 'alert' as const,
          sentence: presentation.sentence,
          actionLabel: finderActionButtonLabel(noticeAction, presentation.actionLabel, finder.copied),
          send: noticeAction ? () => finder.run(noticeAction) : null,
        }
      : presentation.kind === 'unavailable'
        ? { tone: 'alert' as const, sentence: presentation.line, actionLabel: presentation.actionLabel, send: () => finder.retry() }
        : null

  return (
    <SignInCard title={FINDER_CARD_TITLE} copy={FINDER_CARD_COPY} size="wide">
      {notice ? (
        <Note
          kind={notice.tone}
          surface="finder-setup"
          actions={
            notice.send ? (
              <Btn disabled={busy} onClick={() => void act(notice.send!)}>{notice.actionLabel}</Btn>
            ) : null
          }
        >
          {notice.sentence}
          {/* Row 15: what a failed Try again said (Rust's fixed sentence, with its remedy). */}
          {finder.actionNote ? <div role="status">{finder.actionNote}</div> : null}
        </Note>
      ) : presentation.kind === 'adding' ? (
        <div className="mw-adding" role="status">
          <span className="mw-spinner" aria-hidden="true" />
          {presentation.line}
        </div>
      ) : null}
    </SignInCard>
  )
}
