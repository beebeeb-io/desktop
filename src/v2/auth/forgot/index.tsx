// DW-A45 to A50: forgot password. Beebeeb cannot reset a password (that would orphan the encrypted data): the 12 words
// re-derive the same master key in Rust, and only the password wrapper changes. Copy never says "reset".
import { useEffect, useState } from 'react'
import { Icon } from '@beebeeb/shared/design'
import { normalizeWord, phraseFilled, spreadPaste } from '../model'
import { AuthWindow, Field, Heading, Note, PrimaryButton, SECRET_FIELD, SecondaryButton, StepLockup, TextLink, Tile } from '../parts'
import { unknownPhraseWords } from '../api'
import * as api from '../create/api'
import { recoverErrorText } from '../create/model'
import { PasswordFields } from '../create/views'
import { usePrivateScreen } from '../privateScreen'
import { seed } from '../../seed'

// ── Views ────────────────────────────────────────────────────────────────────

/** DW-A45, as the board draws it: three option cards, each with its own button; the phrase is the one amber action. */
export function OptionsView({ onPhrase, onDevice, onNeither, onBack }: { onPhrase: () => void; onDevice: () => void; onNeither: () => void; onBack: () => void }) {
  const card = (icon: 'key' | 'scan' | 'x', title: string, detail: string, cta: string, on: () => void, primary = false) => (
    <li data-option className="flex flex-col gap-2 rounded-lg border border-line bg-paper px-3.5 py-3">
      <div className="flex items-center gap-2 text-[13px] font-semibold text-ink"><Icon name={icon} size={16} className="flex-none text-ink-2" />{title}</div>
      <p className="m-0 text-[12px] leading-4 text-ink-3">{detail}</p>
      {primary ? <PrimaryButton onClick={on}>{cta}</PrimaryButton> : <SecondaryButton onClick={on}>{cta}</SecondaryButton>}
    </li>
  )
  return (
    <AuthWindow>
      <StepLockup /><div className="mt-4" />
      <Heading sub="Choose the option that applies to you.">Trouble signing in?</Heading>
      <ul className="m-0 mb-4 flex list-none flex-col gap-2 p-0">
        {card('key', 'I have my recovery phrase', 'The 12 words from account setup. Use them to set a new password.', 'Recover with phrase', onPhrase, true)}
        {card('scan', 'I have another device signed in', 'Approve this PC from a device where you’re already signed in. No phrase needed.', 'Sign in with another device', onDevice)}
        {card('x', 'I have neither', 'We’ll explain what that means for your data.', 'Show my options', onNeither)}
      </ul>
      <div className="text-center"><TextLink onClick={onBack}>Back to sign in</TextLink></div>
    </AuthWindow>
  )
}

export function NeitherView({ onBack }: { onBack: () => void }) {
  return (
    <AuthWindow>
      <Tile icon="lock" tone="neutral" />
      <Heading sub="Without your recovery phrase or a signed-in device, nobody can open your files: not you, not us.">Your files can’t be recovered</Heading>
      <p className="mb-3 text-[12px] leading-[17px] text-ink-2">Why: Beebeeb is zero-knowledge. Your files are encrypted on your devices with keys we never see, so there is nothing on our side to recover them with.</p>
      <p className="mb-3 text-[12px] leading-[17px] text-ink-2">What we can still do: close the account and answer billing questions. Write to <span className="font-mono">support@beebeeb.io</span>.</p>
      <Note tone="red">Never send anyone your recovery phrase, not even us.</Note>
      <SecondaryButton onClick={onBack}>Back to recovery options</SecondaryButton>
    </AuthWindow>
  )
}

export function RecoverPhraseView({ email, words, focus, unknown, state, onEmail, onWords, onVerify, onLost }: {
  email: string; words: string[]; focus?: number; unknown: number[]
  state: 'idle' | 'verifying' | 'wrong' | 'too-many' | 'offline'
  onEmail: (v: string) => void; onWords: (w: string[]) => void; onVerify: () => void; onLost: () => void
}) {
  // This screen replaces the options, so nothing above it is still holding capture exclusion.
  usePrivateScreen()
  const filled = phraseFilled(words) && email.includes('@')
  return (
    <AuthWindow>
      <StepLockup /><div className="mt-4" />
      <Heading sub="Prove the phrase, then set a new password. Your files are not touched. The phrase never leaves this PC.">Recover with your phrase</Heading>
      {state === 'wrong' && <Note tone="red" title="That phrase did not verify">Nothing was changed: your password is as it was.</Note>}
      {(state === 'too-many' || state === 'offline') && <Note tone="red">{recoverErrorText(state === 'too-many' ? 'rate_limited' : 'offline')}</Note>}
      <Field label="Email" icon="mail" type="email" autoComplete="email" value={email} disabled={state === 'verifying'} onChange={(e) => onEmail(e.target.value)} />
      <div className="mb-1 text-[12px] font-medium text-ink-2">Recovery phrase</div>
      <div className="grid grid-cols-3 gap-1.5" data-phrase-grid>
        {words.map((w, i) => (
          <label key={i} data-word className={`flex h-8 items-center gap-1.5 rounded-md border bg-paper px-2 font-mono text-[12px] text-ink ${unknown.includes(i) ? 'border-red' : 'border-line'} focus-within:ring-2 focus-within:ring-amber-deep`}>
            <span className="w-4 flex-none text-right text-[11px] text-ink-4">{i + 1}</span>
            {/* eslint-disable-next-line jsx-a11y/no-autofocus */}
            <input {...SECRET_FIELD} aria-label={`Word ${i + 1}`} aria-invalid={unknown.includes(i) || undefined} autoFocus={focus === i} value={w}
              disabled={state === 'verifying'} className="min-w-0 flex-1 border-0 bg-transparent outline-none"
              onChange={(e) => { const next = [...words]; next[i] = normalizeWord(e.target.value); onWords(next) }}
              onPaste={(e) => { e.preventDefault(); onWords(spreadPaste(words, i, e.clipboardData.getData('text'))) }} />
          </label>
        ))}
      </div>
      {unknown.length > 0 && <p className="mt-1.5 text-[12px] text-red" role="alert">{unknown.length === 1 ? 'The marked word is not a recovery word.' : 'The marked words are not recovery words.'}</p>}
      <p className="mt-2 text-[12px] text-ink-3">Paste all 12 words into any box to fill them.</p>
      <div className="mt-3 flex flex-col gap-2.5">
        <PrimaryButton onClick={onVerify} disabled={!filled || state === 'verifying' || state === 'too-many'}>{state === 'verifying' ? 'Verifying…' : 'Verify phrase'}</PrimaryButton>
        {state === 'wrong' && <div className="text-center"><TextLink onClick={onLost}>I don’t have it</TextLink></div>}
      </div>
    </AuthWindow>
  )
}

export function NewPasswordView(props: Parameters<typeof PasswordFields>[0] & { canSet: boolean; problem: string | null; onSet: () => void }) {
  const { canSet, problem, onSet, ...fields } = props
  return (
    <AuthWindow>
      <StepLockup /><div className="mt-4" />
      <Heading sub="Your phrase checked out. Choose a new password: it wraps the same keys, so your files stay as they are. Other devices are signed out.">Set a new password</Heading>
      {problem && <Note tone="red">{problem}</Note>}
      <form onSubmit={(e) => { e.preventDefault(); if (canSet && !fields.busy) onSet() }} noValidate>
        <PasswordFields {...fields} />
        <PrimaryButton type="submit" disabled={!canSet || fields.busy}>{fields.busy ? 'Setting…' : 'Set new password'}</PrimaryButton>
      </form>
    </AuthWindow>
  )
}

export function PasswordUpdatedView({ signInNext, onContinue }: { signInNext: boolean; onContinue: () => void }) {
  return (
    <AuthWindow>
      <Tile icon="check" tone="green" />
      <Heading sub="We emailed you a security notice that your password was changed.">Password updated</Heading>
      <ul className="m-0 mb-4 flex list-none flex-col gap-1.5 p-0 text-[13px] text-ink" data-changes>
        <li className="flex gap-2"><Icon name="check" size={14} className="mt-0.5 text-green" />Files unchanged: the same keys, the same phrase.</li>
        <li className="flex gap-2"><Icon name="check" size={14} className="mt-0.5 text-green" />Other devices signed out.</li>
      </ul>
      <PrimaryButton onClick={onContinue}>{signInNext ? 'Sign in with the new password' : 'Continue to Drive'}</PrimaryButton>
    </AuthWindow>
  )
}

// ── Controller ───────────────────────────────────────────────────────────────

type Screen = 'options' | 'neither' | 'phrase' | 'password' | 'updated'

export function ForgotPasswordFlow({ initialEmail = '', onBack, onAnotherDevice, onRecovered, onSignIn }: {
  initialEmail?: string; onBack: () => void; onAnotherDevice: () => void; onRecovered: () => void; onSignIn: () => void
}) {
  const [screen, setScreen] = useState<Screen>(() => seed('forgot.screen', 'options'))
  const [email, setEmail] = useState(() => seed('forgot.email', initialEmail))
  const [words, setWords] = useState<string[]>(() => seed('forgot.words', Array(12).fill('')))
  const [unknown, setUnknown] = useState<number[]>(() => seed('forgot.unknown', []))
  // eslint-disable-next-line beebeeb/no-ad-hoc-error-surface -- DW-A48: the banner and the marked word persist until the words change
  const [state, setState] = useState<'idle' | 'verifying' | 'wrong' | 'too-many' | 'offline'>(() => seed('forgot.state', 'idle'))
  const [pw, setPw] = useState(() => seed('forgot.pw', { password: '', confirm: '', show: false }))
  const [meter, setMeter] = useState<api.PasswordMeter | null>(() => seed('forgot.meter', null))
  const [breach, setBreach] = useState<'breached' | 'blocked' | null>(() => seed('forgot.breach', null))
  const [busy, setBusy] = useState(false)
  // eslint-disable-next-line beebeeb/no-ad-hoc-error-surface -- DW-A49: the failure sits above the fields it gates
  const [problem, setProblem] = useState<string | null>(null)
  const [signInNext, setSignInNext] = useState(false)

  // Leaving the flow forgets the recovery in Rust. The typed words and passwords go with this component's state: a
  // set-state in an unmount cleanup does nothing in a build, and under React's dev StrictMode re-mount it emptied the
  // live screen (the word boxes vanished, `[].every` then let Verify phrase through with no words).
  useEffect(() => () => { void api.cancelSignup() }, [])
  useEffect(() => {
    if (screen !== 'password') return
    return api.followMeter(pw.password, setMeter)
  }, [pw.password, screen])

  const verify = async () => {
    setState('verifying')
    const r = await api.recoverWithPhrase(email, words)
    if (r.ok) { setWords(Array(12).fill('')); setState('idle'); setScreen('password'); return }
    const bad = await unknownPhraseWords(words)
    setUnknown(bad.ok ? bad.value : [])
    setState(r.reason.startsWith('rate_limited') ? 'too-many' : r.reason === 'offline' ? 'offline' : 'wrong')
  }

  const set = async () => {
    setBusy(true)
    setProblem(null)
    const b = await api.breachCheck(pw.password)
    const verdict = b.ok ? b.value : 'blocked'
    if (verdict !== 'clean') { setBreach(verdict); setBusy(false); return }
    const r = await api.recoverSetPassword(pw.password, pw.confirm)
    setBusy(false)
    if (!r.ok) {
      setProblem(r.reason === 'recovery_expired' ? 'This recovery took too long. Verify your phrase again.' : 'We could not set the new password. Nothing was changed. Try again.')
      if (r.reason === 'recovery_expired') setScreen('phrase')
      return
    }
    setPw({ password: '', confirm: '', show: false })
    setSignInNext(r.value === 'sign_in_required')
    setScreen('updated')
  }

  const canSet = meter != null && meter.missing === 0 && pw.password === pw.confirm && breach !== 'breached'
  switch (screen) {
    case 'options':
      return <OptionsView onPhrase={() => setScreen('phrase')} onDevice={onAnotherDevice} onNeither={() => setScreen('neither')} onBack={onBack} />
    case 'neither':
      return <NeitherView onBack={() => setScreen('options')} />
    case 'phrase':
      return (
        <RecoverPhraseView email={email} words={words} unknown={unknown} state={state} onEmail={setEmail}
          onWords={(w) => { setWords(w); setUnknown([]); if (state === 'wrong') setState('idle') }} onVerify={() => void verify()} onLost={() => setScreen('neither')} />
      )
    case 'password':
      return (
        <NewPasswordView password={pw.password} confirm={pw.confirm} meter={meter} breach={breach === 'breached' ? 'breached' : null} show={pw.show} busy={busy}
          canSet={canSet} problem={breach === 'blocked' ? 'We could not check this password against known breaches, and we do not let you continue without that check. Try again.' : problem}
          onPassword={(v) => { setPw((p) => ({ ...p, password: v })); setBreach(null) }} onConfirm={(v) => setPw((p) => ({ ...p, confirm: v }))}
          onShow={() => setPw((p) => ({ ...p, show: !p.show }))} onBlur={() => {}} onSet={() => void set()} />
      )
    case 'updated':
      return <PasswordUpdatedView signInNext={signInNext} onContinue={signInNext ? onSignIn : onRecovered} />
  }
}
