// Shared: With me, By me, Requests (DW-0501, DW-0502, DW-0503) and their empty states (DW-0508, DW-0509, DW-0510).
// A view: the host passes the rows and handles the actions.
import type { ReactNode } from 'react'
import { Icon } from '@beebeeb/shared/design'
import { FileIcon, getFileType } from '@beebeeb/shared/drive-ui'
import { BTN, BTN_AMBER, FOCUS } from '../files/ui'
import { fmtCount, fmtSize, fmtWhen } from '../files/model'
import { isPast, linkExpiry, peopleLabel, requestLine, type FileRequestRow, type Invitation, type LinkShare, type PeopleShare, type SharedTab, type WithMeRow } from './model'

export type RowKind = 'with-me' | 'people' | 'link' | 'request'
export interface SharedHandlers {
  onTab?: (t: SharedTab) => void
  onAccept?: (inv: Invitation) => void
  onDecline?: (inv: Invitation) => void
  onOpen?: (kind: RowKind, id: string) => void
  onContext?: (kind: RowKind, id: string, x: number, y: number) => void
  onPeople?: (id: string) => void
  onNewRequest?: () => void
  onFiles?: () => void
}

export interface SharedViewProps {
  tab: SharedTab
  now: Date
  invitations: Invitation[]
  withMe: WithMeRow[]
  people: PeopleShare[]
  links: LinkShare[]
  requests: FileRequestRow[]
  /** The row a menu is open on: drawn selected. */
  menuOn?: string
  h?: SharedHandlers
}

const TABS: { id: SharedTab; label: string }[] = [{ id: 'with-me', label: 'With me' }, { id: 'by-me', label: 'By me' }, { id: 'requests', label: 'Requests' }]

function Toolbar({ tab, h, sortable }: { tab: SharedTab; h?: SharedHandlers; sortable: boolean }) {
  return (
    <div role="toolbar" aria-label="Shared" className="flex h-12 flex-none items-center gap-3 border-b border-line px-4">
      <h1 className="m-0 text-[15px] font-semibold text-ink">Shared</h1>
      <div role="tablist" aria-label="Show" className="ml-2 inline-flex rounded-md border border-line bg-paper-2 p-0.5">
        {TABS.map((t) => (
          <button key={t.id} type="button" role="tab" aria-selected={tab === t.id} data-tab={t.id} onClick={() => h?.onTab?.(t.id)}
            className={`h-7 rounded px-3 text-[13px] ${tab === t.id ? 'bg-paper text-ink shadow-sm' : 'text-ink-2 hover:text-ink'} ${FOCUS}`}>{t.label}</button>
        ))}
      </div>
      <span className="flex-1" />
      {sortable && <button type="button" className={`inline-flex h-8 items-center gap-1.5 rounded-md px-2.5 text-[13px] text-ink-2 hover:bg-paper-2 ${FOCUS}`}>Sort<Icon name="chevron-down" size={12} /></button>}
      {tab === 'requests' && <button type="button" data-new-request className={BTN_AMBER} onClick={h?.onNewRequest}>New request</button>}
    </div>
  )
}

function Heading({ children, count }: { children: ReactNode; count: number }) {
  return <h2 className="m-0 flex h-9 items-end gap-2 px-4 pb-1.5 text-[12px] font-semibold text-ink-2">{children}<span className="font-mono font-normal text-ink-3">{count}</span></h2>
}

function Row({ id, kind, name, isFolder, cells, selected, h }: { id: string; kind: RowKind; name: string; isFolder: boolean; cells: ReactNode[]; selected?: boolean; h?: SharedHandlers }) {
  return (
    <div role="row" tabIndex={-1} data-row={id} data-kind={kind} aria-selected={selected}
      onDoubleClick={() => h?.onOpen?.(kind, id)} onKeyDown={(e) => { if (e.key === 'Enter') h?.onOpen?.(kind, id) }}
      onContextMenu={(e) => { e.preventDefault(); h?.onContext?.(kind, id, e.clientX, e.clientY) }}
      className={`grid h-11 cursor-default grid-cols-[minmax(0,2.2fr)_minmax(0,1.6fr)_minmax(0,1.1fr)_96px] items-center gap-3 px-4 text-[13px] ${selected ? 'bg-paper-3' : 'hover:bg-paper-2'} ${FOCUS}`}>
      <span role="gridcell" className="flex min-w-0 items-center gap-2.5"><FileIcon type={getFileType(name, isFolder)} size={22} /><span className="truncate text-ink" title={name}>{name}</span></span>
      {cells.map((c, i) => <span key={i} role="gridcell" className="min-w-0 truncate">{c}</span>)}
    </div>
  )
}

function Columns({ labels }: { labels: string[] }) {
  return (
    <div role="row" className="grid h-8 grid-cols-[minmax(0,2.2fr)_minmax(0,1.6fr)_minmax(0,1.1fr)_96px] items-center gap-3 border-b border-line px-4 text-[12px] text-ink-3">
      {labels.map((l) => <span key={l} role="columnheader">{l}</span>)}
    </div>
  )
}

const mono = (s: string) => <span className="font-mono text-[12px] text-ink-3">{s}</span>
const sizeOf = (r: { isFolder?: boolean; items?: number; size?: number | null }) => mono(r.isFolder ? fmtCount(r.items) : fmtSize(r.size ?? null))

function Empty({ kind, title, body, action }: { kind: string; title: string; body: string; action?: ReactNode }) {
  return (
    <div data-empty={kind} className="flex flex-1 flex-col items-center justify-center gap-2 px-8 text-center">
      <div className="mb-1 grid h-16 w-16 place-items-center rounded-2xl border border-line bg-paper-2 text-ink-3"><Icon name={kind === 'requests' ? 'upload' : 'users'} size={26} /></div>
      <h2 className="m-0 text-[15px] font-semibold text-ink">{title}</h2>
      <p className="m-0 max-w-[380px] text-[13px] leading-5 text-ink-2">{body}</p>
      {action && <div className="mt-2">{action}</div>}
    </div>
  )
}

function InviteCard({ inv, h }: { inv: Invitation; h?: SharedHandlers }) {
  return (
    <div data-invitation={inv.id} className="mx-4 mt-3 flex items-center gap-3 rounded-lg border border-line bg-paper-2 px-4 py-3">
      <FileIcon type={getFileType(inv.name, inv.isFolder)} size={28} />
      <span className="min-w-0 flex-1">
        <span className="block text-[13px] font-medium text-ink">{inv.name}</span>
        <span className="block text-[12px] text-ink-2">{inv.from} wants to share this {inv.isFolder ? 'folder' : 'file'}</span>
      </span>
      <button type="button" className={BTN} onClick={() => h?.onDecline?.(inv)}>Decline</button>
      <button type="button" className={BTN} onClick={() => h?.onAccept?.(inv)}>Accept</button>
    </div>
  )
}

export function SharedView(p: SharedViewProps) {
  const { tab, h, now } = p
  const byMeCount = p.people.length + p.links.length
  const sortable = tab === 'with-me' ? p.withMe.length > 1 : tab === 'by-me' ? byMeCount > 1 : p.requests.length > 1
  let body: ReactNode
  if (tab === 'with-me') {
    body = p.withMe.length === 0 && p.invitations.length === 0
      ? <Empty kind="with-me" title="Nothing shared with you" body="When someone shares a file or folder with you, it appears here." />
      : (
        <div className="min-h-0 flex-1 overflow-auto pb-4">
          {p.invitations.length > 0 && <>
            <Heading count={p.invitations.length}>Waiting for you</Heading>
            {p.invitations.map((inv) => <InviteCard key={inv.id} inv={inv} h={h} />)}
          </>}
          <Heading count={p.withMe.length}>Shared with you</Heading>
          <div role="grid" aria-label="Shared with you" data-list-count={p.withMe.length}>
            <Columns labels={['Name', 'Shared by', 'Modified', 'Size']} />
            {p.withMe.map((r) => <Row key={r.id} id={r.id} kind="with-me" name={r.name} isFolder={r.isFolder} selected={p.menuOn === r.id} h={h}
              cells={[<span className="text-ink-2">{r.owner}</span>, mono(r.modifiedAt ? fmtWhen(r.modifiedAt, now) : '—'), sizeOf(r)]} />)}
          </div>
        </div>
      )
  } else if (tab === 'by-me') {
    body = byMeCount === 0
      ? <Empty kind="by-me" title="You haven’t shared anything" body="Right-click any file or folder in Files and choose Share… to share it." action={<a href="?section=files" onClick={(e) => { e.preventDefault(); h?.onFiles?.() }} className={`${BTN} no-underline`}>Files</a>} />
      : (
        <div className="min-h-0 flex-1 overflow-auto pb-4">
          <Heading count={p.people.length}>With people</Heading>
          <div role="grid" aria-label="With people" data-list-count={p.people.length}>
            <Columns labels={['Name', 'People', 'Status', 'Size']} />
            {p.people.map((r) => <Row key={r.id} id={r.id} kind="people" name={r.name} isFolder={r.isFolder} selected={p.menuOn === r.id} h={h}
              cells={[
                <button type="button" onClick={() => h?.onPeople?.(r.id)} className={`inline-flex max-w-full items-center gap-1.5 rounded px-1 text-left text-ink-2 hover:bg-paper-3 hover:text-ink ${FOCUS}`}><Icon name="users" size={13} /><span className="truncate">{peopleLabel(r.people)}</span></button>,
                <span className="text-[12px] text-ink-3">{r.pending ? 'Pending' : ''}</span>, sizeOf(r)]} />)}
          </div>
          <Heading count={p.links.length}>Links</Heading>
          <div role="grid" aria-label="Links" data-list-count={p.links.length}>
            <Columns labels={['Name', 'Link', 'Password', 'Size']} />
            {p.links.map((r) => <Row key={r.id} id={r.id} kind="link" name={r.name} isFolder={false} selected={p.menuOn === r.id} h={h}
              cells={[
                <span className={`inline-flex items-center gap-1.5 text-[12px] ${isPast(r.expiresAt, now) ? 'text-ink-3' : 'text-ink-2'}`}><Icon name="link" size={13} />{linkExpiry(r.expiresAt, now)}</span>,
                <span className="inline-flex items-center gap-1.5 text-[12px] text-ink-3">{r.password ? <><Icon name="lock" size={12} />With password</> : ''}</span>, sizeOf(r)]} />)}
          </div>
        </div>
      )
  } else {
    body = p.requests.length === 0
      ? <Empty kind="requests" title="No file requests" body="Ask anyone to send you files, even without a Beebeeb account." action={<button type="button" className={BTN} onClick={h?.onNewRequest}>New request</button>} />
      : (
        <div className="min-h-0 flex-1 overflow-auto pb-4">
          <Heading count={p.requests.length}>File requests</Heading>
          <div role="grid" aria-label="File requests" data-list-count={p.requests.length}>
            <Columns labels={['Request', 'Files', 'State', '']} />
            {p.requests.map((r) => {
              const l = requestLine(r, now)
              return <Row key={r.id} id={r.id} kind="request" name={r.title} isFolder={false} selected={p.menuOn === r.id} h={h}
                cells={[mono(l.files), <span className="text-[12px] text-ink-2">{l.state ?? ''}</span>,
                  <span className="text-[12px] text-ink-3">{l.open ? 'Open' : ''}</span>]} />
            })}
          </div>
          <p className="m-0 mt-3 px-4 text-[12px] leading-4 text-ink-3">Each request is a link. Closing stops new uploads and keeps what arrived.</p>
        </div>
      )
  }
  return (
    <section aria-label="Shared" data-shared={tab} className="flex min-h-0 min-w-0 flex-1 flex-col">
      <Toolbar tab={tab} h={h} sortable={sortable} />
      {body}
    </section>
  )
}
