// Owned by batch B10. Signed out, locked or without a plan, the window shows sign-in (DW-A02/A03), the lock (DW-0009)
// or no plan (DW-0014) instead of Files. Outside Tauri (the browser dev server) every check is unavailable and the
// window renders with the board's fixtures, as before. The dimmed sidebar behind a gate names no account (the window's
// own generic "Account", never the board's example person).
import { type ReactNode, useCallback, useEffect, useRef, useState } from 'react'
import { BBButton, Icon } from '@beebeeb/shared/design'
import { clearSession, command, openUrl } from '../../desktopApi'
import { CaptionBarView } from '../chrome/CaptionBar'
import { Toolbar } from '../chrome/Toolbar'
import { Sidebar } from '../chrome/Sidebar'
import { AccountEntry } from './create/entry'
import { restoreWithPhrase, unlockWithWindowsHello } from './api'
import { HelloPrompt, SECRET_FIELD } from './parts'
import { usePrivateScreen } from './privateScreen'
import { hasNoPlan } from './plan'

type GateState = 'checking' | 'signed-out' | 'locked' | 'no-plan' | 'unavailable' | 'ready'

export const PLANS_URL = 'https://beebeeb.io/pricing'

async function readGate(): Promise<GateState> {
  const status = await command<{ logged_in: boolean; vault_unlocked?: boolean }>('sync_status')
  // A real host that answers nothing is not signed out and not ready: the window says it cannot check the
  // session and offers Retry. The browser dev server has no host at all (`unsupported`), so it keeps the
  // board's fixtures, as before.
  if (!status.ok) return status.unsupported ? 'ready' : 'unavailable'
  if (typeof status.value?.logged_in !== 'boolean') return 'unavailable'
  if (!status.value.logged_in) return 'signed-out'
  if (status.value.vault_unlocked === false) return 'locked'
  const sub = await command<{ plan_state?: string | null; plan?: string; status?: string; quota_bytes?: number }>('account_subscription')
  return sub.ok && hasNoPlan(sub.value) ? 'no-plan' : 'ready'
}

export function AuthGate({ children }: { children: ReactNode }) {
  const [state, setState] = useState<GateState>('checking')
  // A sign-out from the no-plan screen that happened but could not confirm one step (FB-24): its sentence shows until OK.
  // It is held here, not in that screen, because the next check (a focus brings one) unmounts the screen once the session
  // is gone.
  const [signedOutWarning, setSignedOutWarning] = useState<string | null>(null)
  const check = useCallback(() => { void readGate().then(setState) }, [])
  useEffect(() => {
    check()
    // "Check again" also runs when Beebeeb comes back to the front (DW-0014); a lock or sign-out elsewhere shows too.
    window.addEventListener('focus', check)
    return () => window.removeEventListener('focus', check)
  }, [check])
  if (signedOutWarning) return <SignedOutWarning sentence={signedOutWarning} onOk={() => { setSignedOutWarning(null); check() }} />
  if (state === 'checking') return <div className="h-screen bg-paper" aria-busy="true" />
  if (state === 'unavailable') return <CheckFailedView onRetry={check} />
  if (state === 'signed-out') return <AccountEntry startAt="sign-in" onSignedIn={check} onCreated={check} />
  if (state === 'locked') return <LockedWindow onUnlocked={check} />
  if (state === 'no-plan') return <NoPlanWindow onCheckAgain={check} onSignedOut={(warning) => { setSignedOutWarning(warning); check() }} />
  return <>{children}</>
}

/** The window behind a gate: caption bar, the sidebar dimmed (section names only, never a file name), the toolbar dimmed. */
function GateWindow({ children, sidebarFooter, overlay, dimSidebar = true }: { children: ReactNode; sidebarFooter?: ReactNode; overlay?: ReactNode; dimSidebar?: boolean }) {
  return (
    <div className="relative flex h-screen flex-col overflow-hidden bg-paper font-sans text-ink">
      <CaptionBarView title="Beebeeb" />
      <div className="flex min-h-0 flex-1">
        <div className={`relative flex ${dimSidebar ? 'pointer-events-none opacity-45' : ''}`} aria-hidden={dimSidebar || undefined} inert={dimSidebar || undefined}>
          <Sidebar active="files" onGo={() => {}} storage={null} accountName="Account" />
          {sidebarFooter}
        </div>
        <main className="flex min-w-0 flex-1 flex-col">
          <div className="pointer-events-none opacity-45" aria-hidden inert><Toolbar /></div>
          <div className="grid flex-1 place-items-center p-8">{children}</div>
        </main>
      </div>
      {overlay}
    </div>
  )
}

export type LockedMode = 'hello' | 'phrase'

/** DW-0009: app lock. Windows Hello starts by itself; Cancel leaves the window locked and Unlock starts it again.
 *  When Hello is not set up, the one next action is the recovery phrase. */
export function LockedView({ prompt, error, mode = 'hello', onUnlock, onPhrase }: {
  prompt: boolean
  error?: string
  mode?: LockedMode
  onUnlock: () => void
  onPhrase?: () => void
}) {
  usePrivateScreen()
  return (
    <GateWindow overlay={prompt ? <HelloPrompt reason="Unlock Beebeeb" /> : undefined}>
      <section className="flex max-w-[320px] flex-col items-center text-center" aria-labelledby="locked-heading" data-locked>
        <div className="mb-4 grid h-14 w-14 place-items-center rounded-full bg-amber-bg text-amber-deep"><Icon name="lock" size={26} /></div>
        <h1 id="locked-heading" className="m-0 text-[18px] font-semibold">Beebeeb is locked</h1>
        <p className="mt-1 mb-5 text-[13px] text-ink-2">Unlock to see your files.</p>
        {error && <p role="alert" className="-mt-2 mb-4 text-[12px] text-ink-2">{error}</p>}
        {mode === 'phrase'
          ? <BBButton variant="amber" onClick={onPhrase ?? onUnlock}><span className="text-[13px] font-medium">Unlock with the recovery phrase</span></BBButton>
          : <BBButton variant="amber" onClick={onUnlock}><span className="text-[13px] font-medium">Unlock with Windows Hello</span></BBButton>}
      </section>
    </GateWindow>
  )
}

function LockedWindow({ onUnlocked }: { onUnlocked: () => void }) {
  const [prompt, setPrompt] = useState(false)
  const [error, setError] = useState<string>()
  const [mode, setMode] = useState<LockedMode>('hello')
  const [phraseOpen, setPhraseOpen] = useState(false)
  const unlock = useCallback(async () => {
    setPrompt(true)
    setError(undefined)
    const result = await unlockWithWindowsHello()
    setPrompt(false)
    if (result.ok) onUnlocked()
    else if (result.reason === 'hello_unavailable') {
      setMode('phrase')
      setError('Windows Hello isn’t set up on this PC. Unlock with your recovery phrase instead.')
    } else if (result.reason === 'hello_failed') setError('Windows Hello didn’t recognise you. Nothing was unlocked.')
    else setError('Windows Hello didn’t unlock Beebeeb. Nothing was unlocked.')
  }, [onUnlocked])
  // Windows Hello starts by itself when the window opens.
  useEffect(() => { void unlock() }, [unlock])
  if (phraseOpen) return <PhraseUnlockView onUnlocked={onUnlocked} />
  return <LockedView prompt={prompt} error={error} mode={mode} onUnlock={() => void unlock()} onPhrase={() => setPhraseOpen(true)} />
}

/** The fallback when Windows Hello is not set up: the 12 words, one next action, no code rendered raw. */
function PhraseUnlockView({ onUnlocked }: { onUnlocked: () => void }) {
  const [phrase, setPhrase] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string>()
  const words = phrase.trim().split(/\s+/).filter(Boolean)
  const unlock = useCallback(async () => {
    setBusy(true)
    setError(undefined)
    const result = await restoreWithPhrase(words.join(' '))
    setBusy(false)
    if (result.ok) { setPhrase(''); onUnlocked(); return }
    setError(/too many/i.test(result.reason) ? 'Too many tries. Wait an hour.' : 'That phrase doesn’t open this account. Nothing was unlocked.')
  }, [words, onUnlocked])
  return (
    <GateWindow>
      <section className="flex max-w-[320px] flex-col items-center text-center" aria-labelledby="phrase-unlock-heading" data-phrase-unlock>
        <div className="mb-4 grid h-14 w-14 place-items-center rounded-full bg-amber-bg text-amber-deep"><Icon name="lock" size={26} /></div>
        <h1 id="phrase-unlock-heading" className="m-0 text-[18px] font-semibold">Unlock with your recovery phrase</h1>
        <p className="mt-1.5 mb-4 text-[13px] leading-[18px] text-ink-2">Type the 12 words you saved when you set up Beebeeb. They never leave this PC.</p>
        {error && <p role="alert" className="-mt-2 mb-4 text-[12px] text-ink-2">{error}</p>}
        <form className="flex w-full flex-col items-center" onSubmit={(e) => { e.preventDefault(); if (words.length === 12 && !busy) void unlock() }}>
          <label htmlFor="recovery-phrase" className="mb-1 block self-start text-[12px] font-medium text-ink-2">Recovery phrase</label>
          <input
            {...SECRET_FIELD}
            id="recovery-phrase"
            type="password"
            value={phrase}
            onChange={(e) => setPhrase(e.target.value)}
            className="mb-4 h-8 w-full rounded-md border border-line bg-paper px-2.5 font-mono text-[13px] text-ink outline-none focus:ring-2 focus:ring-amber-deep"
          />
          <BBButton type="submit" variant="amber" disabled={words.length !== 12 || busy}>
            <span className="text-[13px] font-medium">{busy ? 'Unlocking…' : 'Unlock with the recovery phrase'}</span>
          </BBButton>
        </form>
      </section>
    </GateWindow>
  )
}

/** DW-0014: signed in, no plan yet. Plans are only on the website; nothing syncs until there is one. */
export function NoPlanView({ checking, error, onChoose, onCheckAgain, onSignOut }: { checking?: boolean; error?: string | null; onChoose: () => void; onCheckAgain: () => void; onSignOut: () => void }) {
  const footer = (
    <div className="absolute inset-x-2 bottom-12 bg-paper-2 px-2.5 pb-3" data-no-plan-meter>
      <div className="mb-1 text-[12px] font-medium text-ink-2">No plan</div>
      <div className="h-1.5 rounded-full bg-paper-3" role="meter" aria-label="Storage used" aria-valuenow={0} aria-valuemin={0} aria-valuemax={100} />
      <a href={PLANS_URL} onClick={(e) => { e.preventDefault(); onChoose() }} className="mt-1.5 block text-[11px] text-ink-3 underline underline-offset-2">Choose a plan on beebeeb.io</a>
    </div>
  )
  return (
    <GateWindow sidebarFooter={footer} dimSidebar={false}>
      <section className="flex max-w-[380px] flex-col items-center text-center" aria-labelledby="no-plan-heading" data-no-plan>
        <h1 id="no-plan-heading" className="m-0 text-[18px] font-semibold">Choose a plan</h1>
        <p className="mt-1.5 mb-5 text-[13px] leading-[18px] text-ink-2">
          Your account is ready, but it has no plan yet. Choose a plan on beebeeb.io, including the free trial. Nothing syncs until a plan is chosen. Beebeeb continues here by itself.
        </p>
        {error && <p role="alert" className="-mt-2 mb-4 text-[12px] text-ink-2">{error}</p>}
        <div className="flex flex-wrap justify-center gap-2">
          <a
            href={PLANS_URL}
            onClick={(e) => { e.preventDefault(); onChoose() }}
            className="inline-flex h-8 items-center justify-center rounded-md bg-amber px-3 text-[13px] font-medium text-amber-ink hover:brightness-95 focus-visible:ring-2 focus-visible:ring-amber-deep focus-visible:ring-offset-2"
          >
            Choose a plan on beebeeb.io
          </a>
          <BBButton onClick={onCheckAgain} disabled={checking}><span className="text-[13px] font-medium">{checking ? 'Checking…' : 'Check again'}</span></BBButton>
          <BBButton variant="ghost" onClick={onSignOut}><span className="text-[13px] font-medium">Sign out</span></BBButton>
        </div>
      </section>
    </GateWindow>
  )
}

function NoPlanWindow({ onCheckAgain, onSignedOut }: { onCheckAgain: () => void; onSignedOut: (warning: string | null) => void }) {
  const [checking, setChecking] = useState(false)
  // eslint-disable-next-line beebeeb/no-ad-hoc-error-surface -- the gate has no toast host: the screen stays and says why the sign-out did not happen
  const [failed, setFailed] = useState<string | null>(null)
  // Only through desktopApi's wrapper, which parses the outcome; a step it could not confirm (FB-24,
  // tests/sessionActionOutcome.test.ts) goes up with the sign-out, and AuthGate says it.
  // One sign-out at a time: a double click sends one clear_session (#270 follow-up).
  const busy = useRef(false)
  const signOut = async () => {
    if (busy.current) return
    busy.current = true
    setFailed(null)
    try {
      const r = await clearSession()
      if (r.ok) onSignedOut(r.value.warning?.sentence ?? null)
      else setFailed(r.reason)
    } finally { busy.current = false }
  }
  return (
    <NoPlanView
      checking={checking}
      error={failed}
      onChoose={() => void openUrl(PLANS_URL)}
      onCheckAgain={() => { setChecking(true); onCheckAgain(); window.setTimeout(() => setChecking(false), 800) }}
      onSignOut={() => void signOut()}
    />
  )
}

/** The host answered nothing: the window says so and offers Retry. Never Files, never the raw reason. */
export function CheckFailedView({ onRetry }: { onRetry: () => void }) {
  return (
    <GateWindow>
      <section className="flex max-w-[380px] flex-col items-center text-center" aria-labelledby="check-failed-heading" data-check-failed>
        <div className="mb-4 grid h-14 w-14 place-items-center rounded-full bg-paper-2 text-ink-2"><Icon name="info" size={26} /></div>
        <h1 id="check-failed-heading" className="m-0 text-[18px] font-semibold">Beebeeb can’t check the session</h1>
        <p className="mt-1.5 mb-5 text-[13px] leading-[18px] text-ink-2">This PC didn’t answer. Nothing was opened or changed.</p>
        <BBButton variant="amber" onClick={onRetry}><span className="text-[13px] font-medium">Retry</span></BBButton>
      </section>
    </GateWindow>
  )
}

/** Signed out, but one step could not be confirmed (FB-24): the sentence, neutrally, then the sign-in after OK. */
export function SignedOutWarning({ sentence, onOk }: { sentence: string; onOk: () => void }) {
  return (
    <GateWindow>
      <section className="flex max-w-[380px] flex-col items-center text-center" aria-labelledby="signed-out-heading" data-signed-out-warning>
        <h1 id="signed-out-heading" className="m-0 text-[18px] font-semibold">Signed out</h1>
        <p role="status" className="mt-1.5 mb-5 text-[13px] leading-[18px] text-ink-2">{sentence}</p>
        <BBButton onClick={onOk}><span className="text-[13px] font-medium">OK</span></BBButton>
      </section>
    </GateWindow>
  )
}
