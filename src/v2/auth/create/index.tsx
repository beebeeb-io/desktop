// The create-account controller (DW-A31 to A44) over B11's commands. Secrets typed here go to Rust and are cleared from
// state on submit and unmount (crypto review rule 9); nothing is ever printed.
import { useEffect, useRef, useState } from 'react'
import { command, openUrl } from '../../../desktopApi'
import * as api from './api'
import { emailErrorText, NEW_CODE_LINE, TICKET_EXPIRED, type CreateStep } from './model'
import { ConfirmStep, CreatingStep, DeviceStep, DoneStep, EmailStep, PasswordStep, PhraseStep, TermsStep, useCountdown, VerifyStep } from './views'
import { seed } from '../../seed'

const TERMS = 'https://beebeeb.io/terms'
const PRIVACY = 'https://beebeeb.io/privacy'

export interface CreateAccountFlowProps {
  /** Back on step 1: the welcome screen (DW-A01). */
  onBack: () => void
  /** DW-A44 Continue: the first run (DW-0201). */
  onCreated: () => void
  /** DW-A43's "created, finish failed": Sign in is the only way on. */
  onSignIn: () => void
}

export function CreateAccountFlow({ onBack, onCreated, onSignIn }: CreateAccountFlowProps) {
  const [step, setStep] = useState<CreateStep>(() => seed('create.step', 'email'))
  const [policy, setPolicy] = useState<api.SignupPolicyView | null>(null)
  const [email, setEmail] = useState('')
  // eslint-disable-next-line beebeeb/no-ad-hoc-error-surface -- the board ties DW-A31's error to the Email field
  const [emailError, setEmailError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [code, setCode] = useState(() => seed('create.code', ''))
  // eslint-disable-next-line beebeeb/no-ad-hoc-error-surface -- DW-A33: the error is tied to the code field and gates Verify
  const [codeState, setCodeState] = useState<'idle' | 'verifying' | 'wrong' | 'too-many'>(() => seed('create.codeState', 'idle'))
  const [notice, setNotice] = useState<{ tone: 'neutral' | 'red'; text: string } | null>(() => seed('create.notice', null))
  const [sentAt, setSentAt] = useState(0)
  const [terms, setTerms] = useState(() => seed('create.terms', { accepted: false, understood: false }))
  const [pw, setPw] = useState(() => seed('create.pw', { password: '', confirm: '', show: false }))
  const [meter, setMeter] = useState<api.PasswordMeter | null>(() => seed('create.meter', null))
  const [breach, setBreach] = useState<'breached' | 'blocked' | null>(() => seed('create.breach', null))
  const [words, setWords] = useState<string[]>(() => seed('create.words', []))
  const [saved, setSaved] = useState(false)
  const [copied, setCopied] = useState(false)
  const [positions, setPositions] = useState<number[]>(() => seed('create.positions', []))
  const [answers, setAnswers] = useState<string[]>(() => seed('create.answers', []))
  const [active, setActive] = useState(() => seed('create.active', 0))
  const [suggestions, setSuggestions] = useState<string[]>(() => seed('create.suggestions', []))
  const [wrongWords, setWrongWords] = useState(() => seed('create.wrongWords', false))
  const [device, setDevice] = useState('')
  // eslint-disable-next-line beebeeb/no-ad-hoc-error-surface -- DW-A43: the failure replaces the progress screen and gates the way on
  const [problem, setProblem] = useState<'failed' | 'finish-failed' | null>(() => seed('create.problem', null))
  /** Where a verified code leads: Terms, or straight back to creating after DW-A43's expired code. */
  const [afterVerify, setAfterVerify] = useState<CreateStep>('terms')
  const secondsLeft = useCountdown(policy?.resend_after_secs ?? 60, sentAt)
  const clipboardTimer = useRef<number | undefined>(undefined)

  useEffect(() => {
    void api.signupBegin().then((r) => { if (r.ok) setPolicy(r.value) })
    void api.defaultDeviceName().then((r) => { if (r.ok) setDevice(r.value) })
    return () => {
      // Leaving the flow forgets everything in Rust. The typed secrets go with this component's state: a set-state in an
      // unmount cleanup does nothing in a build, and under React's dev StrictMode re-mount it emptied the live screen.
      window.clearTimeout(clipboardTimer.current)
      void api.cancelSignup()
    }
  }, [])

  // The meter follows each keystroke (core's evaluator).
  useEffect(() => {
    if (step !== 'password') return
    void api.passwordMeter(pw.password).then((r) => { if (r.ok) setMeter(r.value) })
  }, [pw.password, step])

  const send = async (resend: boolean) => {
    setBusy(true)
    const r = await api.sendCode(email)
    setBusy(false)
    if (!r.ok) { setEmailError(emailErrorText(r.reason)); return }
    setEmailError(null)
    setCode('')
    setCodeState('idle')
    setNotice(resend ? { tone: 'neutral', text: NEW_CODE_LINE } : null)
    setSentAt(Date.now())
    setStep('verify')
  }

  const verify = async () => {
    setCodeState('verifying')
    const r = await api.verifyCode(code)
    if (r.ok) { setCodeState('idle'); setNotice(null); setStep(afterVerify); return }
    setCodeState(r.reason.startsWith('rate_limited') ? 'too-many' : 'wrong')
  }

  const passwordOk = meter != null && meter.missing === 0 && pw.password === pw.confirm && breach !== 'breached'

  const runBreach = async (): Promise<boolean> => {
    if (!pw.password || (meter && meter.missing > 0)) return false
    const r = await api.breachCheck(pw.password)
    const v = r.ok ? r.value : 'blocked'
    setBreach(v === 'clean' ? null : v)
    return v === 'clean'
  }

  const continuePassword = async () => {
    setBusy(true)
    const clean = await runBreach()
    const r = clean ? await api.setPassword(pw.password, pw.confirm) : null
    setBusy(false)
    if (!r?.ok) return
    setPw({ password: '', confirm: '', show: false })
    const w = await api.phraseWords()
    if (w.ok) { setWords(w.value); setStep('phrase') }
  }

  // B11 ruling: Rust puts the words on the clipboard and clears them after 60 s; the web view never runs the copy.
  const copyPhrase = async () => {
    const r = await api.copyPhrase()
    if (!r.ok) return
    setCopied(true)
    window.clearTimeout(clipboardTimer.current)
    clipboardTimer.current = window.setTimeout(() => setCopied(false), 60_000)
  }

  const toConfirm = async () => {
    const r = await api.phraseChallenge()
    if (!r.ok) return
    setPositions(r.value)
    setAnswers(r.value.map(() => ''))
    setWrongWords(false)
    setWords([]) // the words leave app state once the person has saved them
    setStep('confirm')
  }

  const answer = (i: number, v: string) => {
    setAnswers((a) => a.map((x, j) => (j === i ? v : x)))
    setWrongWords(false)
    void command<string[]>('auth_phrase_suggest', { prefix: v }).then((r) => setSuggestions(r.ok && !r.value.includes(v) ? r.value : []))
  }

  const confirm = async () => {
    setBusy(true)
    const r = await api.confirmPhrase(answers)
    setBusy(false)
    if (!r.ok) { setWrongWords(true); return }
    setAnswers([])
    setStep('device')
  }

  const showAgain = async () => {
    const w = await api.phraseWords()
    if (w.ok) { setWords(w.value); setSaved(false); setStep('phrase') }
  }

  const create = async () => {
    setProblem(null)
    setStep('creating')
    const r = await api.createAccount(device)
    if (r.ok) { setStep('done'); return }
    if (r.reason === 'ticket_expired') {
      setNotice({ tone: 'red', text: TICKET_EXPIRED })
      setCode('')
      setAfterVerify('device')
      setStep('verify')
      return
    }
    setProblem(r.reason === 'finish_failed' ? 'finish-failed' : 'failed')
  }

  switch (step) {
    case 'email':
      return <EmailStep email={email} busy={busy} error={emailError} onEmail={(v) => { setEmail(v); setEmailError(null) }} onSend={() => void send(false)} onBack={onBack} />
    case 'verify':
      return (
        <VerifyStep email={email.trim().toLowerCase()} code={code} length={policy?.code_length ?? 8} secondsLeft={secondsLeft} state={codeState} notice={notice}
          onCode={(v) => { setCode(v); if (codeState === 'wrong') setCodeState('idle') }} onVerify={() => void verify()}
          onResend={() => void send(true)} onDifferentEmail={() => setStep('email')} />
      )
    case 'terms':
      return (
        <TermsStep termsUrl={policy?.terms_url ?? TERMS} privacyUrl={policy?.privacy_url ?? PRIVACY} accepted={terms.accepted} understood={terms.understood}
          onAccepted={(v) => setTerms((t) => ({ ...t, accepted: v }))} onUnderstood={(v) => setTerms((t) => ({ ...t, understood: v }))}
          onContinue={() => setStep('password')} onOpen={(u) => void openUrl(u)} />
      )
    case 'password':
      return (
        <PasswordStep password={pw.password} confirm={pw.confirm} meter={meter} breach={breach === 'breached' ? 'breached' : null} blocked={breach === 'blocked'}
          show={pw.show} busy={busy} canContinue={passwordOk}
          onPassword={(v) => { setPw((p) => ({ ...p, password: v })); setBreach(null) }} onConfirm={(v) => setPw((p) => ({ ...p, confirm: v }))}
          onShow={() => setPw((p) => ({ ...p, show: !p.show }))} onBlur={() => void runBreach()} onContinue={() => void continuePassword()} />
      )
    case 'phrase':
      return (
        <PhraseStep words={words} saved={saved} copied={copied} onSaved={setSaved} onCopy={() => void copyPhrase()}
          onSave={() => void api.savePhrase()} onPrint={() => window.print()} onContinue={() => void toConfirm()} />
      )
    case 'confirm':
      return (
        <ConfirmStep positions={positions} answers={answers} suggestions={suggestions} active={active} wrong={wrongWords} busy={busy}
          onAnswer={answer} onActive={(i) => { setActive(i); setSuggestions([]) }} onShowAgain={() => void showAgain()} onConfirm={() => void confirm()} />
      )
    case 'device':
      return <DeviceStep name={device} onName={setDevice} onCreate={() => void create()} />
    case 'creating':
      return <CreatingStep problem={problem} onTryAgain={() => void create()} onSignIn={onSignIn} />
    case 'done':
      return <DoneStep onContinue={onCreated} />
  }
}
