// DW-A31 to A44: create a Beebeeb account, in B10's window (brand panel at the left). Pure views over props; the
// controller is ./index.tsx. Every secret field carries SECRET_FIELD (crypto review rule 8).
import { useEffect, useRef, useState, type ReactNode } from 'react'
import { Icon } from '@beebeeb/shared/design'
import { AuthWindow, Field, Heading, Note, PrimaryButton, SECRET_FIELD, StepLockup, TextLink, Tile } from '../parts'
import type { PasswordMeter } from './api'
import { usePrivateScreen } from '../privateScreen'
import { BREACHED, clock, hintText, needsMoreText, STRENGTH_LABEL, STRENGTH_SEGMENTS, WORDS_MISMATCH } from './model'

const noopSubmit = (fn: () => void, ok: boolean) => (e: React.FormEvent) => { e.preventDefault(); if (ok) fn() }

/** "Step 2 of 6 · Verify" with six bars. */
export function Progress({ step }: { step: number }) {
  const names = ['Email', 'Verify', 'Terms', 'Password', 'Phrase', 'This device']
  return (
    <div className="mb-4" data-progress={step}>
      <div className="mb-1.5 flex gap-1" aria-hidden>
        {names.map((n, i) => <span key={n} className={`h-1 flex-1 rounded-full ${i < step ? 'bg-ink-2' : 'bg-line'}`} />)}
      </div>
      <p className="m-0 text-[11px] text-ink-3"><span className="font-mono">{step} of 6</span> · {names[step - 1]}</p>
    </div>
  )
}

function Back({ onBack, label = 'Back' }: { onBack: () => void; label?: string }) {
  return <div className="mt-4 text-center"><TextLink onClick={onBack}>{label}</TextLink></div>
}

// ── Step 1: email ────────────────────────────────────────────────────────────

export function EmailStep({ email, busy, error, onEmail, onSend, onBack }: {
  email: string; busy: boolean; error: string | null; onEmail: (v: string) => void; onSend: () => void; onBack: () => void
}) {
  return (
    <AuthWindow>
      <StepLockup /><div className="mt-4" />
      <Progress step={1} />
      <Heading sub="One account for Drive, Pass, VPN, Chat and Mail. The same email works in every Beebeeb product.">Create your Beebeeb account</Heading>
      <form onSubmit={noopSubmit(onSend, !busy)} noValidate>
        {/* eslint-disable-next-line jsx-a11y/no-autofocus */}
        <Field label="Email" icon="mail" type="email" autoComplete="email" autoFocus value={email} error={error ?? undefined}
          aria-describedby="email-hint" disabled={busy} onChange={(e) => onEmail(e.target.value)} placeholder="you@example.eu" />
        {!error && <p id="email-hint" className="-mt-1 mb-3 text-[12px] text-ink-3">We send an 8-digit code here. It is also the name of your account.</p>}
        <PrimaryButton type="submit" disabled={busy}>{busy ? 'Sending…' : 'Send the code'}</PrimaryButton>
      </form>
      <Back onBack={onBack} />
    </AuthWindow>
  )
}

// ── Step 2: the code ─────────────────────────────────────────────────────────

export function VerifyStep({ email, code, length, secondsLeft, state, notice, onCode, onVerify, onResend, onDifferentEmail }: {
  email: string; code: string; length: number; secondsLeft: number
  state: 'idle' | 'verifying' | 'wrong' | 'too-many'
  /** DW-A34's calm line, or DW-A43's "code expired" note. */
  notice: { tone: 'neutral' | 'red'; text: string } | null
  onCode: (v: string) => void; onVerify: () => void; onResend: () => void; onDifferentEmail: () => void
}) {
  const full = code.length === length
  const error = state === 'wrong' ? 'That code is not right, or it has expired.' : state === 'too-many' ? 'Too many wrong codes. Ask for a new one.' : undefined
  const canResend = secondsLeft <= 0 || state === 'too-many'
  return (
    <AuthWindow>
      <StepLockup /><div className="mt-4" />
      <Progress step={2} />
      <Heading sub={<>We sent a code to <span className="font-medium text-ink">{email}</span>. It works for 15 minutes.</>}>Check your email</Heading>
      {notice && <Note tone={notice.tone}>{notice.text}</Note>}
      <form onSubmit={noopSubmit(onVerify, full && state !== 'verifying')} noValidate>
        <Field label="Code" mono {...SECRET_FIELD} autoComplete="one-time-code" inputMode="numeric" maxLength={length} value={code}
          placeholder={'0'.repeat(length)} error={error} disabled={state === 'verifying'}
          onChange={(e) => onCode(e.target.value.replace(/\D/g, '').slice(0, length))} />
        <PrimaryButton type="submit" disabled={!full || state === 'verifying' || state === 'too-many'}>{state === 'verifying' ? 'Verifying…' : 'Verify'}</PrimaryButton>
      </form>
      <p className="mt-3 text-[12px] text-ink-3" aria-live={canResend ? 'polite' : 'off'} data-resend>
        {canResend ? <TextLink onClick={onResend}>Send a new code</TextLink> : <>You can ask for a new code in <span className="font-mono">{clock(secondsLeft)}</span></>}
      </p>
      <Back onBack={onDifferentEmail} label="Use a different email" />
    </AuthWindow>
  )
}

// ── Step 3: terms ────────────────────────────────────────────────────────────

export function TermsStep({ termsUrl, privacyUrl, accepted, understood, onAccepted, onUnderstood, onContinue, onOpen }: {
  termsUrl: string; privacyUrl: string; accepted: boolean; understood: boolean
  onAccepted: (v: boolean) => void; onUnderstood: (v: boolean) => void; onContinue: () => void; onOpen: (url: string) => void
}) {
  const link = (label: string, url: string) => <a href={url} onClick={(e) => { e.preventDefault(); onOpen(url) }} className="text-ink underline underline-offset-2">{label}</a>
  const box = (id: string, checked: boolean, on: (v: boolean) => void, label: ReactNode) => (
    <label htmlFor={id} className="mb-3 flex cursor-pointer gap-2.5 text-[13px] leading-[18px] text-ink" data-check>
      <input id={id} type="checkbox" checked={checked} onChange={(e) => on(e.target.checked)} className="mt-0.5 h-4 w-4 flex-none accent-[var(--color-ink-2)]" />
      <span>{label}</span>
    </label>
  )
  return (
    <AuthWindow>
      <StepLockup /><div className="mt-4" />
      <Progress step={3} />
      <Heading sub="Two things to agree to before your keys are made.">Terms and the one thing that matters most</Heading>
      {box('terms', accepted, onAccepted, <>I agree to the {link('Terms of Service', termsUrl)} and the {link('Privacy Policy', privacyUrl)}.</>)}
      {box('honest', understood, onUnderstood, <>I understand that Beebeeb can’t recover my account. Without my password and my recovery phrase, my files can’t be opened by anyone, not even Beebeeb.</>)}
      <div className="mt-2"><PrimaryButton disabled={!accepted || !understood} onClick={onContinue}>Continue</PrimaryButton></div>
    </AuthWindow>
  )
}

// ── Step 4: password ─────────────────────────────────────────────────────────

export function Meter({ meter }: { meter: PasswordMeter | null }) {
  const lit = meter && meter.length > 0 ? STRENGTH_SEGMENTS[meter.strength] : 0
  const short = meter != null && meter.length > 0 && meter.missing > 0
  const tone = short ? 'bg-red' : lit >= 3 ? 'bg-green' : 'bg-ink-3'
  const hint = meter && !short ? hintText(meter.hint) : null
  return (
    <div className="-mt-1 mb-3" data-meter={lit}>
      <div className="mb-1.5 flex gap-1" aria-hidden>
        {[1, 2, 3, 4].map((i) => <span key={i} data-segment={i <= lit ? 'on' : 'off'} className={`h-1 flex-1 rounded-full ${i <= lit ? tone : 'bg-line'}`} />)}
      </div>
      <p className="m-0 text-[12px] leading-4" aria-live="polite">
        {meter && meter.length > 0 && <span className={`font-medium ${short ? 'text-red' : 'text-ink-2'}`}>{STRENGTH_LABEL[meter.strength]}. </span>}
        {short && meter && <span className="text-red">{needsMoreText(meter.min_length, meter.missing)}</span>}
        {(!meter || meter.length === 0) && <span className="text-ink-3">At least {meter?.min_length ?? 12} characters.</span>}
      </p>
      {(hint || short) && <p className="mt-0.5 text-[12px] leading-4 text-ink-3">{hint ?? 'Mix upper and lowercase, and add a number or symbol.'}</p>}
    </div>
  )
}

export function PasswordFields({ password, confirm, meter, breach, show, busy, onPassword, onConfirm, onShow, onBlur }: {
  password: string; confirm: string; meter: PasswordMeter | null; breach: 'breached' | 'blocked' | null; show: boolean; busy: boolean
  onPassword: (v: string) => void; onConfirm: (v: string) => void; onShow: () => void; onBlur: () => void
}) {
  const confirmRef = useRef<HTMLInputElement>(null)
  const mismatch = confirm.length >= password.length && confirm.length > 0 && confirm !== password
  const type = show ? 'text' : 'password'
  return (
    <>
      <div className="relative">
        {/* eslint-disable-next-line jsx-a11y/no-autofocus */}
        <Field label="Password" icon="lock" secret type={type} autoFocus value={password} disabled={busy}
          error={breach === 'breached' ? BREACHED : undefined} onBlur={onBlur}
          onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); confirmRef.current?.focus() } }}
          onChange={(e) => onPassword(e.target.value)} />
        <button type="button" onClick={onShow} aria-label={show ? 'Hide password' : 'Show password'} className="absolute right-2 top-[26px] grid h-6 w-6 place-items-center rounded text-ink-3 hover:bg-paper-2">
          <Icon name={show ? 'eye-off' : 'eye'} size={14} />
        </button>
      </div>
      <Meter meter={meter} />
      <Field label="Confirm password" icon="lock" secret type={type} ref={confirmRef} value={confirm} disabled={busy}
        error={mismatch ? 'Does not match yet.' : undefined} onChange={(e) => onConfirm(e.target.value)} />
    </>
  )
}

/** A step that failed, under its button (M3b's leftovers; the copy is the lead's, create/model.ts STEP_FAILED). */
function StepError({ text }: { text?: string | null }) {
  return text ? <p role="alert" data-step-error className="m-0 mt-2 text-[12px] leading-4 text-red-text">{text}</p> : null
}

export function PasswordStep(props: Parameters<typeof PasswordFields>[0] & { canContinue: boolean; blocked: boolean; onContinue: () => void; error?: string | null }) {
  const { canContinue, blocked, onContinue, error, ...fields } = props
  return (
    <AuthWindow>
      <StepLockup /><div className="mt-4" />
      <Progress step={4} />
      <Heading sub="It unlocks your files day to day on this PC. Your recovery phrase, next, is the real backup.">Choose a password</Heading>
      {blocked && <Note tone="red">{'We could not check this password against known breaches, and we do not let you continue without that check. Try again.'}</Note>}
      <form onSubmit={noopSubmit(onContinue, canContinue && !fields.busy)} noValidate>
        <PasswordFields {...fields} />
        <p className="mb-3 text-[12px] leading-4 text-ink-3">We check it against known data breaches without sending it: only the first 5 characters of its hash leave this PC.</p>
        <PrimaryButton type="submit" disabled={!canContinue || fields.busy}>{fields.busy ? 'Checking…' : 'Continue'}</PrimaryButton>
        <StepError text={error} />
      </form>
    </AuthWindow>
  )
}

// ── Step 5: the phrase ───────────────────────────────────────────────────────

export function PhraseStep({ words, saved, copied, onSaved, onCopy, onSave, onPrint, onContinue, error }: {
  words: string[]; saved: boolean; copied: boolean
  onSaved: (v: boolean) => void; onCopy: () => void; onSave: () => void; onPrint: () => void; onContinue: () => void
  error?: string | null
}) {
  usePrivateScreen()
  return (
    <AuthWindow>
      <StepLockup /><div className="mt-4" />
      <Progress step={5} />
      <Heading sub="This phrase protects Drive, Pass, VPN, Chat and Mail. The same 12 words.">Your recovery phrase</Heading>
      <Note tone="red" title="Shown once, here">It can’t be shown again or reset. Write it down and keep it offline.</Note>
      <ol className="m-0 mb-3 grid list-none grid-cols-3 gap-1.5 p-0" data-phrase-words aria-label="Recovery phrase, 12 words">
        {words.map((w, i) => (
          <li key={i} data-word className="flex h-8 items-center gap-1.5 rounded-md border border-line bg-paper-2 px-2 font-mono text-[12px] text-ink">
            <span className="w-4 flex-none text-right text-[11px] text-ink-4">{i + 1}</span>{w}
          </li>
        ))}
      </ol>
      <div className="mb-3 flex gap-2 text-[12px]">
        {[['copy', copied ? 'Copied' : 'Copy', onCopy], ['download', 'Save as file', onSave], ['file-text', 'Print', onPrint]].map(([icon, label, on]) => (
          <button key={label as string} type="button" onClick={on as () => void} className="flex h-7 items-center gap-1.5 rounded-md border border-line px-2.5 text-ink hover:bg-paper-2">
            <Icon name={icon as 'copy'} size={13} /><span className="text-[12px]">{label as string}</span>
          </button>
        ))}
      </div>
      <label htmlFor="saved" className="mb-3 flex cursor-pointer gap-2.5 text-[13px] text-ink">
        <input id="saved" type="checkbox" checked={saved} onChange={(e) => onSaved(e.target.checked)} className="mt-0.5 h-4 w-4 accent-[var(--color-ink-2)]" />
        I have saved my recovery phrase offline.
      </label>
      <PrimaryButton disabled={!saved} onClick={onContinue}>I saved it, verify</PrimaryButton>
      <StepError text={error} />
    </AuthWindow>
  )
}

/** DW-A40: suggestions from the 2048-word list complete what was typed. The list comes from the Rust side's checker. */
export function ConfirmStep({ positions, answers, suggestions, active, wrong, busy, onAnswer, onActive, onShowAgain, onConfirm }: {
  positions: number[]; answers: string[]; suggestions: string[]; active: number; wrong: boolean; busy: boolean
  onAnswer: (i: number, v: string) => void; onActive: (i: number) => void; onShowAgain: () => void; onConfirm: () => void
}) {
  // PhraseStep unmounts here, and its cleanup turns capture exclusion off while these words are typed.
  usePrivateScreen()
  const full = answers.every((a) => a.trim().length > 0)
  return (
    <AuthWindow>
      <StepLockup /><div className="mt-4" />
      <Progress step={5} />
      <Heading sub="Type the words you wrote down.">Confirm your recovery phrase</Heading>
      <form onSubmit={noopSubmit(onConfirm, full && !busy)} noValidate>
        {positions.map((p, i) => (
          <div key={p}>
            <Field label={`Word ${p}`} mono {...SECRET_FIELD} id={`word-${p}`} aria-label={`Recovery word ${p}`} value={answers[i] ?? ''} disabled={busy}
              error={wrong && i === 1 ? `${WORDS_MISMATCH} Check them against what you wrote down.` : undefined}
              onFocus={() => onActive(i)} onChange={(e) => onAnswer(i, e.target.value.toLowerCase().replace(/[^a-z]/g, ''))} />
            {active === i && suggestions.length > 0 && (
              <ul className="-mt-1 mb-2.5 flex list-none flex-wrap gap-1 p-0" aria-label="Suggestions" data-suggestions>
                {suggestions.map((s) => (
                  <li key={s}><button type="button" onClick={() => onAnswer(i, s)} className="rounded border border-line bg-paper-2 px-2 py-0.5 text-ink hover:bg-paper-3"><span className="font-mono text-[11px]">{s}</span></button></li>
                ))}
              </ul>
            )}
          </div>
        ))}
        <PrimaryButton type="submit" disabled={!full || busy}>{busy ? 'Checking…' : 'Confirm'}</PrimaryButton>
      </form>
      <Back onBack={onShowAgain} label="Show the words again" />
    </AuthWindow>
  )
}

// ── Step 6: this device, then creating, then done ────────────────────────────

export function DeviceStep({ name, onName, onCreate }: { name: string; onName: (v: string) => void; onCreate: () => void }) {
  return (
    <AuthWindow>
      <StepLockup /><div className="mt-4" />
      <Progress step={6} />
      <Heading sub="A name people can recognise.">Name this device</Heading>
      <form onSubmit={noopSubmit(onCreate, true)} noValidate>
        <Field label="Device name" icon="file" maxLength={40} value={name} onChange={(e) => onName(e.target.value)} aria-describedby="device-hint" />
        <p id="device-hint" className="-mt-1 mb-3 text-[12px] text-ink-3">It appears in Devices and sessions, and on sign-in approvals.</p>
        <PrimaryButton type="submit">Create my account</PrimaryButton>
      </form>
    </AuthWindow>
  )
}

export function CreatingStep({ problem, onTryAgain, onSignIn }: {
  problem: 'failed' | 'finish-failed' | null; onTryAgain: () => void; onSignIn: () => void
}) {
  if (problem === 'failed') {
    return (
      <AuthWindow>
        <Tile icon="info" tone="red" />
        <Heading sub="We could not create your account. Nothing was stored. You can try again.">Your account wasn’t created</Heading>
        <PrimaryButton onClick={onTryAgain}>Try again</PrimaryButton>
      </AuthWindow>
    )
  }
  if (problem === 'finish-failed') {
    return (
      <AuthWindow>
        <Tile icon="info" tone="neutral" />
        <Heading sub="Your account was created. We could not finish setting it up on this device. Nothing is lost.">Almost there</Heading>
        <PrimaryButton onClick={onSignIn}>Sign in</PrimaryButton>
      </AuthWindow>
    )
  }
  return (
    <AuthWindow>
      <div role="status" aria-live="polite" data-creating>
        <span className="mb-4 block h-12 w-12 animate-spin rounded-full border-[3px] border-line border-t-ink-2" aria-hidden />
        <Heading sub="Making your keys, your vault and the first session on this PC. This takes a few seconds.">Setting up account encryption</Heading>
      </div>
    </AuthWindow>
  )
}

export function DoneStep({ onContinue }: { onContinue: () => void }) {
  return (
    <AuthWindow>
      <Tile icon="check" tone="green" />
      <Heading sub="Your email works in Drive, Pass, VPN, Chat and Mail.">Your account is ready</Heading>
      <div className="mb-3 rounded-md border border-line px-3 py-2.5" data-next>
        <div className="text-[11px] uppercase tracking-[0.06em] text-ink-3">Next</div>
        <div className="mt-0.5 text-[13px] font-medium text-ink">Set up Drive on this PC</div>
      </div>
      <Note tone="amber">Your recovery phrase was shown once at sign-up. Account says so; it can’t show it again.</Note>
      <PrimaryButton onClick={onContinue}>Continue</PrimaryButton>
    </AuthWindow>
  )
}

/** A countdown that ticks once a second from `from`. */
export function useCountdown(from: number, key: unknown): number {
  const [left, setLeft] = useState(from)
  useEffect(() => {
    setLeft(from)
    const t = window.setInterval(() => setLeft((s) => (s > 0 ? s - 1 : 0)), 1000)
    return () => window.clearInterval(t)
  }, [from, key])
  return left
}
