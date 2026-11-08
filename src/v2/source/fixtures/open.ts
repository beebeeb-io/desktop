// Fixtures for the Open section (B07): the files the viewers, states and the editor show on the board.
import { DriveSourceError, type DriveItem, type DriveSource, type TextDocument, type VersionEntry } from '@beebeeb/shared/drive-ui'
import { createFilesSource, FILES_TREE } from './files'

const file = (id: string, name: string, size: number, mime: string, extra: Partial<DriveItem> = {}): DriveItem => ({
  id, parentId: null, name, isFolder: false, size, modifiedAt: '2026-11-08T09:14:00', mime, shared: { kind: 'none' }, local: 'downloaded', hasThumbnail: false, ...extra,
})

export const OPEN_ITEMS: DriveItem[] = [
  file('o-raw', 'DSC04182.ARW', 61.4 * 1024 ** 2, 'image/x-sony-arw', { hasThumbnail: true }),
  file('o-audio', 'Harbour sessions, take 3.wav', 38 * 1024 ** 2, 'audio/wav'),
  file('o-pdf', 'Studio plan.pdf', 1.2 * 1024 ** 2, 'application/pdf'),
  file('o-py', 'export_invoices.py', 812, 'text/x-python'),
  file('o-md', 'Meeting notes 4 Nov.md', 612, 'text/markdown'),
  file('o-csv', 'Invoices Q3.csv', 3400, 'text/csv'),
  file('o-zip', 'Raw footage 2024.zip', 1.4 * 1024 ** 3, 'application/zip'),
  file('o-log', 'server.log', 3.1 * 1024 ** 2, 'text/plain'),
]

export const PY_TEXT = `import csv
import sys
from datetime import date

RATE = 0.21  # VAT


def export_invoices(path, rows):
    """Write one invoice per row to a CSV file."""
    with open(path, "w", newline="") as handle:
        writer = csv.writer(handle)
        writer.writerow(["number", "client", "net", "vat"])
        for number, row in enumerate(rows, start=1):
            net = round(row["hours"] * row["rate"], 2)
            writer.writerow([number, row["client"], net, round(net * RATE, 2)])
    return len(rows)


if __name__ == "__main__":
    today = date.today().isoformat()
    count = export_invoices(sys.argv[1], [])
    print(f"{today}: wrote {count} invoices")
    if count == 0:
        print("nothing to export")
        sys.exit(1)
`

export const MD_TEXT = `# Meeting notes 4 Nov

Attendees: Anna, Bram, Guus

## Decisions

- Desktop v2 ships **Windows first**, macOS on the same code
- Passkeys run in Rust, never in the web view
- Alpha date: see [the plan](beebeeb://file/Studio plan.pdf)

## Next

1. Review the board with \`DW-1523\`
2. Read the [Windows notes](https://beebeeb.io/blog)
`

export const MD_TEXT_SAVED = `${MD_TEXT}3. Send the notes round
`

export const CSV_TEXT = `Invoice,Client,Date,Net,VAT,Total
1041,Northwind Studio,2026-10-02,1200.00,252.00,1452.00
1042,Harbour Films,2026-10-09,860.50,180.71,1041.21
1043,Northwind Studio,2026-10-16,430.00,90.30,520.30
1044,Lisbon Collective,2026-10-23,2100.00,441.00,2541.00
1045,Harbour Films,2026-10-30,640.00,134.40,774.40
1046,Northwind Studio,2026-11-06,980.00,205.80,1185.80
`

/** 48 rows for the CSV board frame. */
export const CSV_48 = CSV_TEXT + Array.from({ length: 42 }, (_, i) => `${1047 + i},Client ${i + 1},2026-11-${String((i % 28) + 1).padStart(2, '0')},${(300 + i * 17).toFixed(2)},${((300 + i * 17) * 0.21).toFixed(2)},${((300 + i * 17) * 1.21).toFixed(2)}`).join('\n') + '\n'

export const ZIP_ROWS: { path: string; size: number | null; depth: number }[] = [
  { path: 'Raw footage 2024', size: null, depth: 0 },
  { path: 'Day 1', size: null, depth: 1 },
  { path: 'A001_C001.mov', size: 2.4 * 1024 ** 3, depth: 2 },
  { path: 'A001_C002.mov', size: 1.9 * 1024 ** 3, depth: 2 },
  { path: 'Day 2', size: null, depth: 1 },
  { path: 'A002_C001.mov', size: 3.1 * 1024 ** 3, depth: 2 },
  { path: 'Audio', size: null, depth: 1 },
  { path: 'room-tone.wav', size: 88 * 1024 ** 2, depth: 2 },
  { path: 'notes.txt', size: 1200, depth: 1 },
]

export const PDF_PAGES: string[][] = [
  ['Studio plan', 'Northwind Studio, 2027', 'A short plan for the year: who we work for, what we need, and when.'],
  ['Where the work comes from', 'Most of the work is recurring: two agencies, one festival, and a handful of direct clients who come back each spring.', 'Equipment', 'The camera bodies are due for replacement in March. The grading monitor and the audio rig are fine.', 'People', 'Two editors, one producer, and a part-time bookkeeper. We plan one hire in the autumn.'],
  ['Budget', 'The terrace shoot is the largest line. Costs for the terrace permit are covered by the client.'],
  ['Calendar', 'January: planning. February: the terrace shoot. March: new cameras.'],
  ['Risks', 'A late festival date would move the terrace work into April.'],
  ['Next steps', 'Confirm the terrace date with the client, then order the cameras.'],
]

export const AGREEMENT_PAGES: string[][] = [
  ['Service agreement', 'This agreement covers the use of the terrace for the shoot.', 'Costs for the terrace are listed below.'],
  ['Scope', 'The space is available on weekdays only.'],
  ['Access', 'The terrace is closed in the evening.'],
]

export const RAW_FACTS: [string, string][] = [
  ['Taken', 'Sunday 14:20'],
  ['From', 'Shoot selects'],
  ['Camera', 'Sony ILCE-7M4'],
  ['Lens', 'FE 24-70mm F2.8 GM'],
  ['Exposure', 'ISO 400 · 1/500 s · f/4'],
  ['Focal length', '50 mm'],
  ['Size', '61.4 MB'],
  ['Preview', 'The camera’s own, 1616 × 1080'],
  ['Where', 'Shoot selects'],
  ['Version history', '1 version'],
]

export const AUDIO_QUEUE = [
  { id: 'a1', title: 'Harbour sessions, take 3', artist: 'Northwind Studio', length: 252 },
  { id: 'a2', title: 'Harbour sessions, take 4', artist: 'Northwind Studio', length: 241 },
  { id: 'a3', title: 'Room tone', artist: '', length: 62 },
  { id: 'a4', title: 'Terrace ambience', artist: 'Anna', length: 188 },
]

const TEXTS: Record<string, string> = { 'o-py': PY_TEXT, 'o-md': MD_TEXT, 'o-csv': CSV_TEXT }

/**
 * The files source plus the Open fixtures. `saveText` keeps the one rule of the real one: a save that names an
 * older version than the current one fails with 'conflict', never an overwrite. `remote` plants a newer version.
 */
export function createOpenSource(opts: { remoteText?: string } = {}): DriveSource {
  const items = [...FILES_TREE, ...OPEN_ITEMS]
  const base = createFilesSource({ items })
  const texts = new Map(Object.entries(TEXTS))
  const version = new Map<string, number>()
  let remote = opts.remoteText
  const current = (id: string) => version.get(id) ?? 3
  return {
    ...base,
    async readText(id): Promise<TextDocument> {
      const text = texts.get(id)
      if (text === undefined) throw new DriveSourceError('not-found')
      if (remote !== undefined) { version.set(id, 4); texts.set(id, remote); remote = undefined }
      return { text: texts.get(id)!, versionId: `v${current(id)}` }
    },
    async saveText(id, text, baseVersionId): Promise<TextDocument> {
      if (baseVersionId !== `v${current(id)}`) throw new DriveSourceError('conflict')
      version.set(id, current(id) + 1)
      texts.set(id, text)
      return { text, versionId: `v${current(id)}` }
    },
    async readBytes(id) {
      const text = texts.get(id)
      return { bytes: new TextEncoder().encode(text ?? ''), versionId: `v${current(id)}` }
    },
    async versions(id): Promise<VersionEntry[]> {
      const n = current(id)
      return Array.from({ length: n }, (_, i) => ({
        id: `v${n - i}`, createdAt: i === 0 ? '2026-11-08T09:14:00' : `2026-11-0${Math.max(1, 6 - i)}T10:00:00`, size: 612, current: i === 0, deviceName: i === 0 && n > 3 ? 'Anna’s iPhone' : 'Anna-Laptop',
      }))
    },
  }
}
