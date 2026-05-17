import { useCallback, useEffect, useState, type FormEvent, type ReactNode } from 'react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import {
  command,
  commandUnavailableLabel,
  loadSyncStatus,
  type CommandResult,
  type SyncStatus,
  type VaultItem,
} from './desktopApi'
import logoFull from './assets/logo-full.svg'

type Step = 'signin' | 'unlock' | 'finder' | 'pinning' | 'ready'

const STEPS: Array<{ id: Step; title: string; detail: string }> = [
  { id: 'signin', title: 'Sign in', detail: 'Authenticate your account.' },
  { id: 'unlock', title: 'Set up this Mac', detail: 'Restore the vault key on this device.' },
  { id: 'finder', title: 'Install Finder location', detail: 'Register Beebeeb in the Finder sidebar.' },
  { id: 'pinning', title: 'Choose offline folders', detail: 'Default is online-only; pin only what you need.' },
  { id: 'ready', title: 'Review status', detail: 'Open the control center.' },
]

export default function Onboarding() {
  const [step, setStep] = useState<Step>('signin')

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
  const [error, setError] = useState<string | null>(null)

  const submit = async (event: FormEvent) => {
    event.preventDefault()
    setBusy(true)
    setError(null)
    const result = await command<void>('desktop_login', { email, password })
    setBusy(false)
    if (result.ok) {
      onDone()
      return
    }
    setError(result.unsupported ? commandUnavailableLabel('desktop_login') : result.reason)
  }

  return (
    <Card
      title="Welcome back"
      copy="Sign in to unlock your encrypted vault."
    >
      {error && <div className="notice error">{error}</div>}
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
  const [recoveryPhrase, setRecoveryPhrase] = useState('')
  const [busy, setBusy] = useState(false)
  const [result, setResult] = useState<CommandResult<void> | null>(null)

  const unlock = async (event: FormEvent) => {
    event.preventDefault()
    setBusy(true)
    const next = await command<void>('desktop_unlock_with_recovery_phrase', { recoveryPhrase })
    setResult(next)
    setBusy(false)
    if (next.ok) onDone()
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
        <Field
          label="Recovery phrase"
          type="password"
          value={recoveryPhrase}
          onChange={setRecoveryPhrase}
          disabled={busy}
          placeholder="word word word..."
        />
        <div className="row-detail" style={{ marginTop: -6, marginBottom: 14 }}>
          Paste or type all 12 words, separated by spaces. Beebeeb stores the unlocked vault key in
          macOS Keychain for future unlocks.
        </div>
        <button className="button primary" type="submit" disabled={busy || !recoveryPhrase.trim()}>
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
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState<string | null>(null)

  useEffect(() => {
    command<string>('default_sync_root').then((result) => {
      if (result.ok) setSyncRoot(result.value)
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
    const result = await command<void>('install_finder_location', { path: syncRoot })
    setBusy(false)
    if (result.ok) {
      onDone()
      return
    }
    setMessage(result.unsupported ? commandUnavailableLabel('install_finder_location') : result.reason)
  }, [onDone, syncRoot])

  return (
    <Card
      title="Install the Finder location"
      copy="Beebeeb should appear as a Finder sidebar location. This is separate from choosing optional offline folders."
    >
      {message && <div className="notice">{message}</div>}
      <div className="panel" style={{ marginTop: 16, background: '#faf8f5' }}>
        <div className="section-label">Finder path</div>
        <div className="mono" style={{ marginTop: 8, fontSize: 13 }}>
          {syncRoot ?? '~/Beebeeb'}
        </div>
      </div>
      <div className="button-row" style={{ marginTop: 16 }}>
        <button className="button" onClick={chooseFolder} disabled={busy}>
          Choose location
        </button>
        <button className="button primary" onClick={install} disabled={busy}>
          {busy ? 'Installing…' : 'Install Finder location'}
        </button>
        {message && (
          <button className="button" onClick={onDone}>
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
  const [notice, setNotice] = useState<string | null>(null)

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
      setNotice(result.unsupported ? commandUnavailableLabel('set_recursive_pin') : result.reason)
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
    await command<void>('show_settings')
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
