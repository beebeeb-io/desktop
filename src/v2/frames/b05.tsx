// Batch B05's frames (design/desktop-v2/BATCHES.md): Shared (17), Search (4), Trash (5). Each renders the real section
// views against B05's fixtures (source/fixtures/shared.ts). Owned by B05 only.
import type { ReactElement, ReactNode } from 'react'
import { DriveSourceProvider } from '@beebeeb/shared/drive-ui'
import type { FrameRegistry } from './types'
import { CaptionBarView } from '../chrome/CaptionBar'
import { Sidebar } from '../chrome/Sidebar'
import { StatusLine } from '../chrome/StatusLine'
import { STORAGE_240, SYNC_OK } from '../chrome/FrameWindow'
import type { SectionId } from '../routes'
import { Menu } from '../sections/files/ui'
import { SharedView, type SharedViewProps } from '../sections/shared/SharedView'
import { InviteDialog, KeyChangedAlert, MismatchDialog, NewRequestDialog, PeopleDialog, PersonDialog, RemoveAccessAlert, SafetyNumberDialog } from '../sections/shared/Dialogs'
import { byMeMenu, requestMenu, withMeMenu, type SharedTab } from '../sections/shared/model'
import { SearchView, type SearchViewProps } from '../sections/search/SearchView'
import { DeleteForGoodDialog, TrashView, trashMenu, type TrashViewProps } from '../sections/trash/TrashView'
import {
  B05_NOW, BY_ME_LINKS, BY_ME_PEOPLE, createSharedSource, INVITATIONS, LISBON, LISBON_PEOPLE, RECENT_SEARCHES, REQUESTS, SAFETY_NUMBER,
  SEARCH_ITEMS, SEARCH_LOADING, TRASH, TRASH_TOTAL_BYTES, WITH_ME,
} from '../source/fixtures/shared'

const SRC = createSharedSource()
const NOW = new Date(B05_NOW)
const GB = 1024 ** 3
const STORAGE = { ...STORAGE_240, usedBytes: 240 * GB, filesBytes: 24 * GB, photosBytes: 216 * GB, backupsBytes: 0 }
const COUNTS: Partial<Record<SectionId, number>> = { shared: 1, trash: 12 }
const TITLES: Partial<Record<SectionId, string>> = { shared: 'Shared - Beebeeb', search: 'Search - Beebeeb', trash: 'Trash - Beebeeb' }
const noop = () => {}

function Win({ section, body, overlay, counts = COUNTS }: { section: SectionId; body: ReactNode; overlay?: ReactNode; counts?: Partial<Record<SectionId, number>> }): ReactElement {
  return (
    <DriveSourceProvider source={SRC}>
      <div data-frame-window className="relative flex h-screen flex-col overflow-hidden bg-paper font-sans text-ink">
        <CaptionBarView title={TITLES[section] ?? 'Beebeeb'} />
        <div className="flex min-h-0 flex-1">
          <Sidebar active={section === 'search' ? 'files' : section} onGo={noop} storage={STORAGE} accountName="Anna" counts={counts} />
          <main className="flex min-w-0 flex-1 flex-col">{body}</main>
        </div>
        <StatusLine sync={SYNC_OK} />
        {overlay}
      </div>
    </DriveSourceProvider>
  )
}

// ── Shared ───────────────────────────────────────────────────────────────────

const FULL: Omit<SharedViewProps, 'tab' | 'now'> = { invitations: INVITATIONS, withMe: WITH_ME, people: BY_ME_PEOPLE, links: BY_ME_LINKS, requests: REQUESTS }
const NONE: Omit<SharedViewProps, 'tab' | 'now'> = { invitations: [], withMe: [], people: [], links: [], requests: [] }
const shared = (tab: SharedTab, o: { data?: Partial<SharedViewProps>; overlay?: ReactNode; counts?: Partial<Record<SectionId, number>> } = {}) => (
  <Win section="shared" counts={o.counts} overlay={o.overlay} body={<SharedView tab={tab} now={NOW} {...FULL} {...o.data} />} />
)
const menuAt = (x: number, y: number, entries: Parameters<typeof Menu>[0]['entries'], label: string) => <Menu x={x} y={y} entries={entries} label={label} onPick={noop} />

const JONAS = LISBON_PEOPLE[1]
const people = (members = LISBON_PEOPLE) => <PeopleDialog name={LISBON.name} items={LISBON.items} members={members} />

// ── Search ───────────────────────────────────────────────────────────────────

const hitsFor = (q: string) => SEARCH_ITEMS.filter((r) => r.item.name.toLowerCase().includes(q.toLowerCase()))
const search = (o: Partial<SearchViewProps>) => <Win section="search" body={<SearchView query="" hits={[]} recent={RECENT_SEARCHES} now={NOW} {...o} />} />

// ── Trash ────────────────────────────────────────────────────────────────────

const trash = (o: Partial<TrashViewProps> = {}, overlay?: ReactNode, counts?: Partial<Record<SectionId, number>>) => (
  <Win section="trash" counts={counts} overlay={overlay} body={<TrashView entries={TRASH} bytes={TRASH_TOTAL_BYTES} now={NOW} {...o} />} />
)

export const FRAMES: FrameRegistry = {
  'DW-0501': () => shared('with-me'),
  'DW-0502': () => shared('by-me'),
  'DW-0503': () => shared('requests'),
  'DW-0504': () => shared('requests', { overlay: <NewRequestDialog folders={['Documents', 'Files', 'Photos']} /> }),
  'DW-0505': () => shared('with-me', { data: { menuOn: 'sw-brand' }, overlay: menuAt(520, 300, withMeMenu(), 'Shared item') }),
  'DW-0506': () => shared('by-me', { data: { menuOn: 'sl-budget' }, overlay: menuAt(520, 392, byMeMenu('link', true), 'Shared item') }),
  'DW-0507': () => shared('requests', { data: { menuOn: 'rq-selects' }, overlay: menuAt(520, 200, requestMenu(true), 'Shared item') }),
  'DW-0508': () => shared('with-me', { data: NONE, counts: {} }),
  'DW-0509': () => shared('by-me', { data: NONE, counts: {} }),
  'DW-0510': () => shared('requests', { data: NONE, counts: {} }),
  'DW-0511': () => shared('by-me', { overlay: people() }),
  'DW-0512': () => shared('by-me', { overlay: <PersonDialog member={JONAS} /> }),
  'DW-0512b': () => shared('by-me', { overlay: <><PersonDialog member={JONAS} /><RemoveAccessAlert member={JONAS} folder={LISBON.name} /></> }),
  'DW-0513': () => shared('by-me', { overlay: <SafetyNumberDialog member={JONAS} groups={SAFETY_NUMBER} /> }),
  'DW-0513b': () => shared('by-me', { overlay: <MismatchDialog member={{ ...JONAS, mismatch: true }} groups={SAFETY_NUMBER} /> }),
  'DW-0514': () => shared('by-me', { overlay: <InviteDialog email="jonas@northwind.studio" access="Can view" /> }),
  'DW-0515': () => shared('by-me', { overlay: <>{people()}<KeyChangedAlert member={JONAS} /></> }),

  'DW-0801': () => search({}),
  'DW-0802': () => search({ query: 'Contr', hits: hitsFor('Contr') }),
  'DW-0802b': () => search({ query: 'Contr', hits: hitsFor('Contr').slice(0, 2), loading: SEARCH_LOADING }),
  'DW-0803': () => search({ query: 'harbr', hits: [] }),

  'DW-0747': () => trash(),
  'DW-0748': () => trash({ menuOn: 't-contract-old' }, menuAt(470, 178, trashMenu(), 'Trash item')),
  'DW-0749': () => trash({}, <DeleteForGoodDialog title="Empty Trash?" count={12} bytes={TRASH_TOTAL_BYTES} factor="code" initialCode="51" />),
  'DW-0749b': () => trash({ entries: TRASH.slice(5), bytes: TRASH_TOTAL_BYTES - TRASH.slice(0, 5).reduce((n, e) => n + (e.item.size ?? 0), 0), emptying: { done: 5, total: 12 } }),
  'DW-0117': () => trash({ entries: [] }, undefined, { shared: 1 }),
}
