// Two-factor (DW-A69 to A72), passkeys (A73 to A76) and sign-in activity (A77 to A79). Shared account screens:
// every product draws them the same. Every answer comes from its account_totp / account_passkeys_* command; a plain
// browser shows the host's honest refusal, never an example key or codes.
import { useEffect, useRef, useState } from 'react'
import { Icon } from '@beebeeb/shared/design'
import { Dialog, Group, Page, Row, Switch } from '../shell/kit'
import { run, errorText, copyFor60s, COPY_DONE, COPY_FAILED, type AccountState } from './data'
import type { Passkey } from './types'
import { Btn, BackLink, CodeBoxes, Field, PasswordInput, Pill, Qr, ago, day, useEscBack } from './ui'

type Set = (f: (s: AccountState) => AccountState) => void

// ── Two-factor ──────────────────────────────────────────────────────────────

export function TotpSetup({ onCodes, onBack, initialDigits = '', moving }: { onCodes: (codes: string[]) => void; onBack: () => void; initialDigits?: string; moving?: string }) {
  const [setup, setSetup] = useState<{ secret: string; qr_rows: string[] } | null>(null)
  const [code, setCode] = useState(initialDigits)
  const [error, setError] = useState<string | null>(null)
  const [copied, setCopied] = useState<boolean | null>(null)
  useEscBack(onBack)
  useEffect(() => {
    let live = true
    void run<{ secret: string; otpauth_uri: string; qr_rows: string[] }>('account_totp_setup', { password: null, currentCode: moving ?? null }).then((r) => {
      if (!live) return
      if (r.ok) setSetup(r.value)
      else setError(errorText(r.code))
    })
    return () => { live = false; void run('account_totp_abandon', {}) }
  }, [moving])
  const turnOn = async () => {
    const c = code
    setCode('')
    const r = await run<{ backup_codes: string[] }>('account_totp_confirm', { code: c })
    if (r.ok) onCodes(r.value.backup_codes)
    else setError(errorText(r.code))
  }
  return (
    <Page title="Set up two-factor" sub="Scan the QR code with your authenticator app, then enter the 6-digit code it shows." back={<BackLink label="Sign-in and security" onBack={onBack} />}>
      <div className="flex gap-6">
        {setup ? <Qr rows={setup.qr_rows} alt={`Authenticator key ${setup.secret}`} /> : <div className="h-[148px] w-[148px] rounded-md border border-line bg-paper-2" aria-busy="true" />}
        <div className="flex min-w-0 flex-1 flex-col gap-3">
          <Field label="Or enter this key in an authenticator on this PC">
            <div className="flex items-center gap-2">
              <code className="rounded-md border border-line bg-paper-2 px-2 py-1 font-mono text-[13px] tracking-wider text-ink" data-testid="totp-key">{setup ? setup.secret.replace(/(.{4})/g, '$1 ').trim() : '…'}</code>
              <Btn disabled={!setup} onClick={() => { if (setup) void copyFor60s(setup.secret).then(setCopied) }}>{copied ? 'Copied' : 'Copy key'}</Btn>
            </div>
          </Field>
          {copied != null ? <p role="status" className="text-[12px] text-ink-3">{copied ? COPY_DONE : COPY_FAILED}</p> : null}
          <Field label="6-digit code" error={error}>
            <CodeBoxes length={6} label="6-digit code" value={code} onChange={(v) => { setCode(v); setError(null) }} error={!!error} focus />
          </Field>
        </div>
      </div>
      <p className="text-[12px] text-ink-3">Moving to a new authenticator replaces the key and the backup codes.</p>
      <div className="flex justify-end gap-2"><Btn onClick={onBack}>Cancel</Btn><Btn variant="amber" disabled={code.length !== 6 || !setup} onClick={() => void turnOn()}>Turn on</Btn></div>
    </Page>
  )
}

export function BackupCodes({ codes, onDone }: { codes: string[]; onDone: () => void }) {
  const [leaving, setLeaving] = useState(false)
  const [note, setNote] = useState<string | null>(null)
  useEscBack(() => setLeaving(true), !leaving)
  const text = `Beebeeb backup codes\nEach code works once.\n\n${codes.map((c, i) => `${i + 1}. ${c}`).join('\n')}\n`
  return (
    <Page title="Your backup codes" sub="Each code signs you in once when you don’t have your authenticator. Save them somewhere safe: they won’t be shown again.">
      <ol className="grid grid-cols-2 gap-x-8 gap-y-2 rounded-[12px] border border-line bg-paper px-6 py-4" data-testid="backup-codes">
        {codes.map((c, i) => (
          <li key={c} className="flex gap-3 font-mono text-[15px] text-ink" aria-label={`Code ${i + 1}: ${c.split('').join(' ')}`}>
            <span className="w-4 text-right text-ink-4">{i + 1}</span><span className="tracking-wider">{c.slice(0, 4)} {c.slice(4)}</span>
          </li>
        ))}
      </ol>
      <div className="flex items-center gap-2">
        <Btn onClick={() => void copyFor60s(text).then((ok) => setNote(ok ? COPY_DONE : COPY_FAILED))}>Copy all codes</Btn>
        <Btn onClick={() => void run<boolean>('account_save_backup_codes', { text }).then((r) => { if (!r.ok) setNote(errorText(r.code)); else if (r.value) setNote('Saved.') })}>Save as text…</Btn>
        {note ? <span role="status" className="text-[12px] text-ink-3">{note}</span> : null}
        <span className="flex-1" />
        <Btn variant="amber" onClick={onDone}>I’ve saved them</Btn>
      </div>
      {leaving ? (
        <Dialog labelId="codes-leave-h" title="Did you save your backup codes?" onCancel={() => setLeaving(false)}
          actions={<><Btn onClick={() => setLeaving(false)}>Back to the codes</Btn><Btn onClick={onDone}>Leave</Btn></>}>
          They won’t be shown again.
        </Dialog>
      ) : null}
    </Page>
  )
}

export function TotpStatus({ state, onBack, onNewCodes, onMove, onTurnedOff, dialog, read, onRetry }: {
  /** `onMove(code)`: the current code the person typed, for the new set-up's step-up (DW-A71 → A69). */
  state: AccountState
  onBack: () => void
  onNewCodes: (codes: string[]) => void
  onMove: (code: string) => void
  onTurnedOff: () => void
  dialog?: 'off' | 'new-codes' | 'move'
  /** The authenticator read. Absent: `state.totp`. A failed read offers no backup-code action. */
  read?: { ok: true; value: { on: boolean; addedAt: string | null; lastUsedAt: string | null; codesLeft: number | null; codesTotal: number | null } } | { ok: false; code: string }
  onRetry?: () => void
}) {
  const facts = read ? (read.ok ? read.value : null) : {
    on: state.totp.on,
    addedAt: state.totp.addedAt,
    lastUsedAt: state.totp.lastUsedAt,
    codesLeft: state.totp.codesLeft,
    // The default total is not a count the server reported.
    codesTotal: state.totp.codesLeft == null ? null : state.totp.codesTotal,
  }
  const [ask, setAsk] = useState<'off' | 'new-codes' | 'move' | null>(facts?.on ? (dialog ?? null) : null)
  useEscBack(onBack, !ask)
  if (read && !read.ok) {
    return (
      <Page title="Two-factor authentication" back={<BackLink label="Sign-in and security" onBack={onBack} />}>
        <p role="alert" className="text-[13px] text-red">{errorText(read.code)}</p>
        <div><Btn onClick={onRetry}>Try again</Btn></div>
      </Page>
    )
  }
  if (!facts || !facts.on) {
    return (
      <Page title="Two-factor authentication" back={<BackLink label="Sign-in and security" onBack={onBack} />}>
        <Group>
          <Row label="Authenticator app" value="Off" />
        </Group>
      </Page>
    )
  }
  const low = facts.codesLeft != null && facts.codesLeft <= 2
  const codes = facts.codesLeft == null ? 'Not reported' : facts.codesTotal == null ? String(facts.codesLeft) : `${facts.codesLeft} of ${facts.codesTotal}`
  return (
    <Page title="Two-factor authentication" back={<BackLink label="Sign-in and security" onBack={onBack} />}
      action={<Pill tone="green">Two-factor on</Pill>}>
      <Group>
        <Row label="Authenticator app" sub={`Added ${facts.addedAt ? day(facts.addedAt) : 'Not reported'} · Last used ${facts.lastUsedAt ? ago(facts.lastUsedAt) : 'Not reported'}`} value="On" />
      </Group>
      <Group title="Backup codes" note="Each backup code works once.">
        <Row label={<span className={low ? 'text-red' : undefined}>Codes left</span>} sub={low ? <span className="text-red">Make new codes before you run out.</span> : undefined}
          value={facts.codesLeft == null ? 'Not reported' : <span className={`font-mono ${low ? 'text-red' : ''}`}>{codes}</span>} />
        <Row label="Make new backup codes" sub="The old codes stop working" onClick={() => setAsk('new-codes')} />
      </Group>
      <Group note="Without your authenticator you can still get in with a backup code, another signed-in device, or your 12 words.">
        <Row label="Move to a new authenticator" onClick={() => setAsk('move')} />
        <Row label={<span className="text-red">Turn off two-factor</span>} onClick={() => setAsk('off')} />
      </Group>
      {ask ? <CodeDialog kind={ask} onCancel={() => setAsk(null)} onOk={(c, codesBack) => { const k = ask; setAsk(null); if (k === 'off') onTurnedOff(); else if (k === 'move') onMove(c); else onNewCodes(codesBack ?? []) }} /> : null}
    </Page>
  )
}

/** A current code first: turning off (DW-A72), new backup codes and moving (the current code, then set-up). */
function CodeDialog({ kind, onCancel, onOk }: { kind: 'off' | 'new-codes' | 'move'; onCancel: () => void; onOk: (code: string, codes?: string[]) => void }) {
  const [code, setCode] = useState('')
  const [error, setError] = useState<string | null>(null)
  const ok = async () => {
    const c = code
    setCode('')
    if (kind === 'off') {
      const r = await run('account_totp_disable', { code: c })
      if (!r.ok) return setError(errorText(r.code))
    }
    if (kind === 'new-codes') {
      // Server #422: new codes with the current code; the authenticator stays (no new set-up).
      const r = await run<{ backup_codes: string[] }>('account_backup_codes_regenerate', { code: c })
      if (!r.ok) return setError(errorText(r.code))
      return onOk(c, r.value.backup_codes)
    }
    onOk(c)
  }
  const title = kind === 'off' ? 'Turn off two-factor?' : kind === 'move' ? 'Move to a new authenticator' : 'Make new backup codes'
  return (
    <Dialog labelId="totp-code-h" title={title} onCancel={onCancel}
      actions={<><Btn onClick={onCancel}>Cancel</Btn>
        <Btn variant={kind === 'off' ? 'danger' : 'amber'} disabled={code.length !== 6} onClick={() => void ok()}
          aria-label={kind === 'off' ? 'Turn off two-factor, destructive' : undefined}>{kind === 'off' ? 'Turn off' : 'Continue'}</Btn></>}>
      <p className="mb-3">{kind === 'off'
        ? 'Enter your current authenticator code to turn off 2FA. Signing in then needs only your password, and your other devices get an alert.'
        : kind === 'move' ? 'Enter the current code from your authenticator. Then scan a new QR code.' : 'Enter the current code from your authenticator. You get new codes and the old ones stop working.'}</p>
      <CodeBoxes length={6} label="6-digit code" value={code} onChange={(v) => { setCode(v); setError(null) }} error={!!error} focus />
      {error ? <p role="alert" className="mt-2 text-[12px] text-red">{error}</p> : null}
      {kind === 'off' ? <p className="mt-3 text-[12px] text-ink-3">Lost your authenticator? Don’t turn it off here: sign in with a backup code, another device or your 12 words first.</p> : null}
    </Dialog>
  )
}

// ── Passkeys ────────────────────────────────────────────────────────────────

/** What the list actually said about unlocking. `null` is not "signs in only". */
function passkeyCapability(prf: boolean | null | undefined): string {
  if (prf === true) return 'Signs in and unlocks your data'
  if (prf === false) return 'Signs in only'
  return 'Not reported'
}

export function Passkeys({ state, setState, onBack, menuOn, removing: initialRemoving, read, onRetry }: {
  state: AccountState
  setState: Set
  onBack: () => void
  menuOn?: string
  removing?: string
  /** The passkey list read. Absent: `state.passkeys`. A name that is not in the list cannot be removed. */
  read?: { ok: true; value: Passkey[] } | { ok: false; code: string }
  onRetry?: () => void
}) {
  const list = read ? (read.ok ? read.value : []) : state.passkeys
  const [menu, setMenu] = useState<string | null>(list.some((p) => p.id === menuOn) ? (menuOn ?? null) : null)
  const [removing, setRemoving] = useState<Passkey | null>(() => list.find((p) => p.name === initialRemoving) ?? null)
  const [adding, setAdding] = useState(false)
  const [error, setError] = useState<string | null>(null)
  useEscBack(onBack, !menu && !removing && !adding)
  if (read && !read.ok) {
    return (
      <Page title="Passkeys" back={<BackLink label="Sign-in and security" onBack={onBack} />}>
        <p role="alert" className="text-[13px] text-red">{errorText(read.code)}</p>
        <div><Btn onClick={onRetry}>Try again</Btn></div>
      </Page>
    )
  }
  const count = `${list.length} ${list.length === 1 ? 'passkey' : 'passkeys'}`
  return (
    <Page title="Passkeys" back={<BackLink label="Sign-in and security" onBack={onBack} />}
      sub="An extra way to confirm it’s you and to unlock your data on a new device. It doesn’t replace your 12 words."
      action={list.length ? <Btn variant="amber" onClick={() => setAdding(true)}>Add a passkey</Btn> : undefined}>
      <p className="text-[13px] text-ink">{count}</p>
      {error ? <p role="alert" className="text-[12px] text-red">{error}</p> : null}
      {list.length === 0 ? (
        <div className="flex flex-col items-center gap-2 rounded-[12px] border border-line bg-paper px-6 py-10 text-center" data-testid="passkeys-empty">
          <Icon name="key" size={22} className="text-ink-3" />
          <h2 className="text-[15px] font-semibold text-ink">No passkeys yet</h2>
          <p className="max-w-[360px] text-[13px] text-ink-3">Add one with Windows Hello: it is saved on this PC and confirms it’s you with your face, fingerprint or PIN.</p>
          <div className="mt-2"><Btn variant="amber" onClick={() => setAdding(true)}>Add a passkey</Btn></div>
        </div>
      ) : (
        <Group note="Whether a passkey can unlock your data depends on the password manager that keeps it.">
          {list.map((p) => {
            const capability = passkeyCapability(p.prf)
            const addedOn = p.created_at ? day(p.created_at) : 'Not reported'
            const lastUsed = p.last_used_at ? ago(p.last_used_at) : 'Not reported'
            return (
              <div key={p.id} className="relative flex min-h-[44px] items-center gap-3 px-4 py-2" data-testid="passkey-row"
                onContextMenu={(e) => { e.preventDefault(); setMenu(p.id) }}
                aria-label={`${p.name}, added ${addedOn}, last used ${lastUsed}, ${capability}`}>
                <Icon name="key" size={15} className="shrink-0 text-ink-3" />
                <div className="min-w-0 flex-1">
                  <div className="text-[13px] leading-5 text-ink">{p.name}</div>
                  <div className="text-[12px] leading-4 text-ink-3">Added {addedOn} · Last used {lastUsed} · {capability}</div>
                </div>
                <button type="button" aria-label={`More for ${p.name}`} aria-haspopup="menu" onClick={() => setMenu(menu === p.id ? null : p.id)}
                  className="grid h-7 w-7 place-items-center rounded-md text-ink-3 hover:bg-paper-2"><Icon name="more" size={15} /></button>
                {menu === p.id ? <PasskeyMenu onClose={() => setMenu(null)} onRemove={() => { setMenu(null); setRemoving(p) }} /> : null}
              </div>
            )
          })}
        </Group>
      )}
      {adding ? <AddPasskey onCancel={() => setAdding(false)} onAdded={() => {
        setAdding(false)
        void run<Passkey[]>('account_passkeys_list', {}).then((listed) => {
          if (!listed.ok) { setError(errorText(listed.code)); return }
          setState((s) => ({ ...s, passkeys: listed.value }))
        })
      }} /> : null}
      {removing && list.some((p) => p.id === removing.id) ? <RemovePasskey passkey={removing} others={list.filter((p) => p.id !== removing.id)} onCancel={() => setRemoving(null)}
        onRemoved={() => { const id = removing.id; setRemoving(null); setState((s) => ({ ...s, passkeys: s.passkeys.filter((p) => p.id !== id) })) }}
        onError={(code) => { setRemoving(null); setError(errorText(code)) }} /> : null}
    </Page>
  )
}

function PasskeyMenu({ onClose, onRemove }: { onClose: () => void; onRemove: () => void }) {
  const ref = useRef<HTMLUListElement>(null)
  useEffect(() => {
    ref.current?.querySelector<HTMLElement>('button')?.focus()
    const off = (e: MouseEvent) => { if (!ref.current?.contains(e.target as Node)) onClose() }
    const key = (e: KeyboardEvent) => { if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); onClose() } }
    document.addEventListener('mousedown', off)
    document.addEventListener('keydown', key, true)
    return () => { document.removeEventListener('mousedown', off); document.removeEventListener('keydown', key, true) }
  }, [onClose])
  const item = 'flex w-full px-3 py-1.5 text-left text-[13px] hover:bg-paper-2'
  return (
    <ul ref={ref} role="menu" className="absolute right-4 top-10 z-30 min-w-[140px] rounded-lg border border-line bg-paper py-1 shadow-lg" data-testid="passkey-menu">
      <li role="none"><button type="button" role="menuitem" className={`${item} text-red`} onClick={onRemove} aria-label="Remove, destructive">Remove</button></li>
    </ul>
  )
}

function AddPasskey({ onCancel, onAdded }: { onCancel: () => void; onAdded: () => void }) {
  const [password, setPassword] = useState('')
  const [error, setError] = useState<string | null>(null)
  const add = async () => {
    const pw = password
    setPassword('')
    const name = 'Windows Hello'
    const r = await run<{ id: string }>('account_passkeys_add', { name, password: pw })
    if (!r.ok) return setError(errorText(r.code))
    onAdded()
  }
  return (
    <Dialog labelId="pk-add-h" title="Add a passkey" onCancel={onCancel}
      actions={<><Btn onClick={onCancel}>Cancel</Btn><Btn variant="amber" disabled={!password} onClick={() => void add()}>Continue</Btn></>}>
      <p className="mb-3">Confirm your identity before adding a new passkey. Then Windows Hello asks for your face, fingerprint or PIN.</p>
      <Field label="Password" error={error}><PasswordInput label="Password" value={password} onChange={(v) => { setPassword(v); setError(null) }} error={!!error} focus /></Field>
    </Dialog>
  )
}

function RemovePasskey({ passkey, others, onCancel, onRemoved, onError }: { passkey: Passkey; others: Passkey[]; onCancel: () => void; onRemoved: () => void; onError: (code: string) => void }) {
  const [password, setPassword] = useState('')
  const lastUnlock = passkey.prf && !others.some((p) => p.prf)
  const remove = async () => {
    const pw = password
    setPassword('')
    const r = await run('account_passkeys_remove', { id: passkey.id, password: pw })
    if (r.ok) onRemoved()
    else onError(r.code)
  }
  return (
    <Dialog labelId="pk-remove-h" title={`Remove ${passkey.name}?`} onCancel={onCancel}
      actions={<><Btn onClick={onCancel}>Cancel</Btn><Btn variant="danger" disabled={!password} onClick={() => void remove()} aria-label={`Remove ${passkey.name}, destructive`}>Remove</Btn></>}>
      {lastUnlock
        ? <p className="mb-2 font-semibold text-ink">This is your last passkey that can unlock your data. Without it, a new device needs another signed-in device or your 12 words.</p>
        : <p className="mb-2">You can still sign in with your other passkeys.</p>}
      <p className="mb-3">The copy in your password manager stays until you remove it there.</p>
      <Field label="Password"><PasswordInput label="Password" value={password} onChange={setPassword} focus /></Field>
    </Dialog>
  )
}

// ── Sign-in activity ────────────────────────────────────────────────────────

function period(iso: string, now: number): string {
  const days = (now - Date.parse(iso)) / 86_400_000
  return days < 1 ? 'Today' : days < 7 ? 'This week' : days < 31 ? 'This month' : 'Earlier'
}

export function Activity({ state, setState, onBack, onPassword, onDevices, asking, now, read, onRetry }: {
  state: AccountState
  setState: Set
  onBack: () => void
  onPassword: () => void
  onDevices: () => void
  asking?: boolean
  now: number
  /** The logging read and its events. Absent: `state`. A failed read offers Try again and no switch. */
  read?: { ok: true; value: { on: boolean; events: AccountState['activity'] } } | { ok: false; code: string }
  onRetry?: () => void
}) {
  const reported = !read ? { on: state.activityLogging, events: state.activity } : read.ok ? read.value : null
  const [ask, setAsk] = useState(!!asking && !!reported?.on)
  const [password, setPassword] = useState('')
  const [error, setError] = useState<string | null>(null)
  useEscBack(onBack, !ask)
  if (read && !read.ok) {
    return (
      <Page title="Sign-in activity" back={<BackLink label="Sign-in and security" onBack={onBack} />}>
        <p role="alert" className="text-[13px] text-red">{errorText(read.code)}</p>
        <div><Btn onClick={onRetry}>Try again</Btn></div>
      </Page>
    )
  }
  const shown = reported ?? { on: state.activityLogging, events: state.activity }
  const on = shown.on
  const toggle = async (next: boolean) => {
    if (!next) return setAsk(true)
    const r = await run('account_set_activity_logging', { on: true, password: null })
    if (r.ok) setState((s) => ({ ...s, activityLogging: true }))
    else setError(errorText(r.code))
  }
  const turnOff = async () => {
    const pw = password
    setPassword('')
    const r = await run('account_set_activity_logging', { on: false, password: pw })
    setAsk(false)
    if (r.ok) setState((s) => ({ ...s, activityLogging: false }))
    else setError(errorText(r.code))
  }
  const groups = new Map<string, typeof state.activity>()
  const newest = [...shown.events].sort((a, b) => (a.at < b.at ? 1 : a.at > b.at ? -1 : 0))
  for (const e of newest) groups.set(period(e.at, now), [...(groups.get(period(e.at, now)) ?? []), e])
  return (
    <Page title="Sign-in activity" back={<BackLink label="Sign-in and security" onBack={onBack} />}>
      {error ? <p role="alert" className="text-[12px] text-red">{error}</p> : null}
      <Group note={on ? 'Keeps sign-ins and security changes, with time, country and address, for every Beebeeb app. Never which files or items you open.' : undefined}>
        <Row label="Record sign-in activity" right={<Switch label="Record sign-in activity" on={on} onChange={(v) => void toggle(v)} />} />
      </Group>
      {!on ? (
        <div className="rounded-[12px] border border-line bg-paper px-4 py-3" data-testid="logging-off">
          <h2 className="text-[13px] font-semibold text-ink">Security logging is off</h2>
          <p className="text-[12px] text-ink-3">Nothing new is recorded. What was recorded is deleted within 30 days. Security alerts, like a new sign-in or a password change, still arrive.</p>
        </div>
      ) : groups.size === 0 ? (
        <div className="rounded-[12px] border border-line bg-paper px-4 py-3" data-testid="activity-empty">
          <p className="text-[13px] text-ink">No sign-in activity recorded yet.</p>
          <p className="text-[12px] text-ink-3">Sign-ins and security changes appear here once the server answers.</p>
        </div>
      ) : (
        [...groups.entries()].map(([title, events]) => (
          <Group key={title} title={title}>
            {events.map((e) => (
              <div key={e.id} className="flex min-h-[44px] flex-col justify-center px-4 py-2" data-testid="event-row">
                <div className="text-[13px] leading-5 text-ink">{e.what}</div>
                <div className="font-mono text-[11px] leading-4 text-ink-3">
                  {new Date(e.at).toLocaleString('en-GB', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit', timeZone: 'Europe/Amsterdam' })}
                  {' · '}{e.country ? e.country : 'Not reported'}{e.address ? ` · ${e.address}` : ''}
                </div>
              </div>
            ))}
          </Group>
        ))
      )}
      {on ? (
        <p className="px-1 text-[12px] leading-4 text-ink-3">
          Device names are what each device reports. Something you don’t recognise? <button type="button" onClick={() => { onPassword() }} className="text-ink underline">Change your password</button> and check <button type="button" onClick={() => { onDevices() }} className="text-ink underline">Devices and sessions</button>.
        </p>
      ) : null}
      {ask ? (
        <Dialog labelId="log-off-h" title="Turn off security logging?" onCancel={() => setAsk(false)}
          actions={<><Btn onClick={() => setAsk(false)}>Cancel</Btn><Btn variant="danger" disabled={!password} onClick={() => void turnOff()} aria-label="Turn off security logging, destructive">Turn off</Btn></>}>
          <p className="mb-2">Without the log you can’t see a sign-in you didn’t make.</p>
          <p className="mb-3">We will stop collecting sign-in and activity data immediately. All data already collected will be deleted within 30 days. Deletion is irreversible.</p>
          {/* Server #422 confirms this with a password step-up; an authenticator code is not a step-up (B10b note). */}
          <Field label="Password"><PasswordInput label="Password" value={password} onChange={setPassword} focus /></Field>
        </Dialog>
      ) : null}
    </Page>
  )
}
