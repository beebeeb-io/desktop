// Owned by batch B11: first run on Windows (DW-0201 to DW-0209, 960 x 640). Mounted for ?window=onboarding&platform=windows
// once ./enabled.ts says so. The backup step reuses src/windows/KnownFolderOnboarding.tsx's logic: the same commands,
// Desktop, Documents and Pictures on by default, the prompt marked seen either way.
import { useCallback, useEffect, useState } from 'react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { command, getKnownFolderBackup, markKnownFolderOnboardingSeen, setKnownFolderBackup, type KnownFolderStatus } from '../../desktopApi'
import { loadFinderSetup, type FinderSetupView } from '../../finderSetup'
import {
  BackupStep, CompareView, DoneStep, ExplorerStep, LockStep, LoginStep, NotificationsStep, PasskeyStep, PhotosStep, SetupWindow,
  type BackupFolder, type ExplorerStatus, type LockAfter, type PreviewSample, type SetupStep, type SetupSummary,
} from './views'
import { AccountEntry } from '../auth/create/entry'
import { seed } from '../seed'

export const DEFAULT_ON = new Set(['desktop', 'documents', 'pictures'])

/** The six known folders in the Windows app's order (src-tauri/src/known_folder.rs). */
export function toBackupFolders(list: KnownFolderStatus[], chosen: Set<string>): BackupFolder[] {
  return list.map((f) => ({
    key: f.key,
    name: f.display_name,
    detail: f.item_count == null || f.item_count <= 0 ? '' : `${f.item_count.toLocaleString('en-US')}${f.item_count >= 10000 ? '+' : ''} ${f.item_count === 1 ? 'file' : 'files'}`,
    on: chosen.has(f.key),
  }))
}

export function explorerStatus(v: FinderSetupView | null): { status: ExplorerStatus; code: string | null } {
  if (!v) return { status: 'adding', code: null }
  if (v.setup === 'ready') return { status: 'ready', code: null }
  if (v.setup === 'failed') {
    const c = v.last_failure?.code
    return { status: 'failed', code: c == null ? null : `0x${(c >>> 0).toString(16).toUpperCase().padStart(8, '0')}` }
  }
  return { status: 'adding', code: null }
}

const SAMPLE: PreviewSample = { label: '4K HEIC', size: '1.0 MB', original: '30.9 MB', photos: 0, local: '0 GB', cloud: '0 GB', space: '20 GB' }

async function signedIn(): Promise<boolean> {
  const r = await command<{ logged_in: boolean; vault_unlocked?: boolean }>('sync_status')
  return r.ok && r.value.logged_in && r.value.vault_unlocked !== false
}

/** The first-run window: the account entry (B10 + B11) until this PC is signed in, then the setup steps. */
export default function OnboardingApp() {
  const [account, setAccount] = useState<'checking' | 'out' | 'in'>('checking')
  useEffect(() => { void signedIn().then((v) => setAccount(v ? 'in' : 'out')) }, [])
  if (account === 'checking') return <div className="h-screen bg-paper" aria-busy="true" />
  if (account === 'out') return <AccountEntry onSignedIn={() => setAccount('in')} onCreated={() => setAccount('in')} />
  return <SetupFlow />
}

export function SetupFlow() {
  const [step, setStep] = useState<SetupStep | 'done' | 'compare'>(() => seed('setup.step', 'passkey'))
  const [summary, setSummary] = useState<SetupSummary>({ passkey: 'Not now', lock: 'Off', backup: 'Off', previews: '4K HEIC', explorer: 'Adding', notifications: 'On', login: 'On' })
  const [passkey, setPasskey] = useState({ asking: false, password: '', error: null as string | null })
  const [lock, setLock] = useState<{ on: boolean; after: LockAfter }>({ on: true, after: 'Immediately' })
  const [known, setKnown] = useState<KnownFolderStatus[]>([])
  const [chosen, setChosen] = useState<Set<string>>(new Set(DEFAULT_ON))
  const [backup, setBackup] = useState({ busy: false, error: null as string | null })
  const [preview, setPreview] = useState({ size: '4K', format: 'HEIC' })
  const [compareMode, setCompareMode] = useState<'Stacked' | 'Wipe' | 'Hold to flip'>('Stacked')
  const [finder, setFinder] = useState<FinderSetupView | null>(null)
  const [login, setLogin] = useState(true)
  const note = (k: keyof SetupSummary, v: string) => setSummary((s) => ({ ...s, [k]: v }))

  const readFinder = useCallback(() => { void loadFinderSetup().then((r) => setFinder(r.ok ? r.value : null)) }, [])
  useEffect(() => {
    readFinder()
    void getKnownFolderBackup().then((r) => { if (r.ok) setKnown(r.value) })
    void command<boolean>('autostart_enabled').then((r) => { if (r.ok) setLogin(r.value) })
    const t = window.setInterval(readFinder, 2000)
    return () => { window.clearInterval(t); setPasskey({ asking: false, password: '', error: null }) }
  }, [readFinder])

  const explorer = explorerStatus(finder)
  const skip = () => setStep('done')

  const addPasskey = async () => {
    if (!passkey.asking) { setPasskey((p) => ({ ...p, asking: true })); return }
    const name = await command<string>('auth_signup_default_device_name')
    const r = await command<{ id: string }>('account_passkeys_add', { name: name.ok ? name.value : 'This PC', password: passkey.password })
    setPasskey({ asking: r.ok ? false : true, password: '', error: r.ok ? null : 'That didn’t work. Check your password and try again, or choose Not now.' })
    if (r.ok) { note('passkey', 'Added'); setStep('lock') }
  }

  const turnOnBackup = async () => {
    setBackup({ busy: true, error: null })
    for (const key of chosen) {
      const r = await setKnownFolderBackup(key, true)
      if (!r.ok) { setBackup({ busy: false, error: 'Backup setup failed. Try again, or choose Not now.' }); return }
    }
    await markKnownFolderOnboardingSeen()
    setBackup({ busy: false, error: null })
    note('backup', known.filter((f) => chosen.has(f.key)).map((f) => f.display_name).join(', ') || 'Off')
    setStep('photos')
  }

  const loginContinue = async () => {
    const current = await command<boolean>('autostart_enabled')
    if (current.ok && current.value !== login) await command('toggle_autostart')
    note('login', login ? 'On' : 'Off')
    note('explorer', explorer.status === 'ready' ? 'On' : explorer.status === 'failed' ? 'Off' : 'Adding')
    setStep('done')
  }

  const finish = async () => {
    // The main window opens; this setup window closes (DW-0208's rule). The guide runs there once (../guide).
    await command<void>('show_main_app_window')
    try { await getCurrentWindow().close() } catch { window.close() }
  }

  let body
  switch (step) {
    case 'passkey':
      body = <PasskeyStep {...passkey} onPassword={(v) => setPasskey((p) => ({ ...p, password: v, error: null }))} onAdd={() => void addPasskey()} onNotNow={() => setStep('lock')} />
      break
    case 'lock':
      body = <LockStep on={lock.on} after={lock.after} onToggle={(on) => setLock((l) => ({ ...l, on }))} onAfter={(after) => setLock((l) => ({ ...l, after }))}
        onContinue={() => { note('lock', lock.on ? 'On' : 'Off'); setStep('backup') }} onNotNow={() => { note('lock', 'Off'); setStep('backup') }} />
      break
    case 'backup':
      body = <BackupStep folders={toBackupFolders(known, chosen)} {...backup}
        onToggle={(k, on) => setChosen((c) => { const n = new Set(c); if (on) n.add(k); else n.delete(k); return n })}
        onAddFolder={() => void command('show_main_app_settings')} onTurnOn={() => void turnOnBackup()}
        onNotNow={() => { void markKnownFolderOnboardingSeen(); note('backup', 'Off'); setStep('photos') }} />
      break
    case 'photos':
      body = <PhotosStep sample={SAMPLE} size={preview.size} format={preview.format} onSize={(size) => setPreview((p) => ({ ...p, size }))} onFormat={(format) => setPreview((p) => ({ ...p, format }))}
        onCompare={() => setStep('compare')} onContinue={() => { note('previews', `${preview.size} ${preview.format}`); setStep('explorer') }} onNotNow={() => setStep('explorer')} />
      break
    case 'compare':
      body = <CompareView mode={compareMode} onMode={setCompareMode} onChoose={() => {}} onDone={() => setStep('photos')} />
      break
    case 'explorer':
      body = <ExplorerStep status={explorer.status} errorCode={explorer.code} onShow={() => void command('finder_setup_show_app')}
        onTryAgain={() => { void command('finder_setup_retry').then(readFinder) }} onContinue={() => setStep('notifications')} />
      break
    case 'notifications':
      body = <NotificationsStep onAllow={() => { note('notifications', 'On'); setStep('login') }} onNotNow={() => { note('notifications', 'Off'); setStep('login') }} />
      break
    case 'login':
      body = <LoginStep on={login} onToggle={setLogin} onContinue={() => void loginContinue()} />
      break
    case 'done':
      body = <DoneStep summary={summary} onRow={() => void command('show_main_app_settings')} onOpen={() => void finish()} />
      break
  }
  return <SetupWindow step={step} explorer={explorer.status} onSkip={step === 'done' || step === 'compare' ? undefined : skip}>{body}</SetupWindow>
}
