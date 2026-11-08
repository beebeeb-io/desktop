// Owned by batch B10. Signed out, locked or without a plan, the window shows sign-in (DW-A02/A03), the lock (DW-0009)
// or no plan (DW-0014) instead of Files. Outside Tauri (the browser dev server) every check is unavailable and the
// window renders with the fixtures, as before.
import { type ReactNode, useCallback, useEffect, useState } from 'react'
import { BBButton, Icon } from '@beebeeb/shared/design'
import { command } from '../../desktopApi'
import { CaptionBarView } from '../chrome/CaptionBar'
import { Toolbar } from '../chrome/FrameWindow'
import { Sidebar } from '../chrome/Sidebar'
import { SignInFlow } from './index'
import { unlockWithWindowsHello } from './api'
import { HelloPrompt } from './parts'

type GateState = 'checking' | 'signed-out' | 'locked' | 'no-plan' | 'ready'

export const PLANS_URL = 'https://beebeeb.io/pricing'

/** No plan yet: the server's subscription has no plan or no quota. */
export function hasNoPlan(sub: { plan?: string | null; status?: string | null; quota_bytes?: number | null } | null): boolean {
  if (!sub) return false
  const plan = (sub.plan ?? '').toLowerCase()
  return plan === '' || plan === 'none' || (sub.status ?? '').toLowerCase() === 'none' || (sub.quota_bytes ?? 1) === 0
}

async function readGate(): Promise<GateState> {
  const status = await command<{ logged_in: boolean; vault_unlocked?: boolean }>('sync_status')
  if (!status.ok) return 'ready'
  if (!status.value.logged_in) return 'signed-out'
  if (status.value.vault_unlocked === false) return 'locked'
  const sub = await command<{ plan?: string; status?: string; quota_bytes?: number }>('account_subscription')
  return sub.ok && hasNoPlan(sub.value) ? 'no-plan' : 'ready'
}

export function AuthGate({ children }: { children: ReactNode }) {
  const [state, setState] = useState<GateState>('checking')
  const check = useCallback(() => { void readGate().then(setState) }, [])
  useEffect(() => {
    check()
    // "Check again" also runs when Beebeeb comes back to the front (DW-0014); a lock or sign-out elsewhere shows too.
    window.addEventListener('focus', check)
    return () => window.removeEventListener('focus', check)
  }, [check])
  if (state === 'checking') return <div className="h-screen bg-paper" aria-busy="true" />
  if (state === 'signed-out') return <SignInFlow onSignedIn={check} />
  if (state === 'locked') return <LockedWindow onUnlocked={check} />
  if (state === 'no-plan') return <NoPlanWindow onCheckAgain={check} onSignedOut={check} />
  return <>{children}</>
}

/** The window behind a gate: caption bar, the sidebar dimmed (section names only, never a file name), the toolbar dimmed. */
function GateWindow({ children, sidebarFooter, overlay, dimSidebar = true }: { children: ReactNode; sidebarFooter?: ReactNode; overlay?: ReactNode; dimSidebar?: boolean }) {
  return (
    <div className="relative flex h-screen flex-col overflow-hidden bg-paper font-sans text-ink">
      <CaptionBarView title="Beebeeb" />
      <div className="flex min-h-0 flex-1">
        <div className={`relative flex ${dimSidebar ? 'pointer-events-none opacity-45' : ''}`} aria-hidden={dimSidebar || undefined} inert={dimSidebar || undefined}>
          <Sidebar active="files" onGo={() => {}} storage={null} accountName="Anna" />
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
export function LockedView({ prompt, error, onUnlock }: { prompt: boolean; error?: string; onUnlock: () => void }) {
  return (
    <GateWindow overlay={prompt ? <HelloPrompt reason="Unlock Beebeeb" /> : undefined}>
      <section className="flex max-w-[320px] flex-col items-center text-center" aria-labelledby="locked-heading" data-locked>
        <div className="mb-4 grid h-14 w-14 place-items-center rounded-full bg-amber-bg text-amber-deep"><Icon name="lock" size={26} /></div>
        <h1 id="locked-heading" className="m-0 text-[18px] font-semibold">Beebeeb is locked</h1>
        <p className="mt-1 mb-5 text-[13px] text-ink-2">Unlock to see your files.</p>
        {error && <p role="alert" className="-mt-2 mb-4 text-[12px] text-ink-2">{error}</p>}
        <BBButton variant="amber" onClick={onUnlock}>Unlock</BBButton>
      </section>
    </GateWindow>
  )
}

function LockedWindow({ onUnlocked }: { onUnlocked: () => void }) {
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
  return <LockedView prompt={prompt} error={error} onUnlock={() => void unlock()} />
}

/** DW-0014: signed in, no plan yet. Plans are only on the website; nothing syncs until there is one. */
export function NoPlanView({ checking, onChoose, onCheckAgain, onSignOut }: { checking?: boolean; onChoose: () => void; onCheckAgain: () => void; onSignOut: () => void }) {
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
        <div className="flex flex-wrap justify-center gap-2">
          <BBButton variant="amber" onClick={onChoose}>Choose a plan on beebeeb.io</BBButton>
          <BBButton onClick={onCheckAgain} disabled={checking}>{checking ? 'Checking…' : 'Check again'}</BBButton>
          <BBButton variant="ghost" onClick={onSignOut}>Sign out</BBButton>
        </div>
      </section>
    </GateWindow>
  )
}

function NoPlanWindow({ onCheckAgain, onSignedOut }: { onCheckAgain: () => void; onSignedOut: () => void }) {
  const [checking, setChecking] = useState(false)
  return (
    <NoPlanView
      checking={checking}
      onChoose={() => void command('plugin:opener|open_url', { url: PLANS_URL })}
      onCheckAgain={() => { setChecking(true); onCheckAgain(); window.setTimeout(() => setChecking(false), 800) }}
      onSignOut={() => void command('clear_session').then(onSignedOut)}
    />
  )
}
