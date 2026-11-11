// B10's sign-in flow, exported for B11 (first run) and the gate. One controller over the view components: every step's
// state lives here, every secret leaves for Rust on submit and is cleared from React state at once (crypto review rule
// 9). No `console.*` call anywhere in this directory prints form state.
import { useCallback, useEffect, useRef, useState } from 'react'
import { CREATE_ACCOUNT_URL, openUrl } from '../../desktopApi'
import {
  deviceRequestCancel, deviceRequestPoll, deviceRequestStart, lastSignedInEmail, type LoginOutcome, nextAfterSignIn,
  passkeySignIn, restoreWithPhrase, signIn, signInProblem, unknownPhraseWords, verifySecondFactor,
} from './api'
import { AnotherDeviceView } from './AnotherDevice'
import { type DeviceStatus, type SignInProblem } from './model'
import { ContinueAsView, PasskeyPasswordView, SignInView, type ContinueAsAccount } from './SignIn'
import { FirstRunView, LostPhraseView, PhraseView, TwoFactorView } from './Steps'

export const FORGOT_PASSWORD_URL = 'https://app.beebeeb.io/forgot-password'

type Screen = 'first-run' | 'sign-in' | 'continue-as' | 'another-device' | 'two-factor' | 'phrase' | 'lost-phrase' | 'passkey-password'

export interface SignInFlowProps {
  /** Called once the session is installed with its keys on this PC. */
  onSignedIn: () => void
  /** B11 builds sign-up (DW-A31); until it is wired, the web sign-up opens. */
  onCreateAccount?: () => void
  /** B11 builds recovery (DW-A45); until it is wired, the web page opens. */
  onForgotPassword?: () => void
  /** Start at the sign-in screen instead of first run (a returning person). */
  startAt?: 'first-run' | 'sign-in' | 'another-device'
}

const EMPTY_WORDS = Array.from({ length: 12 }, () => '')

export function SignInFlow({ onSignedIn, onCreateAccount, onForgotPassword, startAt }: SignInFlowProps) {
  const [screen, setScreen] = useState<Screen>(startAt ?? 'first-run')
  // The shared Beebeeb-apps session store (Continue as) is task 2323's: until it exists there is no account to offer.
  const [continueAs] = useState<ContinueAsAccount | null>(null)
  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [busy, setBusy] = useState(false)
  const [problem, setProblem] = useState<SignInProblem | null>(null)
  /** The board's words for the current problem, from Rust (auth_v2::gate::problem_text). */
  const [problemMessage, setProblemMessage] = useState<string | null>(null)
  const [wait, setWait] = useState(0)
  const failures = useRef(0)
  const [passkeyPrompt, setPasskeyPrompt] = useState(false)
  const [twoFactor, setTwoFactor] = useState<{ mode: 'totp' | 'backup'; code: string; state: 'idle' | 'verifying' | 'wrong' }>({ mode: 'totp', code: '', state: 'idle' })
  const [words, setWords] = useState<string[]>(EMPTY_WORDS)
  const [phraseState, setPhraseState] = useState<'idle' | 'restoring' | 'wrong' | 'too-many'>('idle')
  const [unknown, setUnknown] = useState<number[]>([])
  const [device, setDevice] = useState<{ status: DeviceStatus | 'starting'; code: string | null; secondsLeft: number; deviceName: string | null; qr: string | null }>(
    { status: 'starting', code: null, secondsLeft: 300, deviceName: null, qr: null },
  )

  // A returning person starts at sign-in with the last email filled.
  useEffect(() => {
    void lastSignedInEmail().then((r) => {
      if (r.ok && r.value) { setEmail(r.value); setScreen((s) => (s === 'first-run' ? 'sign-in' : s)) }
    })
  }, [])

  // DW-A21: the button counts down and re-enables itself.
  useEffect(() => {
    if (wait <= 0) return
    const t = window.setTimeout(() => setWait((w) => w - 1), 1000)
    return () => window.clearTimeout(t)
  }, [wait])

  // Clear every secret when the flow goes away (rule 9).
  useEffect(() => () => { setPassword(''); setWords(EMPTY_WORDS) }, [])

  /** Decide the next screen from a sign-in outcome. The decision lives in Rust (auth_v2::gate). */
  const settle = useCallback(async (outcome: LoginOutcome) => {
    const r = await nextAfterSignIn(outcome)
    const next = r.ok ? r.value : 'recovery_phrase'
    if (next === 'two_factor') { setTwoFactor({ mode: 'totp', code: '', state: 'idle' }); setScreen('two-factor'); return }
    if (next === 'recovery_phrase') { setScreen('phrase'); return }
    // drive, reauthenticated
    onSignedIn()
  }, [onSignedIn])

  const submit = useCallback(async () => {
    const secret = password
    setBusy(true)
    setProblem(null)
    const result = await signIn(email, secret)
    setBusy(false)
    if (result.ok) {
      failures.current = 0
      setPassword('')
      void settle(result.value)
      return
    }
    if (/invalid/i.test(result.reason)) failures.current += 1
    // The error mapping lives in Rust (auth_v2::gate::problem_from).
    const r = await signInProblem(result.reason, failures.current)
    if (r.ok) {
      setProblem(r.value.problem)
      setProblemMessage(r.value.text)
      if (r.value.wait_seconds) setWait(r.value.wait_seconds)
    } else {
      setProblem({ kind: 'other' })
      setProblemMessage(null)
    }
  }, [email, password, settle])

  // ── Another device ──
  const startDevice = useCallback(async () => {
    setScreen('another-device')
    setDevice({ status: 'starting', code: null, secondsLeft: 300, deviceName: null, qr: null })
    const r = await deviceRequestStart()
    if (!r.ok) { setDevice((d) => ({ ...d, status: 'failed', secondsLeft: 0 })); return }
    setDevice({ status: r.value.status, code: r.value.user_code, secondsLeft: r.value.seconds_left, deviceName: r.value.device_name, qr: r.value.qr_svg ?? null })
  }, [])
  useEffect(() => {
    if (screen !== 'another-device' || device.status === 'starting') return
    if (!['waiting', 'reconnecting', 'approved'].includes(device.status)) return
    const t = window.setInterval(() => {
      void deviceRequestPoll().then((r) => {
        if (r.ok) setDevice((d) => ({ ...d, status: r.value.status, code: r.value.user_code, secondsLeft: r.value.seconds_left }))
      })
    }, 1000)
    return () => window.clearInterval(t)
  }, [screen, device.status])
  const leaveDevice = useCallback((to: Screen) => { void deviceRequestCancel(); setScreen(to) }, [])

  // ── Two-factor ──
  const verify = useCallback(async (code: string) => {
    setTwoFactor((t) => ({ ...t, code, state: 'verifying' }))
    const r = await verifySecondFactor(code)
    if (r.ok) { setTwoFactor({ mode: 'totp', code: '', state: 'idle' }); void settle(r.value); return }
    setTwoFactor((t) => ({ ...t, code: '', state: 'wrong' }))
  }, [settle])

  // ── The 12 words ──
  const restore = useCallback(async () => {
    setPhraseState('restoring')
    setUnknown([])
    const phrase = words.join(' ')
    const r = await restoreWithPhrase(phrase)
    if (r.ok) { setWords(EMPTY_WORDS); setPhraseState('idle'); onSignedIn(); return }
    if (/too many/i.test(r.reason)) { setPhraseState('too-many'); return }
    const marks = await unknownPhraseWords(words)
    setUnknown(marks.ok ? marks.value : [])
    setPhraseState('wrong')
  }, [words, onSignedIn])

  const create = onCreateAccount ?? (() => void openUrl(CREATE_ACCOUNT_URL))
  const forgot = onForgotPassword ?? (() => void openUrl(FORGOT_PASSWORD_URL))
  const passkey = useCallback(async () => {
    setPasskeyPrompt(true)
    const r = await passkeySignIn(email)
    setPasskeyPrompt(false)
    if (!r.ok) { setProblem({ kind: 'other' }); return }
    if (r.value.vault_unlocked) { onSignedIn(); return }
    // The password screen starts clean: an earlier try's words belong to that try (DW-A30).
    setProblem(null)
    setProblemMessage(null)
    setScreen('passkey-password')
  }, [email, onSignedIn])

  switch (screen) {
    case 'first-run':
      return <FirstRunView onCreate={create} onSignIn={() => setScreen('sign-in')} />
    case 'continue-as':
      return continueAs
        ? <ContinueAsView account={continueAs} state="asking" onTryAgain={() => {}} onAnotherWay={() => setScreen('sign-in')} />
        : null
    case 'another-device':
      return (
        <AnotherDeviceView
          status={device.status} code={device.code} secondsLeft={device.secondsLeft} deviceName={device.deviceName} qrSvg={device.qr}
          onCopyCode={() => { if (device.code) void navigator.clipboard?.writeText(device.code) }}
          onOpenDrive={() => { void deviceRequestCancel(); onSignedIn() }}
          onAskAgain={() => void startDevice()}
          onUseEmail={() => leaveDevice('sign-in')}
        />
      )
    case 'two-factor':
      return (
        <TwoFactorView
          mode={twoFactor.mode} code={twoFactor.code} state={twoFactor.state}
          onCode={(code) => setTwoFactor((t) => ({ ...t, code, state: t.state === 'wrong' ? 'idle' : t.state }))}
          onSubmit={(code) => { if (code.length === 6 || code.length === 8) void verify(code) }}
          onSwitch={() => setTwoFactor((t) => ({ mode: t.mode === 'totp' ? 'backup' : 'totp', code: '', state: 'idle' }))}
          onBackupCode={(code) => setTwoFactor({ mode: 'backup', code, state: 'idle' })}
          onBack={() => { setTwoFactor({ mode: 'totp', code: '', state: 'idle' }); setScreen('sign-in') }}
        />
      )
    case 'phrase':
      return (
        <PhraseView words={words} focus={0} unknown={unknown} state={phraseState}
          onWords={(w) => { setWords(w); if (phraseState === 'wrong') setPhraseState('idle') }}
          onRestore={() => void restore()} onApproveInstead={() => void startDevice()} onLost={() => setScreen('lost-phrase')} />
      )
    case 'lost-phrase':
      return (
        <LostPhraseView onFound={() => setScreen('phrase')}
          onCancelBilling={() => void openUrl('https://app.beebeeb.io/billing')}
          onErase={() => void openUrl('https://app.beebeeb.io/settings')}
          onDelete={() => void openUrl('https://app.beebeeb.io/settings')} />
      )
    case 'passkey-password':
      // A failed Unlock is said here, in the sign-in screen's words (the view takes the error; it was never passed).
      return <PasskeyPasswordView email={email} password={password} busy={busy} error={problemMessage ?? undefined} waitSeconds={wait} onPassword={setPassword} onUnlock={() => void submit()} />
    default:
      return (
        <SignInView
          continueAs={continueAs} email={email} password={password} busy={busy} problem={problem} waitSeconds={wait} passkeyPrompt={passkeyPrompt}
          onEmail={(v) => { setEmail(v); if (problem?.kind !== 'rate-limited') setProblem(null) }}
          onPassword={setPassword}
          onSubmit={() => void submit()}
          onContinueAs={() => setScreen('continue-as')}
          onAnotherDevice={() => void startDevice()}
          onForgot={forgot}
          onPasskey={() => void passkey()}
          onCreate={create}
        />
      )
  }
}
