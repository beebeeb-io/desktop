// Batch B08's fixtures: the Office documents the board draws (the iPhone's files, DI-1507..1511) and a source
// whose readBytes/saveBytes/versions behave like the engine's (a stale base version is a conflict). Owned by B08.
import { DriveSourceError, type DriveItem, type DriveSource, type VersionEntry } from '@beebeeb/shared/drive-ui'
import { createFilesSource, FILES_TREE } from './files'

const none = { kind: 'none' } as const
function file(id: string, parentId: string | null, name: string, size: number, mime: string): DriveItem {
  return { id, parentId, name, isFolder: false, size, modifiedAt: '2026-11-08T09:12:00', mime, shared: none, local: 'downloaded', hasThumbnail: false, storageCountry: 'DE' }
}

export const OFFICE_ITEMS: DriveItem[] = [
  file('x-agreement', 'f-documents', 'Service agreement.docx', 48_200, 'application/vnd.openxmlformats-officedocument.wordprocessingml.document'),
  file('x-budget', null, 'Budget 2026.xlsx', 98_304, 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet'),
  file('x-recap', 'f-lisbon', 'Lisbon trip recap.pptx', 18.4 * 1024 ** 2, 'application/vnd.openxmlformats-officedocument.presentationml.presentation'),
  file('x-proposal', 'f-documents', 'Proposal 2012.doc', 31_744, 'application/msword'),
]

export interface DocSection { heading: string; text: string; selected?: string }
export interface DocFixture { title: string; sub: string; outline: [string, number][]; sections: DocSection[] }

export const SELECTED_TEXT = 'delivers 60 edited photos and a 90-second film.'

export const DOC_AGREEMENT: DocFixture = {
  title: 'Service agreement',
  sub: 'Northwind Studio and Lisbon Harbour Hotel · 28 October 2026',
  outline: [['Service agreement', 0], ['1. Scope of work', 1], ['2. Delivery', 1], ['3. Fees', 1], ['4. Usage', 1], ['5. Cancellation', 1], ['6. Signatures', 1]],
  sections: [
    { heading: '1. Scope of work', text: 'Northwind Studio photographs the hotel’s terrace, rooms and harbour view over two days and ', selected: SELECTED_TEXT },
    { heading: '2. Delivery', text: 'Edited photos within 14 days of the shoot, as full-size JPEG and web-size copies, shared through a Beebeeb link that expires after 30 days.' },
    { heading: '3. Fees', text: 'A fixed fee of €6,400, half on signing and half on delivery. Travel within Lisbon is included.' },
    { heading: '4. Usage', text: 'The hotel may use the photos on its own website, social channels and printed materials for five years.' },
  ],
}

export const DOC_2012: DocFixture = {
  title: 'Proposal',
  sub: 'Northwind Studio · 14 March 2012',
  outline: [['Proposal', 0], ['Who we are', 1], ['What we propose', 1], ['Fee', 1]],
  sections: [
    { heading: 'Who we are', text: 'Northwind Studio is a two-person photography studio in Amsterdam. Since 2009 we have photographed hotels, restaurants and the people who run them.' },
    { heading: 'What we propose', text: 'One shoot day for the new lobby and twelve rooms, delivered as 40 edited photos within ten working days.' },
    { heading: 'Fee', text: '€1,850 for the day, editing included. Prints are quoted separately.' },
  ],
}

/** The table Insert › Table… puts on the page (DW-1550). */
export const INSERTED_TABLE = [['Day', 'Location', 'Hours'], ['1', 'Alfama', '8'], ['2', 'Belém', '6.5']]

export const SHEET_COLS = ['A', 'B', 'C', 'D', 'E', 'F', 'G']
export const SHEET_ROWS: string[][] = [
  ['Item', 'Q1', 'Q2', 'Q3', 'Total'],
  ['Studio rent', '4,200', '4,200', '4,200', '12,600'],
  ['Equipment', '3,150', '820', '1,940', '5,910'],
  ['Travel', '640', '1,880', '2,410', '4,930'],
  ['Software', '410', '410', '455', '1,275'],
  ['Insurance', '1,120', '0', '0', '1,120'],
  ['Freelancers', '2,800', '3,600', '5,150', '11,550'],
  ['Prints and frames', '380', '0', '1,260', '1,640'],
  ['Marketing', '520', '640', '700', '1,860'],
  ['Total', '13,220', '11,550', '16,115', '40,885'],
  [],
  ['Budget', '14,000', '14,000', '16,000', '44,000'],
  ['Left', '780', '2,450', '−115', '3,115'],
]
export const SHEET_TABS = ['2026', '2025', 'Notes']

export interface SlideFixture { title: string; sub?: string; points: string[]; photo: boolean }
export const SLIDES: SlideFixture[] = [
  { title: 'Lisbon trip recap', sub: 'Northwind Studio · October 2026', points: [], photo: true },
  { title: 'What we shot', points: ['212 photos over two days · Alfama, Belém, the harbour', '60 edited selects for the hotel', 'A 90-second film'], photo: true },
  { title: 'Next steps', points: ['Deliver selects by 26 October', 'Hotel review call on 29 October'], photo: false },
]

/** The version each document is at in the fixtures (the status bar's "saved as version N"). */
export const OFFICE_VERSION: Record<string, number> = { 'x-agreement': 7, 'x-budget': 12, 'x-recap': 5, 'x-proposal': 3 }

/** Readable bytes for each document: the fixtures have no engine, so the content is a marker the editor never parses. */
export function createOfficeSource(opts: { remoteSavedFirst?: boolean } = {}): DriveSource {
  const items = [...FILES_TREE.filter((i) => !OFFICE_ITEMS.some((o) => o.name === i.name)), ...OFFICE_ITEMS]
  const base = createFilesSource({ items })
  const version = new Map(Object.entries(OFFICE_VERSION))
  let remote = opts.remoteSavedFirst ?? false
  const current = (id: string) => version.get(id) ?? 1
  return {
    ...base,
    async readBytes(id) {
      if (!OFFICE_ITEMS.some((i) => i.id === id)) throw new DriveSourceError('not-found')
      return { bytes: new TextEncoder().encode(id), versionId: `v${current(id)}` }
    },
    async saveBytes(id, bytes, baseVersionId) {
      if (remote) { version.set(id, current(id) + 1); remote = false }
      if (baseVersionId !== `v${current(id)}`) throw new DriveSourceError('conflict')
      version.set(id, current(id) + 1)
      return { bytes, versionId: `v${current(id)}` }
    },
    async versions(id): Promise<VersionEntry[]> {
      const n = current(id)
      return Array.from({ length: n }, (_, i) => ({ id: `v${n - i}`, createdAt: `2026-11-0${Math.max(1, 8 - i)}T09:12:00`, size: 48_200, current: i === 0, deviceName: i === 0 ? 'This PC' : 'Anna’s iPhone' }))
    },
  }
}
