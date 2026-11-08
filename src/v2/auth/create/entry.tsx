// B11's account entry: B10's sign-in with B11's create-account (DW-A31) and forgot-password (DW-A45) wired in.
import { useState } from 'react'
import { SignInFlow } from '../index'
import { ForgotPasswordFlow } from '../forgot'
import { CreateAccountFlow } from './index'

type Mode = { kind: 'sign-in'; startAt: 'first-run' | 'sign-in' | 'another-device' } | { kind: 'create' } | { kind: 'forgot' }

export function AccountEntry({ onSignedIn, onCreated, startAt = 'first-run' }: {
  /** A sign-in finished (keys on this PC). */
  onSignedIn: () => void
  /** Sign-up finished: DW-A44 Continue (the first run, DW-0201). Recovery's Continue to Drive comes here too. */
  onCreated: () => void
  startAt?: 'first-run' | 'sign-in'
}) {
  const [mode, setMode] = useState<Mode>({ kind: 'sign-in', startAt })
  const signIn = (at: 'first-run' | 'sign-in' | 'another-device' = 'sign-in') => setMode({ kind: 'sign-in', startAt: at })
  if (mode.kind === 'create') return <CreateAccountFlow onBack={() => signIn('first-run')} onCreated={onCreated} onSignIn={() => signIn()} />
  if (mode.kind === 'forgot') {
    return <ForgotPasswordFlow onBack={() => signIn()} onAnotherDevice={() => signIn('another-device')} onRecovered={onCreated} onSignIn={() => signIn()} />
  }
  return (
    <SignInFlow key={mode.startAt} startAt={mode.startAt} onSignedIn={onSignedIn}
      onCreateAccount={() => setMode({ kind: 'create' })} onForgotPassword={() => setMode({ kind: 'forgot' })} />
  )
}
