// DW-A02, A03 (the sign-in screen with and without Continue as), A18 to A22 and A63 (its working and error states),
// A29 (a passkey, Windows draws the prompt) and A30 (the passkey proved who you are; the password unlocks).
import { type FormEvent, useEffect, useRef } from 'react'
import { Icon } from '@beebeeb/shared/design'
import { canSubmit, LOCK_NOTE, problemText, type SignInProblem } from './model'
import {
  AuthWindow, Avatar, Divider, Field, Heading, HelloGlyph, HelloPrompt, Note, PrimaryButton, SecondaryButton, StepLockup, TextLink,
} from './parts'

/** The account another Beebeeb app on this PC holds (read from that session, never typed). */
export interface ContinueAsAccount {
  name: string
  email: string
  /** The app that holds the session ("Drive"). */
  app: string
}

export interface SignInViewProps {
  continueAs: ContinueAsAccount | null
  email: string
  password: string
  busy: boolean
  problem: SignInProblem | null
  /** Seconds left on a 429 wait; the button counts down and re-enables itself (DW-A21). */
  waitSeconds?: number
  passkeyPrompt?: boolean
  onEmail: (v: string) => void
  onPassword: (v: string) => void
  onSubmit: () => void
  onContinueAs: () => void
  onAnotherDevice: () => void
  onForgot: () => void
  onPasskey: () => void
  onCreate: () => void
}

export function SignInView(p: SignInViewProps) {
  const offline = p.problem?.kind === 'offline'
  const fieldError = p.problem && p.problem.kind !== 'offline' ? problemText(p.problem) : undefined
  const waiting = (p.waitSeconds ?? 0) > 0
  const ready = canSubmit(p.email, p.password)
  const passwordRef = useRef<HTMLInputElement>(null)
  useEffect(() => { if (fieldError) passwordRef.current?.focus() }, [fieldError])
  const submit = (e: FormEvent) => { e.preventDefault(); if (ready && !p.busy && !waiting) p.onSubmit() }
  const anotherDevice = (
    <SecondaryButton onClick={p.onAnotherDevice} disabled={p.busy}>
      <Icon name="key" size={15} />Sign in with another device
    </SecondaryButton>
  )
  return (
    <AuthWindow
      overlay={p.passkeyPrompt ? (
        <HelloPrompt reason="Sign in to beebeeb.io" detail={`Use your passkey for ${p.email || 'your account'}`} />
      ) : undefined}
    >
      <StepLockup />
      <div className="mt-4" />
      <Heading sub="One Beebeeb account for Drive, Pass, VPN, Chat and Mail.">Sign in to Drive</Heading>
      {offline && <Note tone="red" title="You’re offline">Nothing was sent. Sign in again when you’re connected.</Note>}
      {p.continueAs && (
        <section aria-label={`Continue as ${p.continueAs.name}, ${p.continueAs.email}`} data-continue-as className="mb-3 rounded-lg border border-line bg-paper-2 p-3.5">
          <div className="mb-3 flex items-center gap-3">
            <Avatar name={p.continueAs.name} />
            <div className="min-w-0">
              <div className="text-[13px] font-semibold text-ink">Continue as {p.continueAs.name}</div>
              <div className="truncate font-mono text-[11px] text-ink-3">{p.continueAs.email}</div>
              <div className="text-[11px] text-ink-3">Signed in to {p.continueAs.app} on this PC</div>
            </div>
          </div>
          <PrimaryButton onClick={p.onContinueAs} disabled={p.busy || offline}>
            <HelloGlyph size={15} />Continue with Windows Hello
          </PrimaryButton>
        </section>
      )}
      {p.continueAs && <>{anotherDevice}<Divider>or use email and password</Divider></>}
      <form onSubmit={submit} noValidate>
        <Field label="Email" icon="mail" type="email" autoComplete="off" spellCheck={false} value={p.email} disabled={p.busy}
          onChange={(e) => p.onEmail(e.target.value)} />
        <Field label="Password" icon="lock" secret ref={passwordRef} value={p.password} disabled={p.busy} error={fieldError}
          onChange={(e) => p.onPassword(e.target.value)} />
        {p.problem?.kind === 'locked-hint' && <p data-lock-note className="-mt-1 mb-3 text-[12px] leading-4 text-ink-2">{LOCK_NOTE}</p>}
        <div className="mb-4 flex items-center justify-between">
          <TextLink onClick={p.onForgot}>Forgot password?</TextLink>
          <span className="inline-flex items-center gap-1.5 text-ink-2"><Icon name="key" size={13} /><TextLink onClick={p.onPasskey}>Use a passkey</TextLink></span>
        </div>
        <PrimaryButton type="submit" amber={!p.continueAs} disabled={p.busy || waiting || (!ready && !offline)}>
          {p.busy ? 'Unlocking vault…' : waiting ? `Try again in ${p.waitSeconds} s` : 'Sign in'}
        </PrimaryButton>
      </form>
      {!p.continueAs && <div className="mt-2.5">{anotherDevice}</div>}
      <p className="mt-4 text-center text-[12px] text-ink-3">New to Beebeeb? <TextLink onClick={p.onCreate}>Create an account</TextLink></p>
    </AuthWindow>
  )
}

/** DW-A30: the passkey proved who you are; this passkey cannot unlock the vault, so the password does. */
export function PasskeyPasswordView({ email, password, busy, error, onPassword, onUnlock }: {
  email: string; password: string; busy: boolean; error?: string; onPassword: (v: string) => void; onUnlock: () => void
}) {
  return (
    <AuthWindow>
      <StepLockup />
      <div className="mt-4" />
      <Heading sub={<span className="font-mono text-[12px]">{email}</span>}>Enter your password</Heading>
      <Note tone="green" title="Identity confirmed with your passkey">Passkey verified. Enter your password to decrypt your files on this device.</Note>
      <form onSubmit={(e) => { e.preventDefault(); if (password && !busy) onUnlock() }} noValidate>
        {/* Focus lands on the password: the one thing left to do on this screen (DW-A30). */}
        {/* eslint-disable-next-line jsx-a11y/no-autofocus */}
        <Field label="Password" icon="lock" secret autoFocus value={password} disabled={busy} error={error} onChange={(e) => onPassword(e.target.value)} />
        <PrimaryButton type="submit" disabled={!password || busy}>{busy ? 'Unlocking vault…' : 'Unlock'}</PrimaryButton>
      </form>
    </AuthWindow>
  )
}

/** DW-A04 to A06: Continue as, with Windows Hello. */
export function ContinueAsView({ account, state, onTryAgain, onAnotherWay }: {
  account: ContinueAsAccount
  state: 'asking' | 'failed' | 'cancelled'
  onTryAgain: () => void
  onAnotherWay: () => void
}) {
  return (
    <AuthWindow
      overlay={state === 'cancelled' ? undefined : (
        <HelloPrompt reason={`Continue to Drive as ${account.name}`} state={state === 'failed' ? 'failed' : 'looking'} />
      )}
    >
      {state === 'failed' && <Note tone="red" title="Windows Hello didn’t recognise you">Nothing was signed in.</Note>}
      {state === 'cancelled' && <Note tone="neutral">You cancelled Windows Hello. Nothing was signed in.</Note>}
      <div className="mb-4 grid h-20 w-20 place-items-center rounded-full border-4 border-line text-ink-2" data-hello-ring>
        <HelloGlyph size={40} />
      </div>
      <Heading sub={<span className="font-mono text-[12px]">{account.email}</span>}>Continue as {account.name}</Heading>
      <div className="flex flex-col gap-2.5">
        {state !== 'asking' && (
          <PrimaryButton onClick={onTryAgain}><HelloGlyph size={15} />{state === 'cancelled' ? 'Try Windows Hello again' : 'Try again'}</PrimaryButton>
        )}
        <SecondaryButton onClick={onAnotherWay}>Use another way to sign in</SecondaryButton>
      </div>
    </AuthWindow>
  )
}
