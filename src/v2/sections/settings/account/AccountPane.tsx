// Account (DW-A51) with change name (A64), change email (A65) and the code to the new address (A66).
import { useCallback, useEffect, useState } from 'react'
import { Group, Page, Row } from '../shell/kit'
import { useAccount, run, errorText, type Scenario } from './data'
import { Btn, BackLink, CodeBoxes, Field, PasswordInput, TextInput, useEscBack } from './ui'

export type AccountPage = 'main' | 'name' | 'email' | 'email-code'

function Avatar({ name }: { name: string }) {
  const initials = name.split(/\s+/).filter(Boolean).slice(0, 2).map((w) => w[0]!.toUpperCase()).join('') || '?'
  return <span aria-hidden="true" className="grid h-8 w-8 shrink-0 place-items-center rounded-full bg-paper-3 text-[12px] font-semibold text-ink-2">{initials}</span>
}

const STATE_TONE: Record<string, string> = { 'This app': 'text-ink', 'Signed in': 'text-ink-2', 'Not signed in': 'text-ink-3' }

export function AccountPane({ initial = 'main', scenario = 'default', codeDigits = '' }: { initial?: AccountPage; scenario?: Scenario; codeDigits?: string } = {}) {
  const { state, setState } = useAccount(scenario)
  const [page, setPage] = useState<AccountPage>(initial)
  const [pendingEmail, setPendingEmail] = useState(initial === 'email-code' ? 'anna.devries@example.eu' : '')
  const home = useCallback(() => setPage('main'), [])

  if (page === 'name') return <ChangeName current={state.name} onDone={(name) => { setState((s) => ({ ...s, name })); home() }} onCancel={home} />
  // The address is kept only for Send again (EmailCode → here); a cancel or a confirmed change starts the next visit empty.
  if (page === 'email') return <ChangeEmail initialEmail={pendingEmail} onSent={(email) => { setPendingEmail(email); setPage('email-code') }} onCancel={() => { setPendingEmail(''); home() }} />
  if (page === 'email-code') {
    return <EmailCode email={pendingEmail} initialDigits={codeDigits} onBack={() => setPage('email')}
      onDone={() => { setState((s) => ({ ...s, email: pendingEmail, emailVerified: true })); setPendingEmail(''); home() }} />
  }

  return (
    <Page title="Account">
      <Group>
        <Row label={<span className="flex items-center gap-3"><Avatar name={state.name} />{state.name || 'Add your name'}</span>} onClick={() => setPage('name')} />
        <Row label={state.email} sub={state.emailVerified ? 'Sign-in email' : undefined}
          value={state.emailVerified ? undefined : <span className="text-red">Not verified</span>} onClick={() => setPage('email')} />
        {!state.emailVerified ? <Row label="Verify" sub="We send a code to this address" onClick={() => setPage('email-code')} /> : null}
      </Group>
      <Group title="Recovery" note="We can’t show it again. Keep your 12 words on paper or in a password manager.">
        <div className="flex min-h-[44px] items-center px-4 py-2 text-[13px] text-ink" aria-label="Recovery phrase, shown once at sign-up" data-testid="row">
          Recovery phrase <span className="ml-1 text-ink-3">· Shown once at sign-up</span>
        </div>
      </Group>
      <Group title="This account uses" note="One Beebeeb account for every product. Not signed in on this PC is not an error.">
        {state.products.map((p) => (
          <Row key={p.name} label={p.name} value={<span className={STATE_TONE[p.state]}>{p.state}</span>} />
        ))}
      </Group>
    </Page>
  )
}

function ChangeName({ current, onDone, onCancel }: { current: string; onDone: (name: string) => void; onCancel: () => void }) {
  const [name, setName] = useState(current)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  useEscBack(onCancel)
  const save = async () => {
    const trimmed = name.trim()
    if (!trimmed) return setError(errorText('name_empty'))
    if (trimmed.length > 64) return setError(errorText('name_too_long'))
    setBusy(true)
    const r = await run('account_set_name', { name: trimmed }, null)
    setBusy(false)
    if (r.ok) onDone(trimmed)
    else setError(errorText(r.code))
  }
  return (
    <Page title="Name" sub="Your name in Drive, Pass, VPN, Chat and Mail, and on shared links where you choose to show it."
      back={<BackLink label="Account" onBack={onCancel} />}>
      <form className="flex flex-col gap-4" onSubmit={(e) => { e.preventDefault(); void save() }}>
        <Field label="Name" error={error}><TextInput label="Name" value={name} focus onChange={(v) => { setName(v); setError(null) }} error={!!error} /></Field>
        <div className="flex justify-end gap-2"><Btn onClick={onCancel}>Cancel</Btn><Btn variant="amber" disabled={busy} onClick={() => void save()}>Save</Btn></div>
      </form>
    </Page>
  )
}

function ChangeEmail({ initialEmail = '', onSent, onCancel }: { initialEmail?: string; onSent: (email: string) => void; onCancel: () => void }) {
  const [email, setEmail] = useState(initialEmail)
  const [password, setPassword] = useState('')
  const [error, setError] = useState<{ field: 'email' | 'password'; text: string } | null>(null)
  const [busy, setBusy] = useState(false)
  useEscBack(onCancel)
  const send = async () => {
    setBusy(true)
    const pw = password
    setPassword('')
    const r = await run<{ email: string }>('account_change_email', { newEmail: email, password: pw }, { email: email.trim().toLowerCase() })
    setBusy(false)
    if (r.ok) return onSent(r.value.email)
    setError({ field: r.code === 'wrong_password' ? 'password' : 'email', text: errorText(r.code) })
  }
  const valid = /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email.trim()) && password.length > 0
  return (
    <Page title="Change email" sub="We send an 8-digit code to the new address. Your recovery phrase and your files are not touched: the email is only the account’s name for signing in."
      back={<BackLink label="Account" onBack={onCancel} />}>
      <form className="flex flex-col gap-4" onSubmit={(e) => { e.preventDefault(); if (valid) void send() }}>
        <Field label="New email" error={error?.field === 'email' ? error.text : null}>
          <TextInput label="New email" type="email" value={email} focus onChange={(v) => { setEmail(v); setError(null) }} error={error?.field === 'email'} />
        </Field>
        <Field label="Password" error={error?.field === 'password' ? error.text : null} hint="Re-enter your password to start the email change process.">
          <PasswordInput label="Password" value={password} onChange={(v) => { setPassword(v); setError(null) }} error={error?.field === 'password'} />
        </Field>
        <p className="text-[12px] text-ink-3">Until the code is entered, your sign-in email stays the same. Both addresses get a notice when it changes.</p>
        <div className="flex justify-end gap-2"><Btn onClick={onCancel}>Cancel</Btn><Btn variant="amber" disabled={!valid || busy} onClick={() => void send()}>Send code</Btn></div>
      </form>
    </Page>
  )
}

function EmailCode({ email, initialDigits, onBack, onDone }: { email: string; initialDigits: string; onBack: () => void; onDone: () => void }) {
  const [code, setCode] = useState(initialDigits)
  const [error, setError] = useState<string | null>(null)
  const [wait, setWait] = useState(42)
  useEscBack(onBack)
  useEffect(() => {
    if (wait <= 0) return
    const t = window.setTimeout(() => setWait((w) => w - 1), 1000)
    return () => window.clearTimeout(t)
  }, [wait])
  const confirm = async () => {
    const c = code
    setCode('')
    const r = await run('account_verify_email', { code: c }, null)
    if (r.ok) onDone()
    else setError(r.code === 'invalid_code' ? 'That code is wrong or has expired. Check the newest email, or send a new code.' : errorText(r.code))
  }
  return (
    <Page title="Check your new address" sub={`We sent an 8-digit code to ${email}. It works for 15 minutes.`} back={<BackLink label="Change email" onBack={onBack} />}>
      <form className="flex flex-col gap-4" onSubmit={(e) => { e.preventDefault(); if (code.length === 8) void confirm() }}>
        <CodeBoxes length={8} label="8-digit code" value={code} onChange={(v) => { setCode(v); setError(null) }} error={!!error} focus />
        {error ? <p role="alert" className="text-[12px] text-red">{error}</p> : null}
        <p className="text-[12px] text-ink-3">
          {/* A new code is a new email change (the server's /me/email/start takes the password), so Send again goes
              back to Change email with the address kept: it never pretends a code went out. */}
          {wait > 0 ? <>Send again in <span className="font-mono">0:{String(wait).padStart(2, '0')}</span></> : <button type="button" onClick={onBack} className="text-ink underline">Send again</button>}
          {' · '}A new code asks for your password again.
        </p>
        <p className="text-[12px] text-ink-3">Sign-in on your other devices keeps working; their next sign-in uses the new address.</p>
        <div className="flex justify-end gap-2"><Btn onClick={onBack}>Back</Btn><Btn variant="amber" disabled={code.length !== 8} onClick={() => void confirm()}>Confirm</Btn></div>
      </form>
    </Page>
  )
}
