// Owned by batch B11: first run on Windows (DW-0201 to DW-0209, 960 x 640). Mounted for ?window=onboarding&platform=windows
// once ./enabled.ts says so. The backup step reuses src/windows/KnownFolderOnboarding.tsx's logic: the same commands,
// Desktop, Documents and Pictures on by default, the prompt marked seen either way. File Explorer is Windows' Cloud Files
// registration (`windows_shell_integration_state`, `install_windows_shell_integration`, as the v2 spec lists them);
// macOS's Finder reconciler (`finder_setup_state`) answers only on a Mac.
import { useCallback, useEffect, useRef, useState } from 'react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { command, getKnownFolderBackup, markKnownFolderOnboardingSeen, setKnownFolderBackup, type KnownFolderStatus } from '../../desktopApi'
import { startGuide } from '../guide/Guide'
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

/** `windows_shell_integration_state` (src-tauri finder_setup_commands.rs, `FinderInstallState`): the last registration's result. */
export interface ShellIntegrationView {
  installed: boolean
  path: string | null
  /** `installed`, `error`, or `missing` before the first registration on this PC. */
  status: string
  last_error: string | null
  last_attempt_at: number | null
  reason_category: string | null
}

/** The HRESULT Windows returned, in mono for support (DW-0280: 0x8007017C), when the error text carries one. */
export function hresultOf(error: string | null): string | null {
  const m = error?.match(/0x[0-9a-f]{8}/i)
  return m ? `0x${m[0].slice(2).toUpperCase()}` : null
}

/**
 * The step's state: ready once registered; failed on a recorded error, or when the last try was refused before Rust
 * recorded anything (`refused`: its reason); adding while a try runs or before the first one.
 */
export function explorerStatus(v: ShellIntegrationView | null, trying = false, refused: string | null = null): { status: ExplorerStatus; code: string | null } {
  if (v?.installed || v?.status === 'installed') return { status: 'ready', code: null }
  if (trying) return { status: 'adding', code: null }
  if (v?.status === 'error') return { status: 'failed', code: hresultOf(v.last_error) }
  if (refused != null) return { status: 'failed', code: hresultOf(refused) }
  return { status: 'adding', code: null }
}

/** A PC that never registered starts here, right after sign-in ("Adding started at sign-in", DW-0206). */
export function needsFirstRegistration(v: ShellIntegrationView | null): boolean {
  return v != null && !v.installed && v.status === 'missing'
}

/** Whether setup reads the File Explorer state again every 2 s: on its own step, or while a try runs (#271 review). */
export const shellPolls = (step: string, trying: boolean) => step === 'explorer' || trying

/** "Show in File Explorer" before the config names a folder: nothing to open yet. */
export const NO_FOLDER_YET = 'Beebeeb doesn’t know this PC’s Beebeeb folder yet. Try again in a moment.'
/** "Show in File Explorer" refused (the folder could not be opened). */
export const SHOW_FAILED = 'Couldn’t open the Beebeeb folder in File Explorer. Try again in a moment.'
/** Try again when the state can't be read: it does not guess which folder to add. */
export const NO_STATE = 'Beebeeb couldn’t read this PC’s settings, so nothing was tried. Try again in a moment.'

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
  const [shell, setShell] = useState<ShellIntegrationView | null>(null)
  const [trying, setTrying] = useState(false)
  const [refused, setRefused] = useState<string | null>(null)
  const firstTry = useRef(false)
  const [login, setLogin] = useState(true)
  const note = (k: keyof SetupSummary, v: string) => setSummary((s) => ({ ...s, [k]: v }))

  const readShell = useCallback(async () => {
    const r = await command<ShellIntegrationView>('windows_shell_integration_state')
    setShell(r.ok ? r.value : null)
    return r.ok ? r.value : null
  }, [])
  // Registers this PC's sync folder with File Explorer (and starts sync there); Try again runs the same, on the same folder.
  // A try Rust refuses before it records an outcome (the session is changing, the folder cannot be made) still ends in the
  // failed state with Try again, never in "Adding" for ever. A browser without the command stays as it was.
  const addToExplorer = useCallback(async (path: string | null) => {
    setTrying(true)
    const r = await command('install_windows_shell_integration', { path })
    setRefused(r.ok || r.unsupported ? null : r.reason)
    setTrying(false)
    await readShell()
  }, [readShell])
  useEffect(() => {
    void readShell().then((v) => {
      if (!needsFirstRegistration(v) || firstTry.current) return
      firstTry.current = true
      void addToExplorer(v?.path ?? null)
    })
    void getKnownFolderBackup().then((r) => { if (r.ok) setKnown(r.value) })
    void command<boolean>('autostart_enabled').then((r) => { if (r.ok) setLogin(r.value) })
  }, [readShell, addToExplorer])
  // The state is read again every 2 s only while it can change on screen: the File Explorer step, or a try running
  // (each read loads the config from disk; a try's own answer already brings the new state). #271 review.
  useEffect(() => {
    if (!shellPolls(step, trying)) return
    const t = window.setInterval(() => void readShell(), 2000)
    return () => { window.clearInterval(t) }
  }, [step, trying, readShell])

  // "Show in File Explorer" opens this PC's sync folder, as v1 does (src/pages/SyncFolder.tsx): Rust refuses a null path
  // on Windows. A refusal is shown, never dropped (#271 P2).
  // eslint-disable-next-line beebeeb/no-ad-hoc-error-surface -- the setup window has no toast host: the refusal stays under the button it is about
  const [showError, setShowError] = useState<string | null>(null)
  const showInExplorer = async () => {
    const path = shell?.path ?? (await readShell())?.path ?? null
    if (!path) { setShowError(NO_FOLDER_YET); return }
    const r = await command<void>('open_finder_location', { path })
    // Our words, never the command's text (the Finder family's rule, tests/finderSetupSourceContract.test.ts).
    setShowError(r.ok || r.unsupported ? null : SHOW_FAILED)
  }
  // Try again registers the folder the config holds: the state is read first, and without a read it does not guess
  // (Rust would fall back to its default folder, not the configured one). #271 review.
  const tryAgain = async () => {
    setShowError(null)
    const v = await readShell()
    if (!v) { setShowError(NO_STATE); return }
    await addToExplorer(v.path ?? null)
  }

  const explorer = explorerStatus(shell, trying, refused)
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
    // The main window opens on Files and runs the guide once (DW-0380); this setup window closes (DW-0208's rule).
    startGuide()
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
        onAddFolder={() => void command('show_main_app_settings', { page: 'backups' })} onTurnOn={() => void turnOnBackup()}
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
      body = <ExplorerStep status={explorer.status} errorCode={explorer.code} showError={showError} onShow={() => void showInExplorer()}
        onTryAgain={() => void tryAgain()} onContinue={() => setStep('notifications')} />
      break
    case 'notifications':
      body = <NotificationsStep onAllow={() => { note('notifications', 'On'); setStep('login') }} onNotNow={() => { note('notifications', 'Off'); setStep('login') }} />
      break
    case 'login':
      body = <LoginStep on={login} onToggle={setLogin} onContinue={() => void loginContinue()} />
      break
    case 'done':
      body = <DoneStep summary={summary} onRow={(page) => void command('show_main_app_settings', { page })} onOpen={() => void finish()} />
      break
  }
  return <SetupWindow step={step} explorer={explorer.status} onSkip={step === 'done' || step === 'compare' ? undefined : skip}>{body}</SetupWindow>
}
