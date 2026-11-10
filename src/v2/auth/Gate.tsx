// Owned by batch B10. Signed out, locked or without a plan, the window shows sign-in (DW-A02/A03), the lock (DW-0009)
// or no plan (DW-0014) instead of Files. Outside Tauri (the browser dev server) every check is unavailable and the
// window renders with the board's fixtures, as before. The dimmed sidebar behind a gate names no account (the window's
// own generic "Account", never the board's example person).
import { type ReactNode, useCallback, useEffect, useRef, useState } from 'react'
import { BBButton, Icon } from '@beebeeb/shared/design'
import { clearSession, command } from '../../desktopApi'
import { CaptionBarView } from '../chrome/CaptionBar'
import { Toolbar } from '../chrome/Toolbar'
import { Sidebar } from '../chrome/Sidebar'
import { AccountEntry } from './create/entry'
import { SignInFlow } from './index'
import { unlockWithWindowsHello } from './api'
import { HelloPrompt } from './parts'
import { usePrivateScreen } from './privateScreen'
import { hasNoPlan } from './plan'

type GateState = 'checking' | 'signed-out' | 'locked' | 'needs-phrase' | 'no-plan' | 'ready'

export const PLANS_URL = 'https://beebeeb.io/pricing'

async function readGate(): Promise<GateState> {
  const status = await command<{ logged_in: boolean; vault_unlocked?: boolean }>('sync_status')
  if (!status.ok) return 'ready'
  if (!status.value.logged_in) return 'signed-out'
  if (status.value.vault_unlocked === false) {
    const key = await command<boolean>('auth_vault_key_present')
    // No key on this PC is the 12 words. A key, or a store that did not answer, stays on the lock.
    if (key.ok && key.value === false) return 'needs-phrase'
    return 'locked'
  }
  const sub = await command<{ plan_state?: string | null; plan?: string; status?: string; quota_bytes?: number }>('account_subscription')
  return sub.ok && hasNoPlan(sub.value) ? 'no-plan' : 'ready'
}

export function AuthGate({ children }: { children: ReactNode }) {
  const [state, setState] = useState<GateState>('checking')
  // A sign-out from the no-plan screen that happened but could not confirm one step (FB-24): its sentence shows until OK.
  // It is held here, not in that screen, because the next check (a focus brings one) unmounts the screen once the session
  // is gone.
  const [signedOutWarning, setSignedOutWarning] = useState<string | null>(null)
  // leaveFlow is set only when this sign-in flow finished. A focus event is not that: password sign-in
  // stores the session before the 12-word step paints, and a focus check would replace that step with the lock.
  const check = useCallback((leaveFlow = false) => {
    void readGate().then((next) => {
      setState((current) => {
        if (!leaveFlow && current === 'signed-out' && (next === 'locked' || next === 'needs-phrase')) return current
        return next
      })
    })
  }, [])
  useEffect(() => {
    check(false)
    const onFocus = () => check(false)
    // "Check again" also runs when Beebeeb comes back to the front (DW-0014); a lock or sign-out elsewhere shows too.
    window.addEventListener('focus', onFocus)
    return () => window.removeEventListener('focus', onFocus)
  }, [check])
  if (signedOutWarning) {
    return (
      <SignedOutWarning
        sentence={signedOutWarning}
        onOk={() => { setSignedOutWarning(null); check(true) }}
      />
    )
  }
  if (state === 'checking') return <div className="h-screen bg-paper" aria-busy="true" />
  if (state === 'signed-out') return <AccountEntry onSignedIn={() => check(true)} onCreated={() => check(true)} />
  if (state === 'locked') {
    return <LockedWindow onUnlocked={() => check(true)} onUsePhrase={() => setState('needs-phrase')} />
  }
  if (state === 'needs-phrase') return <SignInFlow startAt="phrase" onSignedIn={() => check(true)} />
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

/** DW-0009: app lock. Windows Hello starts by itself; Cancel leaves the window locked and Unlock starts it again. */
export function LockedView({ prompt, error, onUnlock, onUsePhrase }: {
  prompt: boolean
  error?: string
  onUnlock: () => void
  onUsePhrase: () => void
}) {
  usePrivateScreen()
  return (
    <GateWindow overlay={prompt ? <HelloPrompt reason="Unlock Beebeeb" /> : undefined}>
      <section className="flex max-w-[320px] flex-col items-center text-center" aria-labelledby="locked-heading" data-locked>
        <div className="mb-4 grid h-14 w-14 place-items-center rounded-full bg-amber-bg text-amber-deep"><Icon name="lock" size={26} /></div>
        <h1 id="locked-heading" className="m-0 text-[18px] font-semibold">Beebeeb is locked</h1>
        <p className="mt-1 mb-5 text-[13px] text-ink-2">Unlock to see your files.</p>
        {error && <p role="alert" className="-mt-2 mb-4 text-[12px] text-ink-2">{error}</p>}
        <BBButton variant="amber" onClick={onUnlock}><span className="text-[13px] font-medium">Unlock</span></BBButton>
        <BBButton variant="ghost" onClick={onUsePhrase}>
          <span className="text-[13px] font-medium">Use your recovery phrase</span>
        </BBButton>
      </section>
    </GateWindow>
  )
}

function LockedWindow({ onUnlocked, onUsePhrase }: { onUnlocked: () => void; onUsePhrase: () => void }) {
  const [prompt, setPrompt] = useState(false)
  const [error, setError] = useState<string>()
  const unlock = useCallback(async () => {
    setPrompt(true)
    setError(undefined)
    const result = await unlockWithWindowsHello()
    setPrompt(false)
    if (result.ok) onUnlocked()
    else if (result.reason === 'hello_unavailable') setError('Windows Hello isn’t set up on this PC. Set it up in Windows Settings, or sign out and sign in again.')
    else if (result.reason === 'hello_failed') setError('Windows Hello didn’t recognise you. Nothing was unlocked.')
  }, [onUnlocked])
  // Windows Hello starts by itself when the window opens.
  useEffect(() => { void unlock() }, [unlock])
  return <LockedView prompt={prompt} error={error} onUnlock={() => void unlock()} onUsePhrase={onUsePhrase} />
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
          Your account is ready. Choose a plan on beebeeb.io, including the free trial. Beebeeb continues here by itself.
        </p>
        {error && <p role="alert" className="-mt-2 mb-4 text-[12px] text-ink-2">{error}</p>}
        <div className="flex flex-wrap justify-center gap-2">
          <BBButton variant="amber" onClick={onChoose}><span className="text-[13px] font-medium">Choose a plan on beebeeb.io</span></BBButton>
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
      onChoose={() => void command('plugin:opener|open_url', { url: PLANS_URL })}
      onCheckAgain={() => { setChecking(true); onCheckAgain(); window.setTimeout(() => setChecking(false), 800) }}
      onSignOut={() => void signOut()}
    />
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
