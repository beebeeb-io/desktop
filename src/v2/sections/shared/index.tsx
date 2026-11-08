// Shared, live: DriveSource.shared() for the lists it can fill; share_list (Rust) for invitations, link expiry,
// pending and file requests; the share_* and file_request_* commands for every action (sections/shared/api.ts).
// A command this build lacks shows the honest "isn't available" toast, never invented data.
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useDriveSource } from '@beebeeb/shared/drive-ui'
import type { SectionProps } from '../types'
import { showToast } from '../../chrome/Toast'
import { Menu } from '../files/ui'
import { SharedView, type RowKind } from './SharedView'
import {
  InviteDialog, KeyChangedAlert, LeaveAlert, LinkPasswordDialog, MismatchDialog, NewRequestDialog, PeopleDialog, PersonDialog, RemoveAccessAlert, SafetyNumberDialog,
  type NewRequestInput,
} from './Dialogs'
import { byMeMenu, EXPIRY_CHOICES, requestLine, requestMenu, rowsFromEntries, withMeMenu, type Member, type SharedTab } from './model'
import * as api from './api'
import type { ShareLists } from './api'

type Dialog =
  | { kind: 'new-request' }
  | { kind: 'leave'; itemId: string }
  | { kind: 'password'; itemId: string }
  | { kind: 'people'; itemId: string }
  | { kind: 'person'; itemId: string; member: Member }
  | { kind: 'remove'; itemId: string; member: Member }
  | { kind: 'safety'; itemId: string; member: Member; groups: string[] }
  | { kind: 'mismatch'; itemId: string; member: Member; groups: string[] }
  | { kind: 'invite'; itemId: string }
  | { kind: 'key-changed'; itemId: string; member: Member; email: string; canEdit: boolean }
  | { kind: 'mark-changed'; itemId: string; member: Member }

const EMPTY_LISTS: ShareLists = { invitations: [], links: [], requests: [], pending: [] }
const days = (label: string) => (label === 'Never' ? null : Number(label.replace(/\D/g, '')) || null)
const bytes = (label: string) => (label === 'No limit' ? null : Number(label.replace(/\D/g, '')) * 1024 ** 3)

function toast(r: { ok: boolean }, done?: string) {
  if (r.ok) { if (done) showToast({ kind: 'done', text: done }) } else showToast({ kind: 'failed', text: api.failureText(r as never) })
}

export default function Section({ nav }: SectionProps) {
  const source = useDriveSource()
  const tab = (['with-me', 'by-me', 'requests'].includes(nav.route.sub ?? '') ? nav.route.sub : 'with-me') as SharedTab
  const [rows, setRows] = useState(() => rowsFromEntries([]))
  const [lists, setLists] = useState<ShareLists>(EMPTY_LISTS)
  const [members, setMembers] = useState<Member[]>([])
  const [menu, setMenu] = useState<{ kind: RowKind; id: string; x: number; y: number } | null>(null)
  const [dialog, setDialog] = useState<Dialog | null>(null)
  const [busy, setBusy] = useState(false)
  const dismissed = useRef(new Set<string>())
  const now = useMemo(() => new Date(), [])

  const load = useCallback(async () => {
    const entries = await source.shared().catch(() => [])
    setRows(rowsFromEntries(entries))
    const l = await api.shareList()
    if (l.ok) setLists(l.value)
  }, [source])
  useEffect(() => {
    void load()
    return source.subscribe((e) => { if (e.kind === 'changed') void load() })
  }, [source, load])

  // Links: DriveSource knows which items have one; share_list adds expiry and password.
  const links = rows.links.map((l) => lists.links.find((x) => x.id === l.id) ?? l)
  const people = rows.people.map((p) => ({ ...p, pending: lists.pending.includes(p.id) }))
  const nameOf = (id: string) => [...rows.people, ...rows.links, ...rows.withMe].find((r) => r.id === id)?.name ?? ''

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
      if (id === 'open' || id === 'preview') await source.open(rowId).catch(() => showToast({ kind: 'failed', text: 'Couldn’t open it. Try again.' }))
      else if (id === 'offline') await source.setAvailableOffline(rowId, true).then(() => showToast({ kind: 'done', text: 'Kept on this PC' }), () => showToast({ kind: 'failed', text: 'Couldn’t keep it on this PC.' }))
      else if (id === 'reveal') await source.revealInFileManager(rowId).catch(() => {})
      else if (id === 'details') nav.go({ section: 'files', id: rowId, sub: 'details' })
      else if (id === 'leave') setDialog({ kind: 'leave', itemId: rowId })
    } else if (kind === 'link' || kind === 'people') {
      if (id === 'copy') { const r = await api.shareLinkCopy(rowId); toast(r, r.ok ? `Link copied: ${r.value.display}` : undefined) }
      else if (choice) { toast(await api.shareLinkUpdate(rowId, { expiresDays: choice.days }), 'Expiry changed'); void load() }
      else if (id === 'password') setDialog({ kind: 'password', itemId: rowId })
      else if (id === 'open') await source.open(rowId).catch(() => {})
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

  const invite = async (itemId: string, email: string, canEdit: boolean, acceptChangedKey = false) => {
    setBusy(true)
    const r = await api.shareInvite(itemId, email, canEdit ? 'Can edit' : 'Can view', acceptChangedKey)
    setBusy(false)
    if (!r.ok) return toast(r)
    if (r.value.status === 'key-changed') {
      const member = members.find((m) => m.id === (r.value as { memberId: string }).memberId) ?? { id: r.value.memberId, name: email, email }
      return setDialog({ kind: 'key-changed', itemId, member, email, canEdit })
    }
    await openPeople(itemId)
  }

  const safety = async (itemId: string, member: Member) => {
    const r = await api.shareSafetyNumber(member.id)
    if (!r.ok) return toast(r)
    setDialog({ kind: 'safety', itemId, member, groups: r.value.groups })
  }

  const go = (t: SharedTab) => nav.go({ section: 'shared', id: null, sub: t })
  const d = dialog
  return (
    <>
      <SharedView
        tab={tab} now={now} invitations={lists.invitations} withMe={rows.withMe} people={people} links={links} requests={lists.requests} menuOn={menu?.id}
        h={{
          onTab: go,
          onAccept: async (inv) => { toast(await api.shareAccept(inv.id), `${inv.name} added`); void load() },
          onDecline: async (inv) => { toast(await api.shareDecline(inv.id)); void load() },
          onOpen: (kind, id) => { if (kind === 'with-me' || kind === 'link' || kind === 'people') void source.open(id).catch(() => {}) },
          onContext: (kind, id, x, y) => setMenu({ kind, id, x, y }),
          onPeople: (id) => void openPeople(id),
          onNewRequest: () => setDialog({ kind: 'new-request' }),
          onFiles: () => nav.go({ section: 'files' }),
        }}
      />
      {menu && <Menu x={menu.x} y={menu.y} entries={entries} label="Shared item" onPick={(id) => void pick(id)} onClose={() => setMenu(null)} />}
      {d?.kind === 'leave' && <LeaveAlert name={nameOf(d.itemId)} onCancel={() => setDialog(null)}
        onLeave={async () => { setDialog(null); toast(await api.shareLeave(d.itemId), 'Left'); void load() }} />}
      {d?.kind === 'password' && <LinkPasswordDialog name={nameOf(d.itemId)} hasPassword={!!links.find((l) => l.id === d.itemId)?.password} onClose={() => setDialog(null)}
        onSave={async (pw) => { setDialog(null); toast(await api.shareLinkUpdate(d.itemId, { password: pw }), pw ? 'Password set' : 'Password removed'); void load() }} />}
      {d?.kind === 'new-request' && <NewRequestDialog folders={['Files', 'Documents', 'Photos']} busy={busy} onCreate={(r) => void createRequest(r)} onClose={() => setDialog(null)} />}
      {d?.kind === 'mark-changed' && <><PeopleDialog name={nameOf(d.itemId)} items={people.find((p) => p.id === d.itemId)?.items ?? 0} members={members} />
        <KeyChangedAlert member={d.member}
          onCompare={() => void safety(d.itemId, d.member)}
          onShareAnyway={async () => { const r = await api.shareAcceptChangedKey(d.member.id); if (!r.ok) return toast(r); await openPeople(d.itemId) }}
          onCancel={() => { dismissed.current.add(d.member.id); setDialog({ kind: 'people', itemId: d.itemId }) }} /></>}
      {d?.kind === 'people' && <PeopleDialog name={nameOf(d.itemId)} items={people.find((p) => p.id === d.itemId)?.items ?? 0} members={members}
        onPerson={(m) => setDialog({ kind: 'person', itemId: d.itemId, member: m })} onInvite={() => setDialog({ kind: 'invite', itemId: d.itemId })} onClose={() => setDialog(null)} />}
      {d?.kind === 'person' && <PersonDialog member={d.member}
        onAccess={async (a) => { const r = await api.shareSetAccess(d.itemId, d.member.id, a); toast(r); if (r.ok) setDialog({ ...d, member: { ...d.member, access: a } }) }}
        onSafety={() => void safety(d.itemId, d.member)} onRemove={() => setDialog({ kind: 'remove', itemId: d.itemId, member: d.member })} onBack={() => void openPeople(d.itemId)} />}
      {d?.kind === 'remove' && <RemoveAccessAlert member={d.member} folder={nameOf(d.itemId)} busy={busy}
        onRemove={async () => { setBusy(true); const r = await api.shareRevoke(d.itemId, d.member.id); setBusy(false); toast(r); if (r.ok) await openPeople(d.itemId) }}
        onCancel={() => setDialog({ kind: 'person', itemId: d.itemId, member: d.member })} />}
      {d?.kind === 'safety' && <SafetyNumberDialog member={d.member} groups={d.groups}
        onMatch={async () => { const r = await api.shareMarkCompared(d.member.id, true); toast(r, 'Marked as compared'); setDialog({ kind: 'person', itemId: d.itemId, member: { ...d.member, compared: r.ok } }) }}
        onNoMatch={async () => { const r = await api.shareMarkCompared(d.member.id, false); if (!r.ok) return toast(r); setDialog({ kind: 'mismatch', itemId: d.itemId, member: d.member, groups: d.groups }) }}
        onClose={() => setDialog({ kind: 'person', itemId: d.itemId, member: d.member })} />}
      {d?.kind === 'mismatch' && <MismatchDialog member={d.member} groups={d.groups}
        onRemove={() => setDialog({ kind: 'remove', itemId: d.itemId, member: d.member })} onCompare={() => setDialog({ kind: 'safety', itemId: d.itemId, member: d.member, groups: d.groups })} onClose={() => setDialog(null)} />}
      {d?.kind === 'invite' && <InviteDialog busy={busy} onSend={(email, a) => void invite(d.itemId, email, a === 'Can edit')} onClose={() => void openPeople(d.itemId)} />}
      {d?.kind === 'key-changed' && <KeyChangedAlert member={d.member}
        onCompare={() => void safety(d.itemId, d.member)} onShareAnyway={() => void invite(d.itemId, d.email, d.canEdit, true)} onCancel={() => void openPeople(d.itemId)} />}
    </>
  )
}
