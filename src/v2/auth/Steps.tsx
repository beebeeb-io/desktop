// DW-A01 (first run), A23 to A25 (the 12 words on a new device, wrong phrase, the honest dead end) and A26 to A28
// (two-factor: authenticator code, wrong code, backup code).
import { useRef } from 'react'
import { Icon } from '@beebeeb/shared/design'
import { digitsOnly, normalizeWord, spreadPaste } from './model'
import { AuthWindow, Field, Heading, Note, PrimaryButton, SECRET_FIELD, SecondaryButton, StepLockup, TextLink } from './parts'

export function FirstRunView({ onCreate, onSignIn }: { onCreate: () => void; onSignIn: () => void }) {
  return (
    <AuthWindow>
      <Heading sub="Sign in to your Beebeeb account, or create one. It works in every Beebeeb app.">Welcome to Beebeeb Drive</Heading>
      <div className="flex flex-col gap-2.5">
        <PrimaryButton onClick={onCreate}>Create a Beebeeb account</PrimaryButton>
        <SecondaryButton onClick={onSignIn}>Sign in</SecondaryButton>
      </div>
    </AuthWindow>
  )
}

// ── Two-factor ───────────────────────────────────────────────────────────────

export function TwoFactorView({ mode, code, state, onCode, onSubmit, onSwitch, onBack }: {
  mode: 'totp' | 'backup'
  code: string
  state: 'idle' | 'verifying' | 'wrong'
  onCode: (v: string) => void
  /** Gets the code itself: the sixth digit's change submits before the parent has re-rendered with it (DW-A26). */
  onSubmit: (code: string) => void
  onSwitch: () => void
  onBack: () => void
}) {
  const inputRef = useRef<HTMLInputElement>(null)
  if (mode === 'backup') {
    return (
      <AuthWindow>
        <StepLockup /><div className="mt-4" />
        <Heading sub="Use one of the backup codes you saved when you turned on two-factor. Each works once.">Use a backup code</Heading>
        {state === 'wrong' && <Note tone="red" title="That backup code isn’t right">Nothing was signed in.</Note>}
        <form onSubmit={(e) => { e.preventDefault(); if (code.length === 8 && state !== 'verifying') onSubmit(code) }} noValidate>
          <Field label="Backup code" mono {...SECRET_FIELD} inputMode="numeric" maxLength={8} placeholder="12345678" value={code}
            disabled={state === 'verifying'} onChange={(e) => onCode(digitsOnly(e.target.value, 8))} />
          <PrimaryButton type="submit" disabled={code.length !== 8 || state === 'verifying'}>{state === 'verifying' ? 'Verifying…' : 'Verify backup code'}</PrimaryButton>
        </form>
        <div className="mt-4 text-center"><TextLink onClick={onSwitch}>Use the authenticator code</TextLink></div>
      </AuthWindow>
    )
  }
  const boxes = Array.from({ length: 6 }, (_, i) => code[i] ?? '')
  return (
    <AuthWindow>
      <StepLockup /><div className="mt-4" />
      <Heading sub="Enter the 6-digit code from your authenticator app.">Two-factor code</Heading>
      {state === 'wrong' && <Note tone="red" title="That code isn’t right">Nothing was signed in. Check that this PC’s clock is right: codes change every 30 seconds.</Note>}
      <label htmlFor="totp" className="sr-only">Authenticator code</label>
      {/* One real field; the six boxes are its display (DW-A26). */}
      <div className="relative mb-3" onClick={() => inputRef.current?.focus()} role="presentation">
        <div className={`flex gap-2 ${state === 'verifying' ? 'opacity-50' : ''}`} aria-hidden data-digit-boxes>
          {boxes.map((d, i) => (
            <span key={i} data-digit className={`grid h-12 w-11 place-items-center rounded-md border font-mono text-[20px] ${
              state === 'wrong' ? 'border-red text-red' : i === code.length ? 'border-ink-2' : 'border-line'} bg-paper`}>
              {d}
            </span>
          ))}
        </div>
        <input
          id="totp" ref={inputRef} {...SECRET_FIELD} type="text" inputMode="numeric" maxLength={6} value={code}
          disabled={state === 'verifying'}
          onChange={(e) => { const v = digitsOnly(e.target.value, 6); onCode(v); if (v.length === 6) onSubmit(v) }}
          onKeyDown={(e) => { if (e.key === 'Enter' && code.length === 6) onSubmit(code) }}
          className="absolute inset-0 opacity-0"
        />
      </div>
      <p className="mb-5 text-[12px] text-ink-3">{state === 'verifying' ? 'Verifying…' : 'The code is checked after the sixth digit.'}</p>
      <div className="flex items-center justify-between">
        <TextLink onClick={onBack}>Back</TextLink>
        <TextLink onClick={onSwitch}>Use a backup code</TextLink>
      </div>
    </AuthWindow>
  )
}

// ── The 12 words ─────────────────────────────────────────────────────────────

export function PhraseView({ words, focus, unknown, state, onWords, onRestore, onApproveInstead, onLost }: {
  words: string[]
  focus?: number
  /** Positions of words not in the word list (marked only after a failed restore, DW-A24). */
  unknown: number[]
  state: 'idle' | 'restoring' | 'wrong' | 'too-many'
  onWords: (w: string[]) => void
  onRestore: () => void
  onApproveInstead: () => void
  onLost: () => void
}) {
  const filled = words.filter(Boolean).length === 12
  return (
    <AuthWindow>
      <Heading sub="This PC doesn’t have your keys yet. Type your 12-word recovery phrase to restore them here. It never leaves this device.">
        Set up this device
      </Heading>
      {state === 'wrong' && <Note tone="red" title="That phrase doesn’t open this account">Nothing was restored.</Note>}
      {state === 'too-many' && <Note tone="red">Too many tries. Wait an hour.</Note>}
      <div className="grid grid-cols-3 gap-2" data-phrase-grid>
        {words.map((w, i) => (
          <label key={i} className={`flex h-9 items-center gap-1.5 rounded-md border bg-paper px-2 font-mono text-[12px] text-ink ${unknown.includes(i) ? 'border-red' : 'border-line'} focus-within:ring-2 focus-within:ring-amber-deep`} data-word>
            <span className="w-4 flex-none text-right font-mono text-[11px] text-ink-4">{i + 1}</span>
            <input
              {...SECRET_FIELD}
              aria-label={`Word ${i + 1}`}
              aria-invalid={unknown.includes(i) || undefined}
              // eslint-disable-next-line jsx-a11y/no-autofocus
              autoFocus={focus === i}
              value={w}
              disabled={state === 'restoring'}
              onChange={(e) => { const next = [...words]; next[i] = normalizeWord(e.target.value); onWords(next) }}
              onPaste={(e) => { e.preventDefault(); onWords(spreadPaste(words, i, e.clipboardData.getData('text'))) }}
              className="min-w-0 flex-1 border-0 bg-transparent outline-none"
            />
          </label>
        ))}
      </div>
      <p className="mt-3 text-[12px] leading-4 text-ink-3">
        Paste all 12 words into any box to fill them. Or <TextLink onClick={onApproveInstead}>approve from another device</TextLink> instead.
      </p>
      <div className="mt-4 flex flex-col gap-2.5">
        <PrimaryButton onClick={onRestore} disabled={!filled || state === 'restoring' || state === 'too-many'}>
          {state === 'restoring' ? 'Restoring keys…' : 'Restore keys'}
        </PrimaryButton>
        <div className="text-center"><TextLink onClick={onLost}>I’ve lost my recovery phrase</TextLink></div>
      </div>
    </AuthWindow>
  )
}

export function LostPhraseView({ onFound, onCancelBilling, onErase, onDelete }: {
  onFound: () => void; onCancelBilling: () => void; onErase: () => void; onDelete: () => void
}) {
  const exits = [
    { label: 'Cancel billing', detail: 'Stop paying. Your encrypted files stay until the plan ends.', on: onCancelBilling, href: 'https://app.beebeeb.io/billing' },
    { label: 'Erase all files', detail: 'Removes every file from Beebeeb 72 hours after you ask.', on: onErase, href: 'https://app.beebeeb.io/settings' },
    { label: 'Delete the account', detail: 'Removes the account and everything in it.', on: onDelete, href: 'https://app.beebeeb.io/settings' },
  ]
  return (
    <AuthWindow>
      <Heading sub="Your files are encrypted with keys only your recovery phrase or a signed-in device can restore. Without either, nobody can open them: not you, not us.">
        Without the phrase, your data can’t be restored
      </Heading>
      <ul className="m-0 mb-4 flex list-none flex-col gap-2 p-0">
        {exits.map((x) => (
          <li key={x.label} data-exit className="rounded-md border border-line px-3 py-2">
            <a href={x.href} onClick={(e) => { e.preventDefault(); x.on() }} className="text-[13px] font-medium text-ink underline underline-offset-2">{x.label}</a>
            <div className="text-[12px] text-ink-3">{x.detail}</div>
          </li>
        ))}
      </ul>
      <p className="mb-4 flex gap-2 text-[12px] leading-4 text-ink-2">
        <Icon name="mail" size={14} className="mt-0.5 flex-none text-ink-3" />
        <span>Questions: <span className="font-mono">support@beebeeb.io</span>. Never send anyone your recovery phrase, not even us.</span>
      </p>
      <SecondaryButton onClick={onFound}>I found my phrase</SecondaryButton>
    </AuthWindow>
  )
}
