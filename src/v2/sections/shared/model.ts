// Shared's view models and copy helpers (DW-0501 to DW-0515). No data source here: the live section and the frames
// hand the views these shapes.
import type { ShareEntry } from '@beebeeb/shared/drive-ui'
import type { MenuEntry } from '../files/ui'
import type { FileRequestRow, LinkShare, Member, PeopleShare, WithMeRow } from '../../source/fixtures/shared'

export type { FileRequestRow, Invitation, LinkShare, PeopleShare, WithMeRow, Member, Access } from '../../source/fixtures/shared'
export type SharedTab = 'with-me' | 'by-me' | 'requests'

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']
const dayMonth = (d: Date) => `${d.getDate()} ${MONTHS[d.getMonth()]}`
const daysBetween = (a: Date, b: Date) => Math.round((Date.UTC(b.getFullYear(), b.getMonth(), b.getDate()) - Date.UTC(a.getFullYear(), a.getMonth(), a.getDate())) / 86_400_000)

export function isPast(iso: string | null | undefined, now: Date): boolean {
  return !!iso && new Date(iso).getTime() <= now.getTime()
}

/** A link's expiry: "Expires in 12 days", "Expires tomorrow", "Expired 3 Oct", "No expiry". */
export function linkExpiry(iso: string | null, now: Date): string {
  if (!iso) return 'No expiry'
  const d = new Date(iso)
  if (d.getTime() <= now.getTime()) return `Expired ${dayMonth(d)}`
  const n = daysBetween(now, d)
  return n <= 1 ? 'Expires tomorrow' : `Expires in ${n} days`
}

/** A request's line: "3 files · Into Documents", "12 of 50 files · Expires 14 Nov", "5 files · Closed", "No files · Expired 3 Oct". */
export function requestLine(r: FileRequestRow, now: Date): { files: string; state: string | null; open: boolean } {
  const files = r.files === 0 ? 'No files' : r.maxFiles ? `${r.files} of ${r.maxFiles} files` : `${r.files} ${r.files === 1 ? 'file' : 'files'}`
  if (r.closed) return { files, state: 'Closed', open: false }
  if (isPast(r.expiresAt, now)) return { files, state: `Expired ${dayMonth(new Date(r.expiresAt!))}`, open: false }
  if (r.expiresAt) return { files, state: `Expires ${dayMonth(new Date(r.expiresAt))}`, open: true }
  return { files, state: r.into ? `Into ${r.into}` : null, open: true }
}

/** The safety number's state on a person (DW-0512). A comparison this PC cannot read is said, never shown as none. */
export function safetyState(m: Pick<Member, 'compared' | 'mismatch' | 'markUnreadable'>): string {
  if (m.markUnreadable) return 'Couldn’t be read'
  return m.mismatch ? 'Didn’t match' : m.compared ? 'Compared' : 'Not compared'
}

export const peopleLabel = (people: string[]) => (people.length <= 2 ? people.join(', ') : `${people[0]}, ${people[1]} and ${people.length - 2} more`)
export const initialsOf = (name: string) => name.split(/\s+/).filter(Boolean).map((p) => p[0]).filter((c) => c === c.toUpperCase()).slice(0, 2).join('')

// ── Menus (DW-0505, DW-0506, DW-0507) ────────────────────────────────────────

export function withMeMenu(): MenuEntry[] {
  return [
    { id: 'open', label: 'Open' },
    { id: 'preview', label: 'Preview', shortcut: 'Space' },
    { id: 'offline', label: 'Make available offline' },
    { id: 'reveal', label: 'Show in File Explorer' },
    { id: 'details', label: 'Details', shortcut: 'Alt+Enter', dividerAfter: true },
    { id: 'leave', label: 'Leave…' },
  ]
}

export const EXPIRY_CHOICES = [
  { id: 'expiry-1', label: '1 day', days: 1 },
  { id: 'expiry-7', label: '7 days', days: 7 },
  { id: 'expiry-30', label: '30 days', days: 30 },
  { id: 'expiry-never', label: 'Never', days: null },
] as const

/** A link Anna made; a share with people offers People and Stop sharing instead of the link items. */
export function byMeMenu(kind: 'link' | 'people', expiryOpen = false): MenuEntry[] {
  if (kind === 'people') return [{ id: 'people', label: 'People', dividerAfter: true }, { id: 'stop', label: 'Stop sharing' }]
  return [
    { id: 'copy', label: 'Copy link' },
    { id: 'expiry', label: 'Change expiry', submenu: EXPIRY_CHOICES.map((c) => ({ id: c.id, label: c.label })), open: expiryOpen },
    { id: 'password', label: 'Password…' },
    { id: 'open', label: 'Open' },
    { id: 'people', label: 'People', dividerAfter: true },
    { id: 'stop', label: 'Stop sharing' },
  ]
}

/** An open request; a closed or expired one only offers View files and Remove. */
export function requestMenu(open: boolean): MenuEntry[] {
  if (!open) return [{ id: 'view', label: 'View files', dividerAfter: true }, { id: 'remove', label: 'Remove' }]
  return [
    { id: 'copy', label: 'Copy link' },
    { id: 'view', label: 'View files' },
    { id: 'expiry', label: 'Change expiry', submenu: EXPIRY_CHOICES.map((c) => ({ id: c.id, label: c.label })), dividerAfter: true },
    { id: 'close', label: 'Close request' },
  ]
}

// ── From DriveSource (live) ──────────────────────────────────────────────────

/** The rows DriveSource.shared() can fill. Invitations, link expiry, pending and requests come from share_list. */
export function rowsFromEntries(entries: ShareEntry[]): { withMe: WithMeRow[]; people: PeopleShare[]; links: LinkShare[] } {
  const withMe: WithMeRow[] = []
  const people: PeopleShare[] = []
  const links: LinkShare[] = []
  for (const e of entries) {
    const base = { id: e.item.id, name: e.item.name, isFolder: e.item.isFolder }
    if (e.direction === 'with-me') withMe.push({ ...base, owner: e.people[0]?.name ?? '', items: e.item.childCount, size: e.item.size, modifiedAt: e.item.modifiedAt })
    else if (e.hasLink) links.push({ id: base.id, name: base.name, expiresAt: null, password: false, size: e.item.size, isFolder: base.isFolder })
    if (e.direction === 'by-me' && e.people.length > 0) people.push({ ...base, people: e.people.map((p) => p.name), items: e.item.childCount, size: e.item.size })
  }
  return { withMe, people, links }
}
