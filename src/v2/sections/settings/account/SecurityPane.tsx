// Sign-in and security (DW-A52) with the checkup (A84, A84a), change password (A67, A68), the two sign-out confirms
// (A57, A57a) and the four-step delete (A58, A80 to A83). Two-factor, passkeys and activity are in Factors.tsx.
// Drive's App lock sits above the shared rows (the securityExtra slot, DW-0716).
import { useCallback, useEffect, useState } from 'react'
import { Icon } from '@beebeeb/shared/design'
import { command } from '../../../../desktopApi'
import { hasTauri } from '../../../source'
import { Btn, Choice, Dialog, Group, Page, Row, Switch } from '../shell/kit'
import { useSettings } from '../shell/context'
import { useAccount, run, errorText, type AccountState, type Scenario } from './data'
import { FIXTURE_BACKUP_CODES, FIXTURE_NOW, FIXTURE_SHRED_AFTER } from '../../../source/fixtures/account'
import { Activity, BackupCodes, Passkeys, TotpSetup, TotpStatus } from './Factors'
import { BackLink, Check, CodeBoxes, Field, PasswordInput, Ring, ago, dateTime, useEscBack } from './ui'

export type SecurityPage =
  | 'main' | 'checkup' | 'password' | 'totp-setup' | 'backup-codes' | 'totp' | 'passkeys' | 'activity'
  | 'delete-1' | 'delete-2' | 'delete-3' | 'delete-4' | 'delete-done'

export interface SecurityInit {
  page?: SecurityPage
  scenario?: Scenario
  dialog?: 'signout' | 'signout-all' | 'totp-off' | 'activity-off'
  passwordError?: boolean
  digits?: string
  passkeyMenu?: string
  removePasskey?: string
}

// ── the checkup's words (SecurityView.tsx:54-62, shortened per DW-A84a) ───────
const FACTOR: Record<string, { title: string; doneState: (s: AccountState) => string; todo: string; to: SecurityPage | 'account' | 'devices' }> = {
  email_verified: { title: 'Email verified', doneState: (s) => s.email, todo: 'Verify your email', to: 'account' },
  two_factor_enabled: { title: 'Two-factor authentication', doneState: () => 'Authenticator app', todo: 'Add a second factor', to: 'totp' },
  strong_password: { title: 'Strong password', doneState: () => 'Strong when it was set', todo: 'Change your master password', to: 'password' },
  recovery_phrase_saved: { title: 'Recovery phrase saved', doneState: () => 'Confirmed at sign-up', todo: 'Recovery phrase saved', to: 'account' },
  passkey_added: { title: 'Passkeys', doneState: (s) => `${s.passkeys.length} passkey${s.passkeys.length === 1 ? '' : 's'}`, todo: 'Add a passkey', to: 'passkeys' },
  password_not_breached: { title: 'Password not in a breach', doneState: () => 'Checked when it was set', todo: 'Change your master password', to: 'password' },
  all_devices_recognized: { title: 'Devices and sessions', doneState: (s) => `${s.devices.length} signed in`, todo: 'Review your active sessions', to: 'devices' },
}

const DELAYS = [{ value: 'immediate', label: 'Immediately' }, { value: '5', label: '5 min' }, { value: '30', label: '30 min' }, { value: '60', label: '60 min' }] as const

export function SecurityPane({ init = {} }: { init?: SecurityInit } = {}) {
  const settings = useSettings()
  const { state, setState, reload } = useAccount(init.scenario)
  const [page, setPageRaw] = useState<SecurityPage>(init.page ?? 'main')
  const [dialog, setDialog] = useState(init.page ? undefined : init.dialog)
  const [codes, setCodes] = useState<string[]>(init.page === 'backup-codes' ? FIXTURE_BACKUP_CODES : [])
  const [shredAfter, setShredAfter] = useState(FIXTURE_SHRED_AFTER)
  const [moving, setMoving] = useState<string | undefined>()
  const setPage = useCallback((p: SecurityPage) => { setPageRaw(p); if (p === 'checkup') void reload() }, [reload])
  const home = useCallback(() => setPage('main'), [setPage])
  const go = (to: SecurityPage | 'account' | 'devices') => (to === 'account' || to === 'devices' ? settings.go(to) : setPage(to))
  const now = hasTauri() ? Date.now() : FIXTURE_NOW

  switch (page) {
    case 'checkup': return <Checkup state={state} onBack={home} go={go} />
    case 'password': return <ChangePassword onBack={home} onDone={() => { setState((s) => ({ ...s, passwordChangedAt: new Date().toISOString() })); home() }} showError={init.passwordError} />
    case 'totp-setup': return <TotpSetup moving={moving} initialDigits={init.digits} onBack={() => (state.totp.on ? setPage('totp') : home())}
      onCodes={(c) => { setCodes(c); setState((s) => ({ ...s, totp: { ...s.totp, on: true, codesLeft: 8, addedAt: new Date().toISOString() } })); setPage('backup-codes') }} />
    case 'backup-codes': return <BackupCodes codes={codes} onDone={() => { setCodes([]); setPage('totp') }} />
    case 'totp': return <TotpStatus state={state} onBack={home} dialog={init.dialog === 'totp-off' ? 'off' : undefined}
      onMove={() => { setMoving('current'); setPage('totp-setup') }} onNewCodes={() => { setMoving('current'); setPage('totp-setup') }}
      onTurnedOff={() => { setState((s) => ({ ...s, totp: { ...s.totp, on: false } })); home() }} />
    case 'passkeys': return <Passkeys state={state} setState={setState} onBack={home} menuOn={init.passkeyMenu ? state.passkeys.find((p) => p.name === init.passkeyMenu)?.id : undefined} removing={init.removePasskey} />
    case 'activity': return <Activity state={state} setState={setState} onBack={home} asking={init.dialog === 'activity-off'} now={now}
      onPassword={() => setPage('password')} onDevices={() => settings.go('devices')} />
    case 'delete-1': return <DeleteStep1 onBack={home} onNext={() => setPage('delete-2')} />
    case 'delete-2': return <DeleteStep2 totp={state.totp.on} initialDigits={init.digits} onBack={() => { void run('account_delete_cancel', {}, null); home() }}
      onNext={(d) => { setShredAfter(d.shred_after); setPage(d.email_link_sent ? 'delete-3' : 'delete-4') }} />
    case 'delete-3': return <DeleteStep3 email={state.email} onCancelled={home} onUsed={() => setPage('delete-4')} />
    case 'delete-4': return <DeleteStep4 shredAfter={shredAfter} onUndone={() => setPage('delete-done')} onSignOut={() => setDialog('signout')} dialog={dialog} closeDialog={() => setDialog(undefined)} />
    case 'delete-done': return <DeleteDone />
  }

  const t = state.totp
  const done = state.factors.filter((f) => f.satisfied).length
  return (
    <Page title="Sign-in and security" action={<Btn onClick={() => setPage('checkup')}>Security checkup · <span className="font-mono">{done} of {state.factors.length}</span></Btn>}>
      <AppLock />
      <Group title="Sign-in">
        <Row label="Password" value={state.passwordChangedAt ? `Changed ${ago(state.passwordChangedAt)}` : undefined} onClick={() => setPage('password')} />
        <Row label="Two-factor authentication" value={t.on ? 'On' : 'Off'} onClick={() => setPage(t.on ? 'totp' : 'totp-setup')} />
        <Row label="Passkeys" value={<span className="font-mono">{state.passkeys.length}</span>} onClick={() => setPage('passkeys')} />
        <Row label="Sign-in activity" value={state.activityLogging ? 'On' : 'Off'} onClick={() => setPage('activity')} />
      </Group>
      <Group title="Devices">
        <Row label="Devices and sessions" value={<span className="font-mono">{state.devices.length}</span>} onClick={() => settings.go('devices')} />
        <Row label="Approve a sign-in" sub="Type the code another device shows, or scan it" href="?window=main-app&frame=DW-A12" />
      </Group>
      <Group title="Sign out" note="Sign out of Drive signs out this app only. Your other devices stay signed in: use Devices and sessions for those.">
        <Row label={<span className="text-red">Sign out of Drive</span>} onClick={() => setDialog('signout')} />
        <Row label={<span className="text-red">Sign out of all Beebeeb apps on this PC</span>} onClick={() => setDialog('signout-all')} />
      </Group>
      <Group title="Delete" note="Every product’s data is deleted after 72 hours to undo. Then it can’t be undone.">
        <Row label={<span className="text-red">Delete your Beebeeb account</span>} onClick={() => setPage('delete-1')} />
      </Group>
      {dialog === 'signout' || dialog === 'signout-all' ? <SignOutDialog all={dialog === 'signout-all'} onCancel={() => setDialog(undefined)} /> : null}
    </Page>
  )
}

/** Drive's App lock row (the securityExtra slot). Windows Hello confirms a change first, as DW-0716 draws it. */
function AppLock() {
  const [on, setOn] = useState(true)
  const [delay, setDelay] = useState<(typeof DELAYS)[number]['value']>('5')
  const [hello, setHello] = useState<null | (() => void)>(null)
  return (
    <Group title="App lock">
      <Row label="Lock Beebeeb" sub={on ? 'Windows Hello · locks again after' : 'Off: anyone using this PC can open Beebeeb'}
        right={<>
          {on ? <Choice label="Lock again after" value={delay} options={[...DELAYS]} onPick={(d) => setHello(() => () => setDelay(d))} /> : null}
          <Switch label="Lock Beebeeb" on={on} onChange={(v) => setHello(() => () => setOn(v))} />
        </>} />
      {hello ? (
        <Dialog labelId="hello-b14-h" title="Confirm with Windows Hello" onCancel={() => setHello(null)}
          actions={<><Btn onClick={() => setHello(null)}>Cancel</Btn><Btn variant="amber" onClick={() => { hello(); setHello(null) }}>Continue</Btn></>}>
          Windows Hello confirms it is you before the lock changes.
        </Dialog>
      ) : null}
    </Group>
  )
}

function SignOutDialog({ all, onCancel }: { all: boolean; onCancel: () => void }) {
  // eslint-disable-next-line beebeeb/no-ad-hoc-error-surface -- the confirm stays open and says why the sign-out did not happen
  const [error, setError] = useState<string | null>(null)
  const signOut = async () => {
    if (!hasTauri()) return onCancel()
    const r = await command('clear_session')
    if (r.ok) window.location.assign(`?window=main-app&frame=${all ? 'DW-A03' : 'DW-A02'}`)
    else setError(r.reason)
  }
  return (
    <Dialog labelId="signout-h" title={all ? 'Sign out of all Beebeeb apps on this PC?' : 'Sign out of Drive?'} onCancel={onCancel}
      actions={<><Btn onClick={onCancel}>Cancel</Btn><Btn variant="danger" onClick={() => void signOut()}>{all ? 'Sign out of all' : 'Sign out'}</Btn></>}>
      {all ? (
        <>
          <p>Drive, Pass, VPN, Chat and Mail on this PC are signed out, and none of them offers Continue as until someone signs in again.</p>
          <p className="mt-2">Their data stays encrypted here. Your other devices stay signed in.</p>
        </>
      ) : (
        <>
          <p>Drive on this PC only. The other Beebeeb apps stay signed in.</p>
          <p className="mt-2">Your files stay encrypted here; signing in again opens them. Nothing is deleted.</p>
        </>
      )}
      {error ? <p role="alert" className="mt-2 text-[12px] text-red">{error}</p> : null}
    </Dialog>
  )
}

function Checkup({ state, onBack, go }: { state: AccountState; onBack: () => void; go: (to: SecurityPage | 'account' | 'devices') => void }) {
  useEscBack(onBack)
  const known = state.factors.filter((f) => FACTOR[f.key])
  const todo = known.filter((f) => !f.satisfied)
  const done = known.filter((f) => f.satisfied)
  return (
    <Page title="Security checkup" back={<BackLink label="Sign-in and security" onBack={onBack} />}>
      <div className="flex items-center gap-4 rounded-[12px] border border-line bg-paper px-4 py-3">
        <Ring done={done.length} total={known.length} />
        <div>
          <div className="text-[15px] font-semibold text-ink"><span className="font-mono">{done.length} of {known.length}</span> done</div>
          <div className="text-[13px] text-ink-3">{todo.length === 0 ? 'Nothing to do.' : todo.length === 1 ? 'One thing to do.' : `${todo.length} things to do.`}</div>
        </div>
      </div>
      {todo.length ? (
        <Group title="To do">
          {todo.map((f) => (
            <Row key={f.key} label={FACTOR[f.key]!.title} sub={FACTOR[f.key]!.todo}
              right={<Btn variant="amber" onClick={() => go(FACTOR[f.key]!.to)}>{f.key === 'passkey_added' ? 'Add' : 'Fix'}</Btn>} />
          ))}
        </Group>
      ) : null}
      <Group title="Done">
        {done.map((f) => (
          <Row key={f.key} label={<span className="flex items-center gap-2"><Check />{FACTOR[f.key]!.title}</span>}
            value={FACTOR[f.key]!.doneState(state)} onClick={() => go(FACTOR[f.key]!.to)} />
        ))}
      </Group>
    </Page>
  )
}

function strength(pw: string): { label: string; ok: boolean } {
  if (pw.length === 0) return { label: '', ok: false }
  if (pw.length < 12) return { label: `Too short: ${pw.length} of 12 characters`, ok: false }
  const kinds = [/[a-z]/, /[A-Z]/, /\d/, /[^A-Za-z0-9]/].filter((r) => r.test(pw)).length
  return pw.length >= 16 || kinds >= 3 ? { label: 'Strong', ok: true } : { label: 'Fair: a longer passphrase is stronger', ok: true }
}

function ChangePassword({ onBack, onDone, showError }: { onBack: () => void; onDone: () => void; showError?: boolean }) {
  const [current, setCurrent] = useState('')
  const [next, setNext] = useState(showError ? 'river-lantern-quiet-74' : '')
  const [again, setAgain] = useState(showError ? 'river-lantern-quiet-74' : '')
  const [error, setError] = useState<{ field: 'current' | 'new' | 'again'; text: string } | null>(
    showError ? { field: 'current', text: 'Current password is incorrect. Nothing was changed.' } : null)
  const [busy, setBusy] = useState(false)
  useEscBack(onBack)
  useEffect(() => () => { setCurrent(''); setNext(''); setAgain('') }, [])
  const s = strength(next)
  const valid = current.length > 0 && s.ok && next === again
  const submit = async () => {
    if (next !== again) return setError({ field: 'again', text: 'Passwords do not match' })
    if (!valid) return
    setBusy(true)
    const c = current
    setCurrent('')
    const r = await run('account_change_password', { current: c, newPassword: next }, null)
    setBusy(false)
    if (r.ok) { setNext(''); setAgain(''); return onDone() }
    if (r.code === 'wrong_password') return setError({ field: 'current', text: 'Current password is incorrect. Nothing was changed.' })
    setError({ field: r.code === 'weak_password' || r.code === 'same_password' ? 'new' : 'current', text: errorText(r.code) })
  }
  return (
    <Page title="Change password" back={<BackLink label="Sign-in and security" onBack={onBack} />}>
      <form className="flex flex-col gap-4" onSubmit={(e) => { e.preventDefault(); void submit() }}>
        <Field label="Current password" error={error?.field === 'current' ? error.text : null}>
          <PasswordInput label="Current password" value={current} onChange={(v) => { setCurrent(v); setError(null) }} error={error?.field === 'current'} focus />
        </Field>
        <Field label="New password" error={error?.field === 'new' ? error.text : null} hint={s.label ? <span data-testid="strength">{s.label}</span> : 'At least 12 characters'}>
          <PasswordInput label="New password" isNew value={next} onChange={(v) => { setNext(v); if (error?.field !== 'current') setError(null) }} error={error?.field === 'new'} />
        </Field>
        <Field label="New password again" error={error?.field === 'again' ? error.text : null}>
          <PasswordInput label="New password again" isNew value={again} onChange={(v) => { setAgain(v); if (error?.field === 'again') setError(null) }} error={error?.field === 'again'} />
        </Field>
        <p className="rounded-md bg-paper-2 px-3 py-2 text-[12px] text-ink-2">
          Your data stays encrypted as it is. Devices that never unlocked it are signed out; your other devices get an alert. The new password is checked against known leaks on Beebeeb’s own servers.
        </p>
        <div className="flex justify-end gap-2"><Btn onClick={onBack}>Cancel</Btn><Btn variant="amber" disabled={!valid || busy} onClick={() => void submit()}>Change password</Btn></div>
      </form>
    </Page>
  )
}

function StepHead({ n }: { n: number }) {
  return <p className="font-mono text-[11px] uppercase tracking-wide text-ink-3">Step {n} of 4</p>
}

function DeleteStep1({ onBack, onNext }: { onBack: () => void; onNext: () => void }) {
  const [password, setPassword] = useState('')
  const [error, setError] = useState<string | null>(null)
  useEscBack(onBack)
  const next = async () => {
    const pw = password
    setPassword('')
    const r = await run('account_delete_check_password', { password: pw }, null)
    if (r.ok) onNext()
    else setError(r.code === 'wrong_password' ? 'Incorrect password. Nothing was deleted.' : errorText(r.code))
  }
  return (
    <Page title="Delete your Beebeeb account?" sub="After you confirm by email, you have 72 hours to undo it. After that, it can’t be undone." back={<BackLink label="Sign-in and security" onBack={onBack} />}>
      <StepHead n={1} />
      <Group title="What gets deleted">
        <Row label="Every product’s data" sub="Drive, Pass, VPN, Chat and Mail" />
        <Row label="Shared links and team access" />
        <Row label="Every device" sub="Each one is signed out" />
        <Row label="Audit logs" sub="Kept 12 months, as the law requires" />
      </Group>
      <form className="flex flex-col gap-4" onSubmit={(e) => { e.preventDefault(); if (password) void next() }}>
        <Field label="Password" error={error}><PasswordInput label="Password" value={password} onChange={(v) => { setPassword(v); setError(null) }} error={!!error} focus /></Field>
        <div className="flex justify-end gap-2"><Btn onClick={onBack}>Cancel</Btn><Btn variant="danger" disabled={!password} onClick={() => void next()} aria-label="Continue, destructive">Continue</Btn></div>
      </form>
    </Page>
  )
}

function DeleteStep2({ totp, initialDigits = '', onBack, onNext }: { totp: boolean; initialDigits?: string; onBack: () => void; onNext: (d: { shred_after: string; email_link_sent: boolean }) => void }) {
  const [backup, setBackup] = useState(false)
  const [code, setCode] = useState(initialDigits)
  const [error, setError] = useState<string | null>(null)
  useEscBack(onBack)
  const len = backup ? 8 : 6
  const next = async () => {
    const c = code
    setCode('')
    const r = await run<{ shred_after: string; email_link_sent: boolean }>('account_delete_confirm', { code: totp ? c : null }, { shred_after: FIXTURE_SHRED_AFTER, email_link_sent: true })
    if (r.ok) onNext(r.value)
    else setError(errorText(r.code))
  }
  return (
    <Page title="Confirm it’s you" sub={totp ? (backup ? 'Enter one of your 8-digit backup codes.' : 'Enter a code from your authenticator app.') : 'Windows Hello confirms it’s you on this PC; the email link in the next step is what Beebeeb checks.'}
      back={<BackLink label="Step 1" onBack={onBack} />}>
      <StepHead n={2} />
      {totp ? (
        <form className="flex flex-col gap-3" onSubmit={(e) => { e.preventDefault(); if (code.length === len) void next() }}>
          <CodeBoxes length={len} label={`${len}-digit code`} value={code} onChange={(v) => { setCode(v); setError(null) }} error={!!error} focus />
          {error ? <p role="alert" className="text-[12px] text-red">{error}</p> : null}
          <button type="button" onClick={() => { setBackup((b) => !b); setCode('') }} className="text-[12px] text-ink underline">{backup ? 'Use your authenticator' : 'Use a backup code'}</button>
          <div className="flex justify-end gap-2"><Btn onClick={onBack}>Cancel</Btn><Btn variant="danger" disabled={code.length !== len} onClick={() => void next()} aria-label="Continue, destructive">Continue</Btn></div>
        </form>
      ) : (
        <div className="flex justify-end gap-2"><Btn onClick={onBack}>Cancel</Btn><Btn variant="danger" onClick={() => void next()}>Continue with Windows Hello</Btn></div>
      )}
    </Page>
  )
}

function DeleteStep3({ email, onCancelled, onUsed }: { email: string; onCancelled: () => void; onUsed: () => void }) {
  const [wait, setWait] = useState(38)
  useEffect(() => {
    if (wait <= 0) return
    const t = window.setTimeout(() => setWait((w) => w - 1), 1000)
    return () => window.clearTimeout(t)
  }, [wait])
  return (
    <Page title="Check your email">
      <StepHead n={3} />
      <div role="status" className="rounded-[12px] border border-line bg-paper-2 px-4 py-3 text-[13px] text-ink">
        We sent a confirmation link to <span className="font-medium">{email}</span>. Nothing is deleted until you use that link.
      </div>
      <p className="text-[13px] text-ink-2">The link works once, for 24 hours. Didn’t ask for this? Cancel the deletion here, then change your password.</p>
      <p className="text-[12px] text-ink-3">
        {wait > 0 ? <>Send again in <span className="font-mono">0:{String(wait).padStart(2, '0')}</span></> : <button type="button" onClick={() => { setWait(60) }} className="text-ink underline">Send again</button>}
        {' · '}This screen moves on by itself once the link is used.
      </p>
      <div className="flex justify-end gap-2">
        <Btn onClick={() => { void run('account_delete_cancel', {}, null); onCancelled() }}>Cancel deletion</Btn>
        <Btn onClick={onUsed}>I’ve opened the link</Btn>
      </div>
    </Page>
  )
}

function DeleteStep4({ shredAfter, onUndone, onSignOut, dialog, closeDialog }: { shredAfter: string; onUndone: () => void; onSignOut: () => void; dialog?: string; closeDialog: () => void }) {
  const [error, setError] = useState<string | null>(null)
  const undo = async () => {
    const r = await run('account_delete_undo', {}, null)
    if (r.ok) onUndone()
    else setError(errorText(r.code))
  }
  return (
    <Page title="Deletion scheduled">
      <StepHead n={4} />
      <div className="rounded-[12px] border border-line bg-paper px-4 py-3">
        <div className="text-[13px] text-ink-2">Your account and every product’s data will be deleted on</div>
        <div className="mt-1 text-[17px] font-semibold text-ink" data-testid="shred-at">{dateTime(shredAfter)}</div>
      </div>
      <Group title="Changed your mind?">
        <Row label="Undo from this screen" />
        <Row label="Or use the link in the second email we sent" />
        <Row label="Or sign in to Beebeeb within the 72 hours" />
      </Group>
      <p className="text-[12px] text-ink-3">Until then your account works as usual on every device. After that, every product’s data is deleted and shredded within 30 days.</p>
      {error ? <p role="alert" className="text-[12px] text-red">{error}</p> : null}
      <div className="flex justify-end gap-2"><Btn onClick={onSignOut}>Sign out</Btn><Btn variant="amber" onClick={() => void undo()}>Undo deletion</Btn></div>
      {dialog === 'signout' ? <SignOutDialog all={false} onCancel={closeDialog} /> : null}
    </Page>
  )
}

function DeleteDone() {
  return (
    <Page title="">
      <div className="flex flex-col items-center gap-3 py-12 text-center" role="status">
        <span className="grid h-12 w-12 place-items-center rounded-full bg-green-bg text-green-text"><Icon name="check" size={22} /></span>
        <h1 className="text-[20px] font-semibold text-ink">Deletion cancelled</h1>
        <p className="max-w-[380px] text-[13px] text-ink-2">Your account is as it was. Nothing was deleted. We sent an email to say so.</p>
        <a href="?window=main-app&section=files" className="mt-2 rounded-md bg-amber px-3 py-1.5 text-[13px] font-medium text-[oklch(0.22_0.01_70)]">Continue to Drive</a>
      </div>
    </Page>
  )
}
