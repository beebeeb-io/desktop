// Shared, live: DriveSource.shared() for the lists it can fill; share_list (Rust) for invitations, link expiry,
// pending and file requests; the share_* and file_request_* commands for every action (sections/shared/api.ts).
// A command this build lacks shows the honest "isn't available" toast, never invented data.
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { useDriveSource, type DriveItem, type ItemDetails, type ShareEntry, type VersionEntry } from '@beebeeb/shared/drive-ui'
import type { SectionProps } from '../types'
import { showToast } from '../../chrome/Toast'
import { BannerSlot } from '../../chrome/Banners'
import { BTN, BTN_AMBER, Menu } from '../files/ui'
import { fmtWhen } from '../files/model'
import { DetailsDialog, VersionsDialog } from '../files/Dialogs'
import {
  InviteDialog, KeyChangedAlert, LeaveAlert, LinkPasswordDialog, MismatchDialog, NewRequestDialog, PeopleDialog, PersonDialog, RemoveAccessAlert, SafetyNumberDialog,
  type NewRequestInput,
} from './Dialogs'
import { afterMarkCompared, byMeMenu, EXPIRY_CHOICES, linkExpiry, requestLine, requestMenu, withMeMenu, type Member } from './model'
import * as api from './api'
import { seed } from '../../seed'
import type { ShareLists } from './api'

type RowKind = 'with-me' | 'people' | 'link' | 'request'

type Dialog =
  | { kind: 'new-request' }
  | { kind: 'leave'; itemId: string }
  | { kind: 'password'; itemId: string }
  | { kind: 'people'; itemId: string }
  | { kind: 'person'; itemId: string; member: Member }
  | { kind: 'remove'; itemId: string; member: Member }
  // `refused`: the comparison is for an invite a changed key stopped (DW-0515); They match then seals it.
  | { kind: 'safety'; itemId: string; member: Member; groups: string[]; refused?: { email: string; canEdit: boolean } }
  | { kind: 'mismatch'; itemId: string; member: Member; groups: string[]; refused?: { email: string; canEdit: boolean } }
  | { kind: 'invite'; itemId: string }
  | { kind: 'key-changed'; itemId: string; member: Member; email: string; canEdit: boolean }
  | { kind: 'mark-changed'; itemId: string; member: Member }
  | { kind: 'details'; details: ItemDetails }
  | { kind: 'versions'; details: ItemDetails; versions: VersionEntry[] }

type WithYou = { id: string; name: string; isFolder: boolean; owner: string; when: string }
type ByYou = { id: string; name: string; isFolder: boolean; people: string; when: string }
type Classified = {
  withYou: WithYou[]
  removed: { id: string; name: string }[]
  byYou: ByYou[]
  folder: Record<string, boolean>
  name: Record<string, string>
  items: Record<string, number>
}

const EMPTY_LISTS: ShareLists = { invitations: [], links: [], requests: [], pending: [] }
const EMPTY_SHARES: Classified = { withYou: [], removed: [], byYou: [], folder: {}, name: {}, items: {} }
const days = (label: string) => (label === 'Never' ? null : Number(label.replace(/\D/g, '')) || null)
const bytes = (label: string) => (label === 'No limit' ? null : Number(label.replace(/\D/g, '')) * 1024 ** 3)

/** Names on the item's own shared state. The share entry's people list is not a source. */
function peopleNames(item: DriveItem): string[] {
  if (item.shared.kind !== 'people') return []
  return item.shared.people.map((p) => p.name).filter((n) => n.trim() !== '')
}

/**
 * With you and by you come only from `item.shared`. A with-me item that is no longer shared with people had its
 * access removed. A link on the mirror (`kind: 'link'` or `hasLink`) is not a row: share_list lists link shares.
 */
function classifyShares(entries: ShareEntry[]): Classified {
  const withYou: WithYou[] = []
  const removed: { id: string; name: string }[] = []
  const byYou: ByYou[] = []
  const folder: Record<string, boolean> = {}
  const name: Record<string, string> = {}
  const items: Record<string, number> = {}
  for (const e of entries) {
    folder[e.item.id] = e.item.isFolder
    name[e.item.id] = e.item.name
    if (typeof e.item.childCount === 'number') items[e.item.id] = e.item.childCount
    const names = peopleNames(e.item)
    const live = e.item.shared.kind === 'people' && names.length > 0
    if (e.direction === 'with-me') {
      if (live) withYou.push({ id: e.item.id, name: e.item.name, isFolder: e.item.isFolder, owner: names.join(', '), when: e.item.modifiedAt })
      else if (e.item.shared.kind !== 'link') removed.push({ id: e.item.id, name: e.item.name })
    } else if (e.direction === 'by-me' && live) {
      byYou.push({ id: e.item.id, name: e.item.name, isFolder: e.item.isFolder, people: names.join(', '), when: e.item.modifiedAt })
    }
  }
  return { withYou, removed, byYou, folder, name, items }
}

function Heading({ children }: { children: ReactNode }) {
  return <h2 className="m-0 flex h-9 items-end px-4 pb-1.5 text-[12px] font-semibold text-ink-2">{children}</h2>
}

function EmptyAccount() {
  return (
    <div data-empty="shared" className="flex flex-1 flex-col items-center justify-center gap-2 px-8 text-center">
      <h2 className="m-0 text-[15px] font-semibold text-ink">Nothing shared yet</h2>
      <p className="m-0 max-w-[420px] text-[13px] leading-5 text-ink-2">When someone shares a file or folder with you, it shows here with their name and the date.</p>
      <p className="m-0 max-w-[420px] text-[13px] leading-5 text-ink-2">What you share with people, or by a link, shows here as well.</p>
    </div>
  )
}

function toast(r: { ok: boolean }, done?: string) {
  if (r.ok) { if (done) showToast({ kind: 'done', text: done }) } else showToast({ kind: 'failed', text: api.failureText(r as never) })
}

export default function Section({ nav }: SectionProps) {
  const source = useDriveSource()
  const [shares, setShares] = useState<Classified>(EMPTY_SHARES)
  const [lists, setLists] = useState<ShareLists>(EMPTY_LISTS)
  const [linksUnlisted, setLinksUnlisted] = useState(false)
  const [members, setMembers] = useState<Member[]>(() => seed('shared.members', []))
  const [menu, setMenu] = useState<{ kind: RowKind; id: string; x: number; y: number } | null>(null)
  const [dialog, setDialog] = useState<Dialog | null>(() => seed('shared.dialog', null))
  const [busy, setBusy] = useState(false)
  // A seal (They match / They don't match / Share anyway) runs once: a second click or a way out while it is in
  // flight is ignored, and the dialog moves only when it settles (2440 item 1).
  const inFlight = useRef(false)
  const dismissed = useRef(new Set<string>())
  const now = useMemo(() => new Date(), [])

  const load = useCallback(async () => {
    const entries = await source.shared().catch(() => [])
    setShares(classifyShares(entries))
    const l = await api.shareList()
    if (l.ok) {
      setLists(l.value)
      setLinksUnlisted(false)
    } else {
      // Link shares are the share command's list. A miss stays on screen as one line; the mirror is not a stand-in.
      setLinksUnlisted(true)
      setLists((cur) => ({ ...cur, links: [] }))
    }
  }, [source])
  useEffect(() => {
    void load()
    return source.subscribe((e) => { if (e.kind === 'changed') void load() })
  }, [source, load])

  const nameOf = (id: string) => shares.name[id] ?? lists.links.find((l) => l.id === id)?.name ?? lists.requests.find((r) => r.id === id)?.title ?? ''
  const isFolder = (id: string) => shares.folder[id] ?? lists.links.find((l) => l.id === id)?.isFolder ?? false
  const empty = shares.withYou.length === 0 && shares.removed.length === 0 && shares.byYou.length === 0
    && lists.links.length === 0 && !linksUnlisted && lists.invitations.length === 0 && lists.requests.length === 0
  const menuAt = (kind: RowKind, id: string, x: number, y: number) => setMenu({ kind, id, x, y })

  // DW-0501 Rules: shared folders and files open as in Files. A folder opens in Files; a file opens in its app.
  const openItem = (id: string) => {
    if (isFolder(id)) return nav.go({ section: 'files', id })
    void source.open(id).catch((e: unknown) => showToast({ kind: 'failed', text: api.openFailureText(e, 'open it') }))
  }
  // DW-0505 Details: DW-0411's dialog over Shared, from the list on this PC.
  const showDetails = (id: string) => {
    source.details(id).then((details) => setDialog({ kind: 'details', details }), (e: unknown) => showToast({ kind: 'failed', text: api.openFailureText(e, 'show the details') }))
  }
  const showVersions = (details: ItemDetails) => {
    source.versions(details.item.id).then((versions) => setDialog({ kind: 'versions', details, versions }), (e: unknown) => showToast({ kind: 'failed', text: api.openFailureText(e, 'open Version history') }))
  }

  const openPeople = async (itemId: string) => {
    const r = await api.sharePeople(itemId)
    if (!r.ok) return toast(r)
    setMembers(r.value)
    // DW-0515: a remembered key no longer matches; nothing new goes to them until the owner chooses.
    const changed = r.value.find((m) => m.keyChanged && !dismissed.current.has(m.id))
    setDialog(changed ? { kind: 'mark-changed', itemId, member: changed } : { kind: 'people', itemId })
  }

  const pick = async (id: string) => {
    if (!menu) return
    const { kind, id: rowId } = menu
    const choice = EXPIRY_CHOICES.find((c) => c.id === id)
    if (kind === 'with-me') {
      if (id === 'open' || id === 'preview') openItem(rowId)
      // DW-0412's words: the download starts now, so it is "Keeping", not "Kept".
      else if (id === 'offline') await source.setAvailableOffline(rowId, true).then(() => showToast({ kind: 'done', text: 'Keeping on this PC' }), (e: unknown) => showToast({ kind: 'failed', text: api.openFailureText(e, 'keep it on this PC') }))
      else if (id === 'reveal') await source.revealInFileManager(rowId).catch((e: unknown) => showToast({ kind: 'failed', text: api.openFailureText(e, 'show it in File Explorer') }))
      else if (id === 'details') showDetails(rowId)
      else if (id === 'leave') setDialog({ kind: 'leave', itemId: rowId })
    } else if (kind === 'link' || kind === 'people') {
      if (id === 'copy') { const r = await api.shareLinkCopy(rowId); toast(r, r.ok ? `Link copied: ${r.value.display}` : undefined) }
      else if (choice) { toast(await api.shareLinkUpdate(rowId, { expiresDays: choice.days }), 'Expiry changed'); void load() }
      else if (id === 'password') setDialog({ kind: 'password', itemId: rowId })
      else if (id === 'open') openItem(rowId)
      else if (id === 'people') await openPeople(rowId)
      else if (id === 'stop') { toast(await api.shareRevoke(rowId, null), 'Stopped sharing'); void load() }
    } else if (kind === 'request') {
      if (id === 'copy') { const r = await api.fileRequestCopy(rowId); toast(r, r.ok ? `Link copied: ${r.value.display}` : undefined) }
      else if (id === 'view') { const rq = lists.requests.find((r) => r.id === rowId); nav.go({ section: 'files', id: null, sub: rq?.into ?? null }) }
      else if (choice) { toast(await api.fileRequestUpdate(rowId, { expiresDays: choice.days }), 'Expiry changed'); void load() }
      else if (id === 'close') { toast(await api.fileRequestUpdate(rowId, { close: true }), 'Request closed'); void load() }
      else if (id === 'remove') { toast(await api.fileRequestRemove(rowId), 'Removed'); void load() }
    }
  }

  const entries = menu
    ? menu.kind === 'with-me' ? withMeMenu()
      : menu.kind === 'request' ? requestMenu(requestLine(lists.requests.find((r) => r.id === menu.id) ?? { id: '', title: '', files: 0 }, now).open)
        : byMeMenu(menu.kind === 'link' ? 'link' : 'people')
    : []

  const createRequest = async (r: NewRequestInput) => {
    setBusy(true)
    const res = await api.fileRequestCreate({ title: r.title.trim(), note: r.note.trim(), folderName: r.into, expiresDays: days(r.expires), maxFiles: r.maxFiles === 'No limit' ? null : Number(r.maxFiles), maxBytes: bytes(r.maxSize) })
    setBusy(false)
    toast(res, res.ok ? `Link copied: ${res.value.display}` : undefined)
    if (res.ok) { setDialog(null); void load() }
  }

  const invite = async (itemId: string, email: string, canEdit: boolean, acceptChangedKey = false, memberId: string | null = null) => {
    // A re-entrant call (a second Share anyway, or the refused-invite seal's own invite) runs nothing.
    if (inFlight.current) return
    inFlight.current = true
    setBusy(true)
    try {
      const r = await api.shareInvite(itemId, email, canEdit ? 'Can edit' : 'Can view', acceptChangedKey, memberId)
      if (!r.ok) {
        toast(r)
        // A folder's Share anyway: Rust withdrew the refused invite, so the dialog about it closes and People reloads.
        if (acceptChangedKey && /sharing a folder to a changed key/.test(r.reason)) await openPeople(itemId)
        return
      }
      if (r.value.status === 'key-changed') {
        const member = members.find((m) => m.id === (r.value as { memberId: string }).memberId) ?? { id: r.value.memberId, name: email, email }
        setDialog({ kind: 'key-changed', itemId, member, email, canEdit })
        return
      }
      await openPeople(itemId)
    } finally {
      inFlight.current = false
      setBusy(false)
    }
  }

  const safety = async (itemId: string, member: Member, refused?: { email: string; canEdit: boolean }) => {
    const r = await api.shareSafetyNumber(member.id)
    if (!r.ok) return toast(r)
    setDialog({ kind: 'safety', itemId, member, groups: r.value.groups, refused })
  }

  // DW-0515 Cancel on a refused seal: "Shares nothing". The refused invite holds no key; it is withdrawn so it never
  // waits on the other side as an invitation that cannot open.
  const withdraw = async (itemId: string, member: Member) => {
    const r = await api.shareRevoke(itemId, member.id)
    if (!r.ok) toast(r)
    await openPeople(itemId)
  }

  const onAccept = async (inv: { id: string; name: string }) => { toast(await api.shareAccept(inv.id), `${inv.name} added`); void load() }
  const onDecline = async (inv: { id: string }) => { toast(await api.shareDecline(inv.id)); void load() }
  const showWithYou = shares.withYou.length > 0 || shares.removed.length > 0
  const showByYou = shares.byYou.length > 0 || lists.links.length > 0 || linksUnlisted
  const d = dialog
  return (
    <>
      <section aria-label="Shared" className="flex min-h-0 min-w-0 flex-1 flex-col">
        <div role="toolbar" aria-label="Shared" className="flex h-12 flex-none items-center gap-3 border-b border-line px-4">
          <h1 className="m-0 text-[15px] font-semibold text-ink">Shared</h1>
          <span className="flex-1" />
          {!empty && <button type="button" data-new-request className={BTN_AMBER} onClick={() => setDialog({ kind: 'new-request' })}>New request</button>}
        </div>
        <BannerSlot />
        {empty ? <EmptyAccount /> : (
          <div className="min-h-0 flex-1 overflow-auto pb-4">
            {lists.invitations.map((inv) => (
              <div key={inv.id} data-invitation={inv.id} className="mx-4 mt-3 flex items-center gap-3 rounded-lg border border-line bg-paper-2 px-4 py-3">
                <span className="min-w-0 flex-1">
                  <span className="block text-[13px] font-medium text-ink">{inv.name}</span>
                  <span className="block text-[12px] text-ink-2">{inv.from} wants to share this {inv.isFolder ? 'folder' : 'file'}</span>
                </span>
                <button type="button" className={BTN} onClick={() => void onDecline(inv)}>Decline</button>
                <button type="button" className={BTN} onClick={() => void onAccept(inv)}>Accept</button>
              </div>
            ))}
            {showWithYou && (
              <section aria-label="Shared with you">
                <Heading>Shared with you</Heading>
                {shares.withYou.map((r) => (
                  <div key={r.id} role="row" data-row={r.id} data-kind="with-me" data-open="yes" tabIndex={-1}
                    onDoubleClick={() => openItem(r.id)} onKeyDown={(e) => { if (e.key === 'Enter') openItem(r.id) }}
                    onContextMenu={(e) => { e.preventDefault(); menuAt('with-me', r.id, e.clientX, e.clientY) }}
                    className="grid h-11 grid-cols-[minmax(0,2.2fr)_minmax(0,1.6fr)_minmax(0,1.1fr)_96px] items-center gap-3 px-4 text-[13px]">
                    <span className="truncate text-ink">{r.name}</span>
                    <span className="truncate text-ink-2">{r.owner}</span>
                    <span className="font-mono text-[12px] text-ink-3">{fmtWhen(r.when, now)}</span>
                    <button type="button" className={BTN} onClick={() => openItem(r.id)}>Open</button>
                  </div>
                ))}
                {shares.removed.map((r) => (
                  <div key={r.id} role="row" data-row={r.id} data-removed="true" className="grid h-11 grid-cols-[minmax(0,2.2fr)_minmax(0,1.6fr)] items-center gap-3 px-4 text-[13px]">
                    <span className="truncate text-ink">{r.name}</span>
                    <span className="text-ink-2">No longer shared</span>
                  </div>
                ))}
              </section>
            )}
            {showByYou && (
              <section aria-label="Shared by you">
                <Heading>Shared by you</Heading>
                {shares.byYou.map((r) => (
                  <div key={r.id} role="row" data-row={r.id} data-kind="people" data-open="yes" tabIndex={-1}
                    onDoubleClick={() => openItem(r.id)} onKeyDown={(e) => { if (e.key === 'Enter') openItem(r.id) }}
                    onContextMenu={(e) => { e.preventDefault(); menuAt('people', r.id, e.clientX, e.clientY) }}
                    className="grid h-11 grid-cols-[minmax(0,2.2fr)_minmax(0,1.6fr)_minmax(0,1.1fr)_96px] items-center gap-3 px-4 text-[13px]">
                    <span className="truncate text-ink">{r.name}</span>
                    <span className="truncate text-ink-2">{r.people}{lists.pending.includes(r.id) ? ' · Pending' : ''}</span>
                    <span className="font-mono text-[12px] text-ink-3">{fmtWhen(r.when, now)}</span>
                    <button type="button" className={BTN} onClick={() => openItem(r.id)}>Open</button>
                  </div>
                ))}
                {lists.links.map((r) => (
                  <div key={r.id} role="row" data-row={r.id} data-kind="link" data-open="yes" tabIndex={-1}
                    onDoubleClick={() => openItem(r.id)} onContextMenu={(e) => { e.preventDefault(); menuAt('link', r.id, e.clientX, e.clientY) }}
                    className="grid h-11 grid-cols-[minmax(0,2.2fr)_minmax(0,1.6fr)_96px] items-center gap-3 px-4 text-[13px]">
                    <span className="truncate text-ink">{r.name}</span>
                    <span className="truncate text-[12px] text-ink-2">{linkExpiry(r.expiresAt, now)}</span>
                    <button type="button" className={BTN} onClick={() => openItem(r.id)}>Open</button>
                  </div>
                ))}
                {linksUnlisted && <p className="m-0 px-4 py-2 text-[13px] text-ink-2">Link shares can’t be listed yet.</p>}
              </section>
            )}
            {lists.requests.length > 0 && (
              <section aria-label="File requests">
                <Heading>File requests</Heading>
                {lists.requests.map((r) => {
                  const line = requestLine(r, now)
                  return (
                    <div key={r.id} role="row" data-row={r.id} data-kind="request" tabIndex={-1}
                      onContextMenu={(e) => { e.preventDefault(); menuAt('request', r.id, e.clientX, e.clientY) }}
                      className="grid h-11 grid-cols-[minmax(0,2.2fr)_minmax(0,1.6fr)_minmax(0,1.1fr)] items-center gap-3 px-4 text-[13px]">
                      <span className="truncate text-ink">{r.title}</span>
                      <span className="font-mono text-[12px] text-ink-3">{line.files}</span>
                      <span className="text-[12px] text-ink-2">{line.state ?? ''}</span>
                    </div>
                  )
                })}
              </section>
            )}
          </div>
        )}
      </section>
      {menu && <Menu x={menu.x} y={menu.y} entries={entries} label="Shared item" onPick={(id) => void pick(id)} onClose={() => setMenu(null)} />}
      {d?.kind === 'leave' && <LeaveAlert name={nameOf(d.itemId)} onCancel={() => setDialog(null)}
        onLeave={async () => { setDialog(null); toast(await api.shareLeave(d.itemId), 'Left'); void load() }} />}
      {d?.kind === 'password' && <LinkPasswordDialog name={nameOf(d.itemId)} hasPassword={!!lists.links.find((l) => l.id === d.itemId)?.password} onClose={() => setDialog(null)}
        onSave={async (pw) => { setDialog(null); toast(await api.shareLinkUpdate(d.itemId, { password: pw }), pw ? 'Password set' : 'Password removed'); void load() }} />}
      {d?.kind === 'new-request' && <NewRequestDialog folders={['Files', 'Documents', 'Photos']} busy={busy} onCreate={(r) => void createRequest(r)} onClose={() => setDialog(null)} />}
      {d?.kind === 'mark-changed' && <><PeopleDialog name={nameOf(d.itemId)} items={shares.items[d.itemId] ?? 0} members={members} />
        <KeyChangedAlert member={d.member}
          onCompare={() => void safety(d.itemId, d.member)}
          onShareAnyway={async () => { const r = await api.shareAcceptChangedKey(d.member.id); if (!r.ok) return toast(r); await openPeople(d.itemId) }}
          onCancel={() => { dismissed.current.add(d.member.id); setDialog({ kind: 'people', itemId: d.itemId }) }} /></>}
      {d?.kind === 'people' && <PeopleDialog name={nameOf(d.itemId)} items={shares.items[d.itemId] ?? 0} members={members}
        onPerson={(m) => setDialog({ kind: 'person', itemId: d.itemId, member: m })} onInvite={() => setDialog({ kind: 'invite', itemId: d.itemId })} onClose={() => setDialog(null)} />}
      {d?.kind === 'person' && <PersonDialog member={d.member}
        onAccess={async (a) => { const r = await api.shareSetAccess(d.itemId, d.member.id, a); toast(r); if (r.ok) setDialog({ ...d, member: { ...d.member, access: a } }) }}
        onSafety={() => void safety(d.itemId, d.member)} onRemove={() => setDialog({ kind: 'remove', itemId: d.itemId, member: d.member })} onBack={() => void openPeople(d.itemId)} />}
      {/* DW-0512b: the confirm sits over the person's access (drawn behind it, inert), as the frame draws it. */}
      {d?.kind === 'remove' && <div inert><PersonDialog member={d.member} /></div>}
      {d?.kind === 'remove' && <RemoveAccessAlert member={d.member} folder={nameOf(d.itemId)} busy={busy}
        onRemove={async () => { setBusy(true); const r = await api.shareRevoke(d.itemId, d.member.id); setBusy(false); toast(r); if (r.ok) await openPeople(d.itemId) }}
        onCancel={() => setDialog({ kind: 'person', itemId: d.itemId, member: d.member })} />}
      {d?.kind === 'safety' && <SafetyNumberDialog member={d.member} groups={d.groups} busy={busy}
        onMatch={async () => {
          if (inFlight.current) return
          inFlight.current = true
          setBusy(true)
          const r = await api.shareMarkCompared(d.member.id, true)
          // A refused invite is not shared yet: They match seals it now, to the key just compared.
          if (d.refused) {
            if (!r.ok) { inFlight.current = false; setBusy(false); toast(r); return }
            // Hand the seal to invite() without an await in between, so no click can slip into the gap.
            inFlight.current = false
            await invite(d.itemId, d.refused.email, d.refused.canEdit, true, d.member.id)
            return
          }
          inFlight.current = false
          setBusy(false)
          // Rust wrote They match to every mark of the person: no earlier state of theirs stands.
          toast(r, 'Marked as compared'); setDialog({ kind: 'person', itemId: d.itemId, member: afterMarkCompared(d.member, r, true) })
        }}
        onNoMatch={async () => {
          if (inFlight.current) return
          inFlight.current = true
          setBusy(true)
          const r = await api.shareMarkCompared(d.member.id, false)
          inFlight.current = false
          setBusy(false)
          if (!r.ok) return toast(r)
          setDialog({ kind: 'mismatch', itemId: d.itemId, member: afterMarkCompared(d.member, r, false), groups: d.groups, refused: d.refused })
        }}
        // A refused invite holds no key: every way out of its comparison but They match withdraws it (as DW-0515 Cancel).
        onClose={() => (d.refused ? void withdraw(d.itemId, d.member) : setDialog({ kind: 'person', itemId: d.itemId, member: d.member }))} />}
      {d?.kind === 'mismatch' && <MismatchDialog member={d.member} groups={d.groups} busy={busy}
        onRemove={() => (d.refused ? void withdraw(d.itemId, d.member) : setDialog({ kind: 'remove', itemId: d.itemId, member: d.member }))}
        onCompare={() => setDialog({ kind: 'safety', itemId: d.itemId, member: d.member, groups: d.groups, refused: d.refused })}
        onClose={() => (d.refused ? void withdraw(d.itemId, d.member) : setDialog(null))} />}
      {d?.kind === 'invite' && <InviteDialog busy={busy} onSend={(email, a) => void invite(d.itemId, email, a === 'Can edit')} onClose={() => void openPeople(d.itemId)} />}
      {d?.kind === 'details' && <DetailsDialog item={d.details.item} details={d.details} now={now} onVersions={() => showVersions(d.details)} onClose={() => setDialog(null)} />}
      {d?.kind === 'versions' && <VersionsDialog name={d.details.item.name} versions={d.versions} now={now} onClose={() => setDialog({ kind: 'details', details: d.details })}
        onRestore={(vid) => void source.restoreVersion(d.details.item.id, vid).then(() => { setDialog(null); showToast({ kind: 'done', text: 'Version restored' }) }, (e: unknown) => showToast({ kind: 'failed', text: api.openFailureText(e, 'restore that version') }))} />}
      {d?.kind === 'key-changed' && <KeyChangedAlert member={d.member} busy={busy}
        onCompare={() => void safety(d.itemId, d.member, { email: d.email, canEdit: d.canEdit })}
        onShareAnyway={() => void invite(d.itemId, d.email, d.canEdit, true, d.member.id)} onCancel={() => void withdraw(d.itemId, d.member)} />}
    </>
  )
}
