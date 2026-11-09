// First run on Windows (DW-0201 to DW-0209): the setup window, 960 x 640, with the step list and the File Explorer status
// at the left. Pure views; the controller is ./index.tsx.
import type { ReactNode } from 'react'
import { BBProductLockup, BBToggle, Icon, type IconName } from '@beebeeb/shared/design'
import { CaptionBarView } from '../chrome/CaptionBar'
import { Field, HelloGlyph, PrimaryButton, SecondaryButton, TextLink } from '../auth/parts'

export type SetupStep = 'passkey' | 'lock' | 'backup' | 'photos' | 'explorer' | 'notifications' | 'login'
export const STEPS: { id: SetupStep; label: string }[] = [
  { id: 'passkey', label: 'Passkey' },
  { id: 'lock', label: 'App lock' },
  { id: 'backup', label: 'Back up folders' },
  { id: 'photos', label: 'Photo previews' },
  { id: 'explorer', label: 'File Explorer' },
  { id: 'notifications', label: 'Notifications' },
  { id: 'login', label: 'Open at login' },
]

export type ExplorerStatus = 'ready' | 'adding' | 'failed'

const EXPLORER_LINE: Record<ExplorerStatus, string> = { ready: 'Beebeeb is in File Explorer', adding: 'Adding Beebeeb to File Explorer…', failed: 'Couldn’t add Beebeeb to File Explorer' }

/** The setup window: caption bar, step list and File Explorer status at the left, the step at the right. */
export function SetupWindow({ step, explorer, onSkip, children }: { step: SetupStep | 'done' | 'compare'; explorer: ExplorerStatus; onSkip?: () => void; children: ReactNode }) {
  const index = STEPS.findIndex((s) => s.id === step)
  return (
    <div className="relative flex h-screen flex-col overflow-hidden bg-paper font-sans text-ink" data-setup-window>
      <CaptionBarView title="Set up Beebeeb" />
      <div className="flex min-h-0 flex-1">
        <aside className="flex w-[240px] flex-none flex-col justify-between border-r border-line bg-paper-2 px-5 py-6" aria-label="Setup steps">
          <div>
            <BBProductLockup product="drive" size={28} />
            {index >= 0 && <p className="mb-2 mt-5 font-mono text-[11px] text-ink-3">Step {index + 1} of 7</p>}
            <ol className={`m-0 list-none p-0 ${index >= 0 ? '' : 'mt-5'}`}>
              {STEPS.map((s, i) => {
                const done = step === 'done' || (index >= 0 && i < index)
                const current = i === index || (step === 'compare' && s.id === 'photos')
                return (
                  <li key={s.id} data-step-item={current ? 'current' : done ? 'done' : 'todo'} aria-current={current ? 'step' : undefined}
                    className={`flex h-8 items-center gap-2.5 text-[13px] ${current ? 'font-medium text-ink' : done ? 'text-ink-2' : 'text-ink-3'}`}>
                    <span className={`grid h-5 w-5 flex-none place-items-center rounded-full border text-[10px] ${current ? 'border-ink-2' : 'border-line'}`}>
                      {done ? <Icon name="check" size={11} /> : <span className="font-mono">{i + 1}</span>}
                    </span>
                    {s.label}
                  </li>
                )
              })}
            </ol>
          </div>
          <div className="text-[12px] leading-4 text-ink-2" data-explorer-status={explorer} role="status">
            <div className="mb-1 flex items-center gap-1.5 text-[11px] uppercase tracking-[0.06em] text-ink-3"><Icon name="folder" size={12} />File Explorer</div>
            <span className="flex items-center gap-1.5">
              <span className={`h-1.5 w-1.5 rounded-full ${explorer === 'ready' ? 'bg-green' : explorer === 'failed' ? 'bg-red' : 'bg-ink-3'}`} aria-hidden />
              {EXPLORER_LINE[explorer]}
            </span>
          </div>
        </aside>
        <main className="flex min-w-0 flex-1 flex-col overflow-auto px-12 py-8">
          <div className="m-auto w-full max-w-[440px]">{children}</div>
          {onSkip && <div className="mt-4 text-right"><TextLink onClick={onSkip}>Skip setup</TextLink></div>}
        </main>
      </div>
    </div>
  )
}

function StepHeading({ title, sub, icon }: { title: string; sub: ReactNode; icon?: IconName | 'hello' }) {
  return (
    <div className="mb-5">
      {icon && <div className="mb-4 grid h-12 w-12 place-items-center rounded-full bg-paper-3 text-ink-2">{icon === 'hello' ? <HelloGlyph size={24} /> : <Icon name={icon} size={22} />}</div>}
      <h1 className="m-0 text-[22px] font-semibold tracking-[-0.01em]">{title}</h1>
      <p className="mt-1.5 text-[13px] leading-[19px] text-ink-2">{sub}</p>
    </div>
  )
}

function Buttons({ children }: { children: ReactNode }) {
  return <div className="mt-6 flex gap-2.5 [&>*]:flex-1">{children}</div>
}

function Fact({ icon, title, children }: { icon: IconName; title: string; children: ReactNode }) {
  return (
    <li className="flex gap-3 py-2" data-fact>
      <Icon name={icon} size={16} className="mt-0.5 flex-none text-ink-3" />
      <span className="text-[13px] leading-[18px]"><span className="font-medium">{title}</span> <span className="text-ink-2">{children}</span></span>
    </li>
  )
}

// DW-0201
export function PasskeyStep({ asking, password, error, onPassword, onAdd, onNotNow }: {
  asking: boolean; password: string; error: string | null; onPassword: (v: string) => void; onAdd: () => void; onNotNow: () => void
}) {
  return (
    <>
      <StepHeading icon="hello" title="Add a passkey" sub="Sign in and unlock your files on a new device, with no codes. It doesn’t replace your 12 words, so keep those safe." />
      <ul className="m-0 list-none divide-y divide-line border-y border-line p-0">
        <Fact icon="lock" title="Saved by Windows">In your Passwords app or password manager, protected by Windows Hello.</Fact>
        <Fact icon="shield" title="Private">Beebeeb can’t use it to open your files by itself.</Fact>
      </ul>
      {asking && (
        <form className="mt-4" onSubmit={(e) => { e.preventDefault(); if (password) onAdd() }} noValidate>
          {/* Deviation (PR body): B15's account_passkeys_add takes the password for its step-up. */}
          <Field label="Your password, to confirm it’s you" icon="lock" secret value={password} error={error ?? undefined} onChange={(e) => onPassword(e.target.value)} />
        </form>
      )}
      <Buttons>
        <PrimaryButton onClick={onAdd} disabled={asking && !password}>Add a passkey</PrimaryButton>
        <SecondaryButton onClick={onNotNow}>Not now</SecondaryButton>
      </Buttons>
    </>
  )
}

export const LOCK_AFTER = ['Immediately', '5 min', '30 min', '60 min'] as const
export type LockAfter = (typeof LOCK_AFTER)[number]

// DW-0202
export function LockStep({ on, after, onToggle, onAfter, onContinue, onNotNow }: {
  on: boolean; after: LockAfter; onToggle: (v: boolean) => void; onAfter: (v: LockAfter) => void; onContinue: () => void; onNotNow: () => void
}) {
  return (
    <>
      <StepHeading icon="lock" title="Lock Beebeeb" sub="Ask for Windows Hello when you open the app, so nobody else can look through your files on this PC." />
      <div className="flex items-center justify-between border-y border-line py-3">
        <span className="text-[13px] font-medium">App lock</span>
        <BBToggle on={on} onChange={onToggle} aria-label="App lock" />
      </div>
      <div className={`flex items-center justify-between border-b border-line py-3 ${on ? '' : 'opacity-50'}`}>
        <span className="text-[13px]">Lock again after</span>
        <div role="radiogroup" aria-label="Lock again after" className="flex rounded-md border border-line p-0.5">
          {LOCK_AFTER.map((a) => (
            <button key={a} type="button" role="radio" aria-checked={after === a} disabled={!on} onClick={() => onAfter(a)}
              className={`rounded px-2.5 py-1 text-[12px] ${after === a ? 'bg-paper-3 font-medium text-ink' : 'text-ink-2'}`}><span className="text-[12px]">{a}</span></button>
          ))}
        </div>
      </div>
      <p className="mt-3 text-[12px] leading-4 text-ink-3">Sync keeps running while the app is locked. File Explorer follows your PC’s lock instead.</p>
      <Buttons>
        <PrimaryButton amber={false} onClick={onContinue}>Continue</PrimaryButton>
        <SecondaryButton onClick={onNotNow}>Not now</SecondaryButton>
      </Buttons>
    </>
  )
}

export interface BackupFolder { key: string; name: string; detail: string; on: boolean }

// DW-0203
export function BackupStep({ folders, busy, error, onToggle, onAddFolder, onTurnOn, onNotNow }: {
  folders: BackupFolder[]; busy: boolean; error: string | null
  onToggle: (key: string, on: boolean) => void; onAddFolder: () => void; onTurnOn: () => void; onNotNow: () => void
}) {
  return (
    <>
      <StepHeading title="Back up your folders" sub="Your folders are encrypted on this PC before they upload. They stay where they are." />
      {error && <p role="alert" className="mb-2 text-[12px] text-red">{error}</p>}
      <ul className="m-0 list-none divide-y divide-line border-y border-line p-0" data-folders>
        {folders.map((f) => (
          <li key={f.key} data-folder={f.key} className="flex items-center gap-3 py-2">
            <Icon name="folder" size={16} className="flex-none text-ink-3" />
            <span className="flex-1 text-[13px]">{f.name}</span>
            <span className="font-mono text-[11px] text-ink-3">{f.detail}</span>
            <BBToggle on={f.on} onChange={(v) => onToggle(f.key, v)} aria-label={`Back up ${f.name}`} disabled={busy} />
          </li>
        ))}
      </ul>
      <p className="mt-3 text-[12px] leading-4 text-ink-3">Change these any time in Settings › Backups, and add other folders there. <TextLink onClick={onAddFolder}>Add a folder…</TextLink></p>
      <Buttons>
        <PrimaryButton onClick={onTurnOn} disabled={busy || !folders.some((f) => f.on)}>{busy ? 'Turning on…' : 'Turn on backup'}</PrimaryButton>
        <SecondaryButton onClick={onNotNow}>Not now</SecondaryButton>
      </Buttons>
    </>
  )
}

export interface PreviewSample { label: string; size: string; original: string; photos: number; local: string; cloud: string; space: string }
export const PREVIEW_SIZES = ['2K', '4K', 'Full'] as const
export const PREVIEW_FORMATS = ['HEIC', 'JPEG', 'WebP'] as const

// DW-0204
export function PhotosStep({ sample, size, format, onSize, onFormat, onCompare, onContinue, onNotNow }: {
  sample: PreviewSample; size: string; format: string
  onSize: (v: string) => void; onFormat: (v: string) => void; onCompare: () => void; onContinue: () => void; onNotNow: () => void
}) {
  const seg = (label: string, options: readonly string[], value: string, on: (v: string) => void) => (
    <div className="flex items-center justify-between border-b border-line py-2.5">
      <span className="text-[13px]">{label}</span>
      <div role="radiogroup" aria-label={label} className="flex rounded-md border border-line p-0.5">
        {options.map((o) => (
          <button key={o} type="button" role="radio" aria-checked={value === o} onClick={() => on(o)}
            className={`rounded px-2.5 py-1 text-[12px] ${value === o ? 'bg-paper-3 font-medium text-ink' : 'text-ink-2'}`}><span className="text-[12px]">{o}</span></button>
        ))}
      </div>
    </div>
  )
  return (
    <>
      <StepHeading title="Photo previews" sub="Photos open instantly from a saved preview, instead of downloading the full original." />
      <div className="mb-1 flex items-center gap-3 rounded-md border border-line p-2.5" data-sample>
        <span className="grid h-14 w-14 flex-none place-items-center rounded bg-paper-3 text-ink-3"><Icon name="image" size={22} /></span>
        <span className="flex-1 text-[12px] leading-[17px]">
          <span className="block font-medium text-ink">{sample.label} · <span className="font-mono">{sample.size}</span></span>
          <span className="block text-ink-3">Original <span className="font-mono">{sample.original}</span></span>
        </span>
        <SecondaryButton onClick={onCompare}>Compare</SecondaryButton>
      </div>
      <div className="border-t border-line">
        {seg('Preview size', PREVIEW_SIZES, size, onSize)}
        {seg('Format', PREVIEW_FORMATS, format, onFormat)}
        <div className="flex items-center justify-between border-b border-line py-2.5 text-[13px]"><span>Space on this PC</span><span className="font-mono text-[12px] text-ink-2">{sample.space}</span></div>
      </div>
      <p className="mt-3 text-[12px] leading-4 text-ink-2">All <span className="font-mono">{sample.photos.toLocaleString('en-US')}</span> photos fit, about <span className="font-mono">{sample.local}</span>. Also about <span className="font-mono">{sample.cloud}</span> in your Beebeeb storage.</p>
      <Buttons>
        <PrimaryButton amber={false} onClick={onContinue}>Continue</PrimaryButton>
        <SecondaryButton onClick={onNotNow}>Not now</SecondaryButton>
      </Buttons>
    </>
  )
}

/** A piece of the navigation pane (Windows draws it; drawn here for reference, as the board does). */
function NavPane() {
  const rows: { name: string; icon: IconName; selected?: boolean }[] = [
    { name: 'Home', icon: 'folder' }, { name: 'Gallery', icon: 'image' }, { name: 'Beebeeb', icon: 'cloud', selected: true }, { name: 'Desktop', icon: 'folder' }, { name: 'Documents', icon: 'folder' },
  ]
  return (
    <ul className="m-0 my-4 w-[220px] list-none rounded-md border border-[#e0e0e0] bg-[#f9f9f9] p-1.5 font-sans text-[12px] text-[#1a1a1a]" data-system-drawn aria-label="File Explorer navigation pane">
      {rows.map((r) => (
        <li key={r.name} data-nav-row className={`flex h-7 items-center gap-2 rounded px-2 ${r.selected ? 'bg-[#e5e5e5] font-medium' : ''}`}>
          <Icon name={r.icon} size={14} className="text-[#5c5c5c]" />{r.name}
        </li>
      ))}
    </ul>
  )
}

// DW-0206, DW-0206b, DW-0280
export function ExplorerStep({ status, errorCode, showError = null, onShow, onContinue, onTryAgain }: {
  status: ExplorerStatus; errorCode: string | null
  /** Why "Show in File Explorer" did not open it (Rust's refusal), or why Try again tried nothing (the state could not
   *  be read): shown under the buttons (#271 P2 and review). */
  showError?: string | null
  onShow: () => void; onContinue: () => void; onTryAgain: () => void
}) {
  if (status === 'adding') {
    return (
      <>
        <StepHeading icon="folder" title="Adding Beebeeb to File Explorer" sub="This takes a few seconds. You can carry on; it finishes by itself." />
        <div className="h-1 overflow-hidden rounded-full bg-line" role="progressbar" aria-label="Adding Beebeeb to File Explorer" data-indeterminate>
          <div className="h-full w-1/3 animate-pulse rounded-full bg-ink-3" />
        </div>
        <Buttons><PrimaryButton amber={false} onClick={onContinue}>Continue</PrimaryButton></Buttons>
      </>
    )
  }
  if (status === 'failed') {
    return (
      <>
        <StepHeading icon="info" title="Couldn’t add Beebeeb to File Explorer" sub="Your files are safe, and the Beebeeb window works without File Explorer." />
        <div className="rounded-md border border-red-border bg-red-bg px-3 py-2.5 text-[12px] leading-[17px]" role="alert">
          <div className="font-medium">Windows didn’t finish adding Beebeeb to File Explorer</div>
          <div className="text-ink-2">Try again in a moment.</div>
          {errorCode && <div className="mt-1 font-mono text-[11px] text-ink-2" data-error-code>{errorCode}</div>}
        </div>
        <Buttons>
          <PrimaryButton onClick={onTryAgain}>Try again</PrimaryButton>
          <SecondaryButton onClick={onContinue}>Continue without File Explorer</SecondaryButton>
        </Buttons>
        {showError && <p role="alert" data-show-error className="m-0 text-[12px] leading-[17px] text-red-text">{showError}</p>}
      </>
    )
  }
  return (
    <>
      <StepHeading icon="folder" title="Beebeeb is in File Explorer" sub="Your Beebeeb files show in File Explorer’s navigation pane, and in other apps’ Open and Save As dialogs." />
      <NavPane />
      <Buttons>
        <PrimaryButton onClick={onContinue}>Continue</PrimaryButton>
        <SecondaryButton onClick={onShow}>Show in File Explorer</SecondaryButton>
      </Buttons>
      {showError && <p role="alert" data-show-error className="m-0 text-[12px] leading-[17px] text-red-text">{showError}</p>}
    </>
  )
}

// DW-0207
export function NotificationsStep({ onAllow, onNotNow }: { onAllow: () => void; onNotNow: () => void }) {
  return (
    <>
      <StepHeading icon="bell" title="Stay informed" sub="Get told when sync stops, or when someone signs in to your account. No marketing, ever." />
      <Buttons>
        <PrimaryButton onClick={onAllow}>Allow notifications</PrimaryButton>
        <SecondaryButton onClick={onNotNow}>Not now</SecondaryButton>
      </Buttons>
    </>
  )
}

// DW-0281
export function LoginStep({ on, onToggle, onContinue }: { on: boolean; onToggle: (v: boolean) => void; onContinue: () => void }) {
  return (
    <>
      <StepHeading icon="activity" title="Open Beebeeb when you log in" sub="So the b is in the notification area and your files sync from the start." />
      <div className="flex items-center justify-between border-y border-line py-3">
        <span className="text-[13px] font-medium">Open at login</span>
        <BBToggle on={on} onChange={onToggle} aria-label="Open at login" />
      </div>
      <Buttons><PrimaryButton amber={false} onClick={onContinue}>Continue</PrimaryButton></Buttons>
    </>
  )
}

export interface SetupSummary { passkey: string; lock: string; backup: string; previews: string; explorer: string; notifications: string; login: string }

// DW-0208
export function DoneStep({ summary, onRow, onOpen }: { summary: SetupSummary; onRow: (page: string) => void; onOpen: () => void }) {
  // Each row opens its own settings page (DW-0208#0: App lock opens Settings › Security), M3e.
  const rows: [string, string, string][] = [
    ['Passkey', summary.passkey, 'security'], ['App lock', summary.lock, 'security'], ['Backup', summary.backup, 'backups'],
    ['Previews', summary.previews, 'thumbnails'], ['File Explorer', summary.explorer, 'sync'],
    ['Notifications', summary.notifications, 'notifications'], ['Open at login', summary.login, 'sync'],
  ]
  return (
    <>
      <StepHeading icon="check" title="You’re set up" sub="You can change any of this later in Settings." />
      <ul className="m-0 list-none divide-y divide-line border-y border-line p-0" data-summary>
        {rows.map(([k, v, page]) => (
          <li key={k} data-summary-row>
            <button type="button" onClick={() => onRow(page)} className="flex h-9 w-full items-center justify-between bg-transparent text-left text-[13px] text-ink hover:bg-paper-2">
              <span>{k}</span><span className="flex items-center gap-1 text-ink-2">{v}<Icon name="chevron-right" size={12} className="text-ink-4" /></span>
            </button>
          </li>
        ))}
      </ul>
      <p className="mt-3 text-[12px] text-ink-3">Your file list and thumbnails keep loading in the background.</p>
      <Buttons><PrimaryButton onClick={onOpen}>Open my Beebeeb</PrimaryButton></Buttons>
    </>
  )
}

const COMPARE_SIZES: [string, string][] = [['2K HEIC', '0.4 MB'], ['4K HEIC', '1.0 MB'], ['4K JPEG', '2.6 MB'], ['Original', '30.9 MB']]

// DW-0209 (as DW-0745)
export function CompareView({ mode, onMode, onChoose, onDone }: { mode: 'Stacked' | 'Wipe' | 'Hold to flip'; onMode: (m: 'Stacked' | 'Wipe' | 'Hold to flip') => void; onChoose: () => void; onDone: () => void }) {
  return (
    <>
      <div className="mb-3 flex items-center justify-between">
        <h1 className="m-0 text-[18px] font-semibold">Compare previews</h1>
        <div role="radiogroup" aria-label="Compare mode" className="flex rounded-md border border-line p-0.5">
          {(['Stacked', 'Wipe', 'Hold to flip'] as const).map((m) => (
            <button key={m} type="button" role="radio" aria-checked={mode === m} onClick={() => onMode(m)}
              className={`rounded px-2.5 py-1 text-[12px] ${mode === m ? 'bg-paper-3 font-medium' : 'text-ink-2'}`}><span className="text-[12px]">{m}</span></button>
          ))}
        </div>
      </div>
      <div className="mb-3 grid h-[200px] grid-cols-2 gap-1 overflow-hidden rounded-md" data-compare={mode}>
        <div className="bg-(image:--sample-photo-original)" />
        <div className="bg-(image:--sample-photo-preview) blur-[0.6px]" />
      </div>
      <ul className="m-0 mb-3 list-none divide-y divide-line border-y border-line p-0">
        {COMPARE_SIZES.map(([k, v]) => <li key={k} data-compare-size className="flex h-8 items-center justify-between text-[13px]"><span>{k}</span><span className="font-mono text-[12px] text-ink-2">{v}</span></li>)}
      </ul>
      <p className="mb-3 text-[12px] text-ink-3">Sizes are examples measured on the chosen photo.</p>
      <Buttons>
        <SecondaryButton onClick={onChoose}>Choose photo</SecondaryButton>
        <PrimaryButton amber={false} onClick={onDone}>Done</PrimaryButton>
      </Buttons>
    </>
  )
}
