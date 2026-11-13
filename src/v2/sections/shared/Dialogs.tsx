// Shared's dialogs in the board's words: New request (DW-0504), People (DW-0511), a person's access (DW-0512), the
// remove confirm (DW-0512b), the safety number (DW-0513) and its mismatch (DW-0513b), Invite someone (DW-0514) and a
// changed key (DW-0515). Views only; no key material is ever passed in (the safety number is display digits).
import { useState, type ReactNode } from 'react'
import { Icon } from '@beebeeb/shared/design'
import { BTN, BTN_AMBER, FOCUS, Modal } from '../files/ui'
import { initialsOf, MARK_UNREADABLE, safetyState, type Access, type Member } from './model'

const BTN_RED = `inline-flex h-8 items-center justify-center rounded-md bg-red px-4 text-[13px] font-medium text-white hover:brightness-95 disabled:opacity-40 ${FOCUS}`
const RED_TEXT = 'text-[color:var(--color-red-text)]'
const INPUT = `h-8 w-full rounded-md border border-line bg-paper px-2.5 text-[13px] text-ink placeholder:text-ink-3 ${FOCUS}`

function Field({ label, children, hint }: { label: string; children: ReactNode; hint?: string }) {
  return (
    <label className="mb-3 block">
      <span className="mb-1 block text-[12px] font-medium text-ink-2">{label}</span>
      {children}
      {hint && <span className="mt-1 block text-[12px] text-ink-3">{hint}</span>}
    </label>
  )
}

function MenuRow({ k, value, options, onChange }: { k: string; value: string; options: string[]; onChange?: (v: string) => void }) {
  return (
    <div className="flex min-h-10 items-center justify-between gap-4 border-b border-line last:border-0">
      <span className="text-ink">{k}</span>
      <select aria-label={k} value={value} onChange={(e) => onChange?.(e.target.value)} className={`h-8 rounded-md border border-line bg-paper px-2 text-[13px] text-ink ${FOCUS}`}>
        {options.map((o) => <option key={o}>{o}</option>)}
      </select>
    </div>
  )
}

function Segmented<T extends string>({ label, value, options, onChange }: { label: string; value: T; options: T[]; onChange?: (v: T) => void }) {
  return (
    <div role="radiogroup" aria-label={label} className="inline-flex rounded-md border border-line bg-paper-2 p-0.5">
      {options.map((o) => (
        <button key={o} type="button" role="radio" aria-checked={value === o} onClick={() => onChange?.(o)}
          className={`h-7 rounded px-3 text-[13px] ${value === o ? 'bg-paper text-ink shadow-sm' : 'text-ink-2 hover:text-ink'} ${FOCUS}`}>{o}</button>
      ))}
    </div>
  )
}

function Avatar({ name }: { name: string }) {
  return <span aria-hidden="true" className="grid h-8 w-8 flex-none place-items-center rounded-full bg-paper-3 text-[12px] font-medium text-ink-2">{initialsOf(name)}</span>
}

// ── DW-0504 ──────────────────────────────────────────────────────────────────

export interface NewRequestInput { title: string; note: string; into: string; expires: string; maxFiles: string; maxSize: string }
export const REQUEST_EXPIRY = ['In 7 days', 'In 30 days', 'In 90 days', 'Never']
export const REQUEST_MAX_FILES = ['No limit', '10', '50', '100']
export const REQUEST_MAX_SIZE = ['No limit', '1 GB', '10 GB', '50 GB']

export function NewRequestDialog({ folders, initial, busy, onCreate, onClose }: {
  folders: string[]; initial?: Partial<NewRequestInput>; busy?: boolean; onCreate?: (r: NewRequestInput) => void; onClose?: () => void
}) {
  const [r, setR] = useState<NewRequestInput>({ title: '', note: '', into: folders[0] ?? 'Files', expires: 'In 30 days', maxFiles: 'No limit', maxSize: 'No limit', ...initial })
  const set = (k: keyof NewRequestInput) => (v: string) => setR((x) => ({ ...x, [k]: v }))
  return (
    <Modal title="New request" onClose={onClose} width={480}
      footer={<><span className="flex-1" /><button type="button" className={BTN} onClick={onClose}>Cancel</button><button type="button" className={`${BTN_AMBER} disabled:opacity-40`} disabled={!r.title.trim() || busy} onClick={() => onCreate?.(r)}>Create link</button></>}>
      <Field label="Title"><input aria-label="Title" className={INPUT} placeholder="Send me your signed contract" value={r.title} onChange={(e) => set('title')(e.target.value)} /></Field>
      <Field label="Note (optional)" hint="Shown on the upload page."><textarea aria-label="Note" rows={2} className={`${INPUT} h-auto py-1.5`} value={r.note} onChange={(e) => set('note')(e.target.value)} /></Field>
      <MenuRow k="Files go to" value={r.into} options={folders} onChange={set('into')} />
      <MenuRow k="Expires" value={r.expires} options={REQUEST_EXPIRY} onChange={set('expires')} />
      <MenuRow k="Most files" value={r.maxFiles} options={REQUEST_MAX_FILES} onChange={set('maxFiles')} />
      <MenuRow k="Total size" value={r.maxSize} options={REQUEST_MAX_SIZE} onChange={set('maxSize')} />
      <p className="m-0 mt-3 text-[12px] leading-4 text-ink-3">Anyone with the link can upload. They can’t see other people’s files or anything else in this folder.</p>
    </Modal>
  )
}

// ── DW-0511 ──────────────────────────────────────────────────────────────────

export function memberLine(m: Member): string {
  if (m.owner) return m.you ? 'You · Owner' : 'Owner'
  return m.invited ? `${m.access} · Invited, waiting` : m.access ?? 'Can view'
}

export function PeopleDialog({ name, items, members, onPerson, onInvite, onClose }: {
  name: string; items: number; members: Member[]; onPerson?: (m: Member) => void; onInvite?: () => void; onClose?: () => void
}) {
  return (
    <Modal title={<span>People<span className="block text-[12px] font-normal text-ink-3">{name} · <span className="font-mono">{items}</span> items</span></span>} label="People" onClose={onClose} width={460}
      footer={<><button type="button" className={BTN} onClick={onInvite}>Invite someone…</button><span className="flex-1" /><button type="button" className={BTN} onClick={onClose}>Done</button></>}>
      <ul aria-label="People with access" data-people={members.length} className="m-0 -mx-2 list-none p-0">
        {members.map((m) => (
          <li key={m.id}>
            <button type="button" disabled={m.you} onClick={() => onPerson?.(m)} data-person={m.id}
              className={`flex w-full items-center gap-3 rounded-md px-2 py-2 text-left enabled:hover:bg-paper-2 ${FOCUS}`}>
              <Avatar name={m.name} />
              <span className="min-w-0 flex-1"><span className="block truncate text-[13px] text-ink">{m.name}</span><span className="block truncate text-[12px] text-ink-3">{memberLine(m)}</span></span>
              {!m.you && <Icon name="chevron-right" size={12} />}
            </button>
          </li>
        ))}
      </ul>
      <p className="m-0 mt-2 text-[12px] leading-4 text-ink-3">Everyone listed can open what’s shared here.</p>
    </Modal>
  )
}

// ── DW-0512 ──────────────────────────────────────────────────────────────────

export function PersonDialog({ member, onAccess, onSafety, onRemove, onBack }: {
  member: Member; onAccess?: (a: Access) => void; onSafety?: () => void; onRemove?: () => void; onBack?: () => void
}) {
  const first = member.name.split(' ')[0]
  return (
    <Modal label={member.name} onClose={onBack} width={460}
      title={<span className="flex items-center gap-2"><button type="button" aria-label="Back" onClick={onBack} className={`grid h-7 w-7 place-items-center rounded-md text-ink-2 hover:bg-paper-2 ${FOCUS}`}><span className="inline-flex rotate-180"><Icon name="chevron-right" size={12} /></span></button>{member.name}</span>}
      footer={<><button type="button" className={`${BTN.replace(" text-ink ", " ")} ${RED_TEXT}`} onClick={onRemove}>Remove access</button><span className="flex-1" /><button type="button" className={BTN} onClick={onBack}>Done</button></>}>
      <div className="flex min-h-10 items-center justify-between border-b border-line"><span className="text-ink">Email</span><span className="font-mono text-[12px] text-ink-2">{member.email}</span></div>
      <div className="flex min-h-12 items-center justify-between border-b border-line"><span className="text-ink">Access</span><Segmented<Access> label="Access" value={member.access ?? 'Can view'} options={['Can view', 'Can edit']} onChange={onAccess} /></div>
      <div className="pt-3 text-[12px] font-semibold text-ink-2">Identity</div>
      <button type="button" onClick={onSafety} data-safety className={`-mx-2 flex w-[calc(100%+16px)] items-center justify-between rounded-md px-2 py-2 text-left hover:bg-paper-2 ${FOCUS}`}>
        <span className="text-ink">Safety number</span>
        <span className="inline-flex items-center gap-1.5 text-[12px] text-ink-3">
          <span>{safetyState(member)}{member.markUnreadable && <> · <span data-compare-again className="text-ink-2">Compare again</span></>}</span>
          <Icon name="chevron-right" size={12} />
        </span>
      </button>
      <p className="m-0 mt-1 text-[12px] leading-4 text-ink-3">{member.markUnreadable ? MARK_UNREADABLE : `Optional. Comparing proves your files go to the real ${first}.`}</p>
    </Modal>
  )
}

// ── DW-0512b ─────────────────────────────────────────────────────────────────

export function RemoveAccessAlert({ member, folder, busy, onRemove, onCancel }: { member: Member; folder: string; busy?: boolean; onRemove?: () => void; onCancel?: () => void }) {
  const first = member.name.split(' ')[0]
  return (
    <Modal title={`Remove ${first}’s access?`} onClose={onCancel} width={420}
      footer={<><span className="flex-1" /><button type="button" className={BTN} onClick={onCancel}>Cancel</button><button type="button" className={BTN_RED} disabled={busy} onClick={onRemove}>Remove access</button></>}>
      <p className="m-0">{first} can’t open anything new in {folder}, and the shared folder goes from their list. Copies they already saved stay with them.</p>
    </Modal>
  )
}

// ── DW-0513, DW-0513b ────────────────────────────────────────────────────────

function Groups({ groups }: { groups: string[] }) {
  return (
    <div role="group" aria-label="Safety number" data-groups={groups.length} className="my-3 grid grid-cols-4 gap-x-4 gap-y-2 rounded-lg border border-line bg-paper-2 px-4 py-3 font-mono text-[15px] tracking-wider text-ink">
      {groups.map((g, i) => <span key={i} className="text-center">{g}</span>)}
    </div>
  )
}

export function SafetyNumberDialog({ member, groups, busy, onMatch, onNoMatch, onClose }: { member: Member; groups: string[]; busy?: boolean; onMatch?: () => void; onNoMatch?: () => void; onClose?: () => void }) {
  const first = member.name.split(' ')[0]
  return (
    <Modal title="Safety number" onClose={busy ? undefined : onClose} width={480}
      footer={<><span className="flex-1" /><button type="button" className={BTN} disabled={busy} onClick={onNoMatch}>They don’t match</button><button type="button" className={BTN} disabled={busy} onClick={onMatch}>They match</button></>}>
      <p className="m-0">Compare this with {first}, in person or on a call. It should be exactly the same on both devices.</p>
      <Groups groups={groups} />
      <p className="m-0 text-[12px] leading-4 text-ink-3">Optional. Never compare over a channel someone else could change, such as the same email. Beebeeb never sees this comparison.</p>
    </Modal>
  )
}

export function MismatchDialog({ member, groups, busy, onRemove, onCompare, onClose }: { member: Member; groups: string[]; busy?: boolean; onRemove?: () => void; onCompare?: () => void; onClose?: () => void }) {
  const first = member.name.split(' ')[0]
  return (
    <Modal label="Sharing stopped" onClose={busy ? undefined : onClose} width={500}
      title={<span className="flex items-center gap-3"><span aria-hidden="true" className={`grid h-9 w-9 place-items-center rounded-lg border border-line bg-paper-2 ${RED_TEXT}`}><Icon name="shield" size={18} /></span><span>Sharing with {first} is stopped<span className="block text-[12px] font-normal text-ink-2">The safety numbers didn’t match.</span></span></span>}
      footer={<><button type="button" className={`${BTN.replace(" text-ink ", " ")} ${RED_TEXT}`} disabled={busy} onClick={onRemove}>Remove {first}’s access</button><span className="flex-1" /><button type="button" className={BTN_AMBER} disabled={busy} onClick={onCompare}>Compare again</button></>}>
      <div className="grid grid-cols-2 gap-3">
        <div className="rounded-lg border border-line px-3 py-2"><div className="text-[12px] font-semibold text-ink">What stops</div><div className="text-[12px] text-ink-2">New files, invites and links to {first}.</div></div>
        <div className="rounded-lg border border-line px-3 py-2"><div className="text-[12px] font-semibold text-ink">What stays</div><div className="text-[12px] text-ink-2">What they already have.</div></div>
      </div>
      <Groups groups={groups} />
      <p className="m-0 text-[12px] leading-4 text-ink-3">{first} may have a new device or account; Beebeeb can’t tell which. Nothing new is encrypted to the unmatched key.</p>
    </Modal>
  )
}

// ── DW-0514 ──────────────────────────────────────────────────────────────────

export function InviteDialog({ email: initialEmail = '', access: initialAccess = 'Can view', busy, error, onSend, onClose }: {
  email?: string; access?: Access; busy?: boolean; error?: string | null; onSend?: (email: string, access: Access) => void; onClose?: () => void
}) {
  const [email, setEmail] = useState(initialEmail)
  const [access, setAccess] = useState<Access>(initialAccess)
  const ok = /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email.trim())
  return (
    <Modal title="Invite someone" onClose={onClose} width={460}
      footer={<><span className="flex-1" /><button type="button" className={BTN} onClick={onClose}>Cancel</button><button type="button" className={`${BTN_AMBER} disabled:opacity-40`} disabled={!ok || busy} onClick={() => onSend?.(email.trim(), access)}>Send invite</button></>}>
      <Field label="Email"><input type="email" aria-label="Email" className={INPUT} value={email} onChange={(e) => setEmail(e.target.value)} spellCheck={false} /></Field>
      <div className="mb-3 flex items-center justify-between"><span className="text-[12px] font-medium text-ink-2">Access</span><Segmented<Access> label="Access" value={access} options={['Can view', 'Can edit']} onChange={setAccess} /></div>
      {error && <p role="alert" className={`m-0 mb-2 text-[12px] ${RED_TEXT}`}>{error}</p>}
      <p className="m-0 flex gap-2 rounded-lg border border-line bg-paper-2 px-3 py-2 text-[12px] leading-4 text-ink-2"><span className="mt-px flex-none text-ink-3"><Icon name="key" size={13} /></span>The first time you share with someone, Beebeeb supplies their key, so you’re trusting Beebeeb for that step. You can compare safety numbers afterwards.</p>
    </Modal>
  )
}

// ── DW-0515 ──────────────────────────────────────────────────────────────────

export function KeyChangedAlert({ member, busy, onCompare, onShareAnyway, onCancel }: { member: Member; busy?: boolean; onCompare?: () => void; onShareAnyway?: () => void; onCancel?: () => void }) {
  const first = member.name.split(' ')[0]
  return (
    <Modal title={`${first}’s key has changed`} onClose={busy ? undefined : onCancel} width={520}
      footer={<><button type="button" className={BTN} disabled={busy} onClick={onShareAnyway}>Share anyway</button><span className="flex-1" /><button type="button" className={BTN} disabled={busy} onClick={onCancel}>Cancel</button><button type="button" className={BTN} disabled={busy} onClick={onCompare}>Compare safety numbers</button></>}>
      <p className="m-0">{first} may have set up a new account, or someone may be trying to read what you share. Nothing new goes to {first} until you choose.</p>
    </Modal>
  )
}

// ── Leave (DW-0505) and a link's password (DW-0506) ──────────────────────────

export function LeaveAlert({ name, onLeave, onCancel }: { name: string; onLeave?: () => void; onCancel?: () => void }) {
  return (
    <Modal title={`Leave ${name}?`} onClose={onCancel} width={420}
      footer={<><span className="flex-1" /><button type="button" className={BTN} onClick={onCancel}>Cancel</button><button type="button" className={BTN_RED} onClick={onLeave}>Leave</button></>}>
      <p className="m-0">It goes from your list. The owner keeps it.</p>
    </Modal>
  )
}

export function LinkPasswordDialog({ name, hasPassword, onSave, onClose }: { name: string; hasPassword: boolean; onSave?: (password: string | null) => void; onClose?: () => void }) {
  const [pw, setPw] = useState('')
  return (
    <Modal title="Password" onClose={onClose} width={420}
      footer={<>{hasPassword && <button type="button" className={BTN} onClick={() => onSave?.(null)}>Remove password</button>}<span className="flex-1" /><button type="button" className={BTN} onClick={onClose}>Cancel</button><button type="button" className={`${BTN_AMBER} disabled:opacity-40`} disabled={!pw} onClick={() => { onSave?.(pw); setPw('') }}>Save</button></>}>
      <p className="m-0 mb-2">Anyone opening the link to {name} needs this password too.</p>
      <input type="password" aria-label="Link password" autoComplete="new-password" className={INPUT} value={pw} onChange={(e) => setPw(e.target.value)} />
    </Modal>
  )
}
