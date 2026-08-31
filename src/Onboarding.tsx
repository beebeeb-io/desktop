import { useCallback, useEffect, useRef, useState, type FormEvent, type KeyboardEvent, type ReactNode } from 'react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import {
  command,
  commandUnavailableLabel,
  loadSyncStatus,
  type CommandResult,
  type DesktopPlatform,
  type FinderInstallState,
  type SyncStatus,
  type VaultItem,
} from './desktopApi'
import logoFull from './assets/logo-full.svg'
import { useToast } from './windows/ui'

type Step = 'signin' | 'unlock' | 'finder' | 'pinning' | 'ready'
const RECOVERY_WORD_COUNT = 12

const STEPS: Array<{ id: Step; title: string; detail: string }> = [
  { id: 'signin', title: 'Sign in', detail: 'Authenticate your account.' },
  { id: 'unlock', title: 'Set up this Mac', detail: 'Restore the vault key on this device.' },
  { id: 'finder', title: 'Install Finder location', detail: 'Register Beebeeb in the Finder sidebar.' },
  { id: 'pinning', title: 'Choose offline folders', detail: 'Default is online-only; pin only what you need.' },
  { id: 'ready', title: 'Review status', detail: 'Open the control center.' },
]

export default function Onboarding() {
  const [step, setStep] = useState<Step>('signin')

  useEffect(() => {
    let cancelled = false

    Promise.all([
      loadSyncStatus(),
      command<DesktopPlatform>('desktop_platform'),
      command<FinderInstallState>('finder_location_state'),
    ]).then(([status, platform, finder]) => {
      if (cancelled || !status?.logged_in) return

      if (!status.vault_unlocked) {
        setStep('unlock')
        return
      }

      const isMacos = platform.ok && platform.value === 'macos'
      if ((isMacos && (!finder.ok || !finder.value.installed)) || (!isMacos && !status.sync_root)) {
        setStep('finder')
        return
      }
      setStep('ready')
    })

    return () => {
      cancelled = true
    }
  }, [])

  return (
    <div className="onboarding-shell">
      <aside className="onboarding-rail">
        <div>
          <div className="onboarding-brand">
            <img src={logoFull} alt="beebeeb.io" className="onboarding-logo" />
            <div className="brand-subtitle">Private macOS file access</div>
          </div>
          <div className="steps">
            {STEPS.map((item, index) => (
              <div key={item.id} className={`step-row ${step === item.id ? 'active' : ''}`}>
                <div className="step-number">{index + 1}</div>
                <div>
                  <div className="row-title">{item.title}</div>
                  <div className="row-detail">{item.detail}</div>
                </div>
              </div>
            ))}
          </div>
        </div>
        <div className="sidebar-footer">
          End-to-end encrypted | EU servers | Zero-knowledge
        </div>
      </aside>

      <main className="onboarding-main">
        {step === 'signin' && <SignInStep onDone={() => setStep('unlock')} />}
        {step === 'unlock' && <UnlockStep onDone={() => setStep('finder')} />}
        {step === 'finder' && <FinderInstallStep onDone={() => setStep('pinning')} />}
        {step === 'pinning' && <PinningStep onDone={() => setStep('ready')} />}
        {step === 'ready' && <ReadyStep />}
      </main>
    </div>
  )
}

function Card({
  title,
  copy,
  children,
}: {
  title: string
  copy: string
  children: ReactNode
}) {
  return (
    <section className="auth-card">
      <div className="auth-card-header">
        <img src={logoFull} alt="beebeeb.io" className="auth-logo" />
      </div>
      <h1 className="page-title">{title}</h1>
      <p className="page-copy" style={{ marginBottom: 22 }}>
        {copy}
      </p>
      {children}
    </section>
  )
}

function Field({
  label,
  type,
  value,
  onChange,
  disabled,
  placeholder,
}: {
  label: string
  type: string
  value: string
  onChange: (value: string) => void
  disabled?: boolean
  placeholder?: string
}) {
  return (
    <label style={{ display: 'block', marginBottom: 14 }}>
      <span className="field-label" style={{ display: 'block', marginBottom: 6 }}>
        {label}
      </span>
      <input
        className="form-input"
        type={type}
        value={value}
        disabled={disabled}
        placeholder={placeholder}
        onChange={(event) => onChange(event.currentTarget.value)}
      />
    </label>
  )
}

function SignInStep({ onDone }: { onDone: () => void }) {
  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [busy, setBusy] = useState(false)
  const { showToast } = useToast()

  const submit = async (event: FormEvent) => {
    event.preventDefault()
    setBusy(true)
    const result = await command<void>('desktop_login', { email, password })
    setBusy(false)
    if (result.ok) {
      onDone()
      return
    }
    showToast({
      variant: 'error',
      title: 'Sign-in failed',
      message: result.unsupported ? commandUnavailableLabel('desktop_login') : result.reason,
    })
  }

  return (
    <Card
      title="Welcome back"
      copy="Sign in to unlock your encrypted vault."
    >
      <form onSubmit={submit} style={{ marginTop: 16 }}>
        <Field label="Email" type="email" value={email} onChange={setEmail} disabled={busy} placeholder="you@example.com" />
        <Field label="Password" type="password" value={password} onChange={setPassword} disabled={busy} placeholder="Your password" />
        <button className="button primary" type="submit" disabled={!email || !password || busy}>
          {busy ? 'Signing in…' : 'Sign in'}
        </button>
      </form>
    </Card>
  )
}

function UnlockStep({ onDone }: { onDone: () => void }) {
  const [recoveryWords, setRecoveryWords] = useState<string[]>(() =>
    Array.from({ length: RECOVERY_WORD_COUNT }, () => ''),
  )
  const [busy, setBusy] = useState(false)
  const [result, setResult] = useState<CommandResult<void> | null>(null)
  const wordRefs = useRef<Array<HTMLInputElement | null>>([])

  const recoveryPhrase = recoveryWords.map((word) => word.trim()).join(' ').trim()
  const filledWords = recoveryWords.filter((word) => word.trim()).length
  const canUnlock = filledWords === RECOVERY_WORD_COUNT

  const unlock = async (event: FormEvent) => {
    event.preventDefault()
    if (!canUnlock) return
    setBusy(true)
    const next = await command<void>('desktop_unlock_with_recovery_phrase', { recoveryPhrase })
    setResult(next)
    setBusy(false)
    if (next.ok) onDone()
  }

  const applyWords = (startIndex: number, rawWords: string[]) => {
    const cleanWords = rawWords.map((word) => word.trim()).filter(Boolean)
    if (cleanWords.length === 0) return

    setResult(null)
    setRecoveryWords((current) => {
      const next = [...current]
      cleanWords.slice(0, RECOVERY_WORD_COUNT - startIndex).forEach((word, offset) => {
        next[startIndex + offset] = word
      })
      return next
    })

    const nextIndex = Math.min(startIndex + cleanWords.length, RECOVERY_WORD_COUNT - 1)
    requestAnimationFrame(() => wordRefs.current[nextIndex]?.focus())
  }

  const updateWord = (index: number, value: string) => {
    if (/\s/.test(value)) {
      applyWords(index, value.split(/\s+/))
      return
    }

    setResult(null)
    setRecoveryWords((current) => current.map((word, wordIndex) => (wordIndex === index ? value : word)))
  }

  const onWordKeyDown = (event: KeyboardEvent<HTMLInputElement>, index: number) => {
    if (event.key === ' ' || event.key === 'Enter') {
      event.preventDefault()
      wordRefs.current[Math.min(index + 1, RECOVERY_WORD_COUNT - 1)]?.focus()
      return
    }
    if (event.key === 'Backspace' && !recoveryWords[index] && index > 0) {
      wordRefs.current[index - 1]?.focus()
    }
  }

  return (
    <Card
      title="Set up this Mac"
      copy="This Mac does not have your encryption keys yet. Restore them to continue."
    >
      {result && !result.ok && (
        <div className="notice">
          {result.unsupported ? commandUnavailableLabel('desktop_unlock_with_recovery_phrase') : result.reason}
        </div>
      )}
      <form onSubmit={unlock} style={{ marginTop: 16 }}>
        <div className="field-label" style={{ marginBottom: 8 }}>
          Recovery phrase
        </div>
        <div className="recovery-grid">
          {recoveryWords.map((word, index) => (
            <label className="recovery-word" key={index}>
              <span className="word-index">{index + 1}</span>
              <input
                ref={(node) => {
                  wordRefs.current[index] = node
                }}
                aria-label={`Recovery word ${index + 1}`}
                autoCapitalize="none"
                autoComplete="off"
                autoCorrect="off"
                className="recovery-input"
                disabled={busy}
                spellCheck={false}
                type="text"
                value={word}
                onChange={(event) => updateWord(index, event.currentTarget.value)}
                onKeyDown={(event) => onWordKeyDown(event, index)}
                onPaste={(event) => {
                  event.preventDefault()
                  applyWords(index, event.clipboardData.getData('text').split(/\s+/))
                }}
              />
            </label>
          ))}
        </div>
        <div className="row-detail" style={{ marginTop: 10, marginBottom: 14 }}>
          Paste all 12 words into any box or type them one by one. Beebeeb stores the unlocked vault
          key in macOS Keychain for future unlocks.
        </div>
        <button className="button primary" type="submit" disabled={busy || !canUnlock}>
          {busy ? 'Unlocking…' : 'Unlock vault'}
        </button>
      </form>
      <div className="button-row" style={{ marginTop: 12 }}>
        {result && !result.ok && result.unsupported && (
          <button className="button" onClick={onDone}>
            Continue to Finder setup
          </button>
        )}
      </div>
    </Card>
  )
}

function FinderInstallStep({ onDone }: { onDone: () => void }) {
  const [syncRoot, setSyncRoot] = useState<string | null>(null)
  const [platform, setPlatform] = useState<DesktopPlatform>('unknown')
  const [finderPath, setFinderPath] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  // KEPT INLINE BY DESIGN — task 1255, lead ruling 2026-08-31 ("option (a)").
  //
  // DO NOT convert this to a Toast and delete the state. `message` is not decoration:
  // it GATES the "Continue without install" button in the button row below. Delete this
  // state and that button disappears from the product — a user whose Finder install
  // fails is left with no way past this onboarding step.
  //
  // All three setMessage sites (the pick_sync_root, install_finder_location and
  // continue_without_finder_location failures) feed that same gate, so they stay
  // together; splitting only some of them leaves the gate driven by a subset of its
  // causes, which is worse than either alternative.
  //
  // Same load-bearing shape as WindowsFirstRun's UnlockStep and its "Continue without
  // unlock" escape hatch. See the 1255 task file for the full ruling and the
  // `escapeHatchVisible: true` evidence.
  const [message, setMessage] = useState<string | null>(null)

  useEffect(() => {
    command<DesktopPlatform>('desktop_platform').then((result) => {
      if (result.ok) setPlatform(result.value)
    })
    command<string>('default_sync_root').then((result) => {
      if (result.ok) setSyncRoot(result.value)
    })
    command<FinderInstallState>('finder_location_state').then((result) => {
      if (result.ok) setFinderPath(result.value.path ?? null)
    })
  }, [])

  const chooseFolder = useCallback(async () => {
    setBusy(true)
    setMessage(null)
    const picked = await command<string | null>('pick_sync_root')
    setBusy(false)
    if (picked.ok && picked.value) {
      setSyncRoot(picked.value)
      return
    }
    if (!picked.ok) setMessage(picked.unsupported ? commandUnavailableLabel('pick_sync_root') : picked.reason)
  }, [])

  const install = useCallback(async () => {
    setBusy(true)
    setMessage(null)
    const result = await command<FinderInstallState>('install_finder_location', {
      path: platform === 'macos' ? null : syncRoot,
    })
    setBusy(false)
    if (result.ok) {
      setFinderPath(result.value.path ?? null)
      onDone()
      return
    }
    setMessage(result.unsupported ? commandUnavailableLabel('install_finder_location') : result.reason)
  }, [onDone, platform, syncRoot])

  const continueWithoutInstall = useCallback(async () => {
    setBusy(true)
    setMessage(null)
    const result = await command<void>('continue_without_finder_location', { path: syncRoot })
    setBusy(false)
    if (result.ok) {
      onDone()
      return
    }
    setMessage(result.unsupported ? commandUnavailableLabel('continue_without_finder_location') : result.reason)
  }, [onDone, syncRoot])

  const isMacos = platform === 'macos'

  return (
    <Card
      title="Install the Finder location"
      copy={
        isMacos
          ? 'Beebeeb appears as a system-managed Finder location. Offline folders are controlled separately.'
          : 'Beebeeb should appear as a file-manager location. This is separate from choosing optional offline folders.'
      }
    >
      {message && <div className="notice">{message}</div>}
      <div className="panel" style={{ marginTop: 16, background: '#faf8f5' }}>
        <div className="section-label">{isMacos ? 'Finder location' : 'Folder path'}</div>
        <div className="mono" style={{ marginTop: 8, fontSize: 13 }}>
          {finderPath ?? (isMacos ? 'Beebeeb in Finder' : syncRoot ?? '~/Beebeeb')}
        </div>
      </div>
      <div className="button-row" style={{ marginTop: 16 }}>
        {!isMacos && (
          <button className="button" onClick={chooseFolder} disabled={busy}>
            Choose location
          </button>
        )}
        <button className="button primary" onClick={install} disabled={busy}>
          {busy ? 'Installing…' : 'Install Finder location'}
        </button>
        {/* This escape hatch EXISTS ONLY while `message` is set — it is the gate described
            on the `message` state above. Removing the inline error removes this button.
            Read that comment before refactoring either one. */}
        {message && !isMacos && (
          <button className="button" onClick={continueWithoutInstall} disabled={busy}>
            Continue without install
          </button>
        )}
      </div>
    </Card>
  )
}

function PinningStep({ onDone }: { onDone: () => void }) {
  const [items, setItems] = useState<VaultItem[]>([])
  const [loading, setLoading] = useState(true)
  // `notice` is deliberately kept for the LOAD failure only: when `list_remote_tree`
  // fails we fall back to `list_vault_folders`, so the list on screen is degraded and
  // needs a persistent explanation. The toggle failure below is a transient ACTION
  // failure and goes to a Toast instead.
  const [notice, setNotice] = useState<string | null>(null)
  const { showToast } = useToast()

  useEffect(() => {
    let cancelled = false
    command<VaultItem[]>('list_remote_tree').then(async (tree) => {
      if (cancelled) return
      if (tree.ok) {
        setItems(tree.value)
        setLoading(false)
        return
      }
      const topLevel = await command<VaultItem[]>('list_vault_folders')
      if (cancelled) return
      if (topLevel.ok) setItems(topLevel.value)
      setNotice(
        tree.unsupported
          ? commandUnavailableLabel('list_remote_tree')
          : tree.reason,
      )
      setLoading(false)
    })
    return () => {
      cancelled = true
    }
  }, [])

  const togglePin = async (item: VaultItem) => {
    const nextPinned = !item.pinned
    const result = await command<void>('set_recursive_pin', {
      itemId: item.id,
      pinned: nextPinned,
    })
    if (!result.ok) {
      showToast({
        variant: 'error',
        title: nextPinned ? 'Couldn’t make folder offline' : 'Couldn’t make folder online-only',
        message: result.unsupported ? commandUnavailableLabel('set_recursive_pin') : result.reason,
      })
      return
    }
    setItems((current) =>
      current.map((entry) => (entry.id === item.id ? { ...entry, pinned: nextPinned } : entry)),
    )
  }

  return (
    <Card
      title="Start online-only"
      copy="No folders are pinned by default. You can make any folder recursively available offline now or later from the control center."
    >
      {notice && <div className="notice">{notice}</div>}
      {loading ? (
        <div className="empty-state" style={{ marginTop: 16 }}>
          Loading remote tree…
        </div>
      ) : items.length === 0 ? (
        <div className="empty-state" style={{ marginTop: 16 }}>
          No remote folders are available yet. Onboarding will continue with everything online-only.
        </div>
      ) : (
        <div className="tree" style={{ marginTop: 16 }}>
          {items
            .filter((item) => item.is_folder)
            .map((item) => (
              <div className="tree-row" key={item.id}>
                <div>
                  <div className="row-title">{item.name}</div>
                  <div className="row-detail">Recursive offline availability</div>
                </div>
                <button className="button" onClick={() => void togglePin(item)}>
                  {item.pinned ? 'Pinned' : 'Online-only'}
                </button>
              </div>
            ))}
        </div>
      )}
      <div className="button-row" style={{ marginTop: 18 }}>
        <button className="button primary" onClick={onDone}>
          Continue with no pinned folders
        </button>
      </div>
    </Card>
  )
}

function ReadyStep() {
  const [status, setStatus] = useState<SyncStatus | null>(null)

  useEffect(() => {
    void loadSyncStatus().then(setStatus)
  }, [])

  const finish = async () => {
    await command<void>('show_main_app_window')
    try {
      const win = getCurrentWindow()
      await win.close()
    } catch {
      window.close()
    }
  }

  return (
    <Card
      title="Control center is ready"
      copy="Use the app for sync health, lock state, offline folders, shared roots, versions, conflicts, and diagnostics. Finder remains the file surface."
    >
      <div className="grid three" style={{ marginTop: 16 }}>
        <div className="metric">
          <div className="metric-label">Engine</div>
          <div className="metric-value">{status?.engine ?? 'Unknown'}</div>
        </div>
        <div className="metric">
          <div className="metric-label">Queue</div>
          <div className="metric-value">{status?.syncing ?? 0}</div>
        </div>
        <div className="metric">
          <div className="metric-label">Conflicts</div>
          <div className="metric-value">{status?.conflicts ?? 0}</div>
        </div>
      </div>
      <button className="button primary" onClick={() => void finish()} style={{ marginTop: 18 }}>
        Open control center
      </button>
    </Card>
  )
}
