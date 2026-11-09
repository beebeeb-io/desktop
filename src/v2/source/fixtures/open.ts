// Fixtures for the Open section (B07): the files the viewers, states and the editor show on the board.
import { DriveSourceError, type DriveItem, type DriveSource, type TextDocument, type VersionEntry } from '@beebeeb/shared/drive-ui'
import { createFilesSource, FILES_TREE, thumbnailFor } from './files'

const file = (id: string, name: string, size: number, mime: string, extra: Partial<DriveItem> = {}): DriveItem => ({
  id, parentId: null, name, isFolder: false, size, modifiedAt: '2026-11-08T09:14:00', mime, shared: { kind: 'none' }, local: 'downloaded', hasThumbnail: false, ...extra,
})

// M3d: the RAW sits in Shoot selects with the shoot's other RAWs (DW-1501's strip, ← →), the recording in Studio
// demos with the folder's other audio (DW-1503's Up next), and Studio plan.pdf in Lisbon trip (DW-1516's Cancel goes
// back to that folder, DW-0420).
const raw = (id: string, name: string) => file(id, name, 61.4 * 1024 ** 2, 'image/x-sony-arw', { parentId: 'f-shoot', hasThumbnail: true })
const wav = (id: string, name: string, mb: number) => file(id, name, mb * 1024 ** 2, 'audio/wav', { parentId: 'f-studio-demos' })

export const OPEN_ITEMS: DriveItem[] = [
  raw('o-raw-1', 'DSC04181.ARW'),
  raw('o-raw', 'DSC04182.ARW'),
  raw('o-raw-3', 'DSC04183.ARW'),
  wav('o-audio', 'Harbour sessions, take 3.wav', 38),
  wav('o-audio-4', 'Harbour sessions, take 4.wav', 36),
  wav('o-audio-tone', 'Room tone.wav', 9),
  wav('o-audio-terrace', 'Terrace ambience.wav', 27),
  file('o-pdf', 'Studio plan.pdf', 1.2 * 1024 ** 2, 'application/pdf', { parentId: 'f-lisbon' }),
  file('o-py', 'export_invoices.py', 812, 'text/x-python'),
  file('o-md', 'Meeting notes 4 Nov.md', 612, 'text/markdown'),
  file('o-csv', 'Invoices Q3.csv', 3400, 'text/csv'),
  file('o-zip', 'Raw footage 2024.zip', 1.4 * 1024 ** 3, 'application/zip'),
  file('o-log', 'server.log', 3.1 * 1024 ** 2, 'text/plain'),
  // DW-1517: online only, and more than the 12.4 GB this PC has free (fixtures/extras.ts).
  file('o-walkthrough', 'Studio walkthrough 4K.mov', 18.2 * 1024 ** 3, 'video/quicktime', { parentId: 'f-studio-demos', local: 'online-only' }),
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
- Alpha date: see [the plan](beebeeb://file/Studio%20plan.pdf)

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

/** Studio demos, inside Shoot selects: a folder no board frame lists, so the Files frames keep their rows. */
const STUDIO_DEMOS: DriveItem = { ...file('f-studio-demos', 'Studio demos', 0, ''), parentId: 'f-shoot', isFolder: true, size: null, childCount: 5, mime: undefined }
/** The Open fixtures that Files lists too (the RAW's and the recording's folders), merged into the board's tree. */
export const OPEN_TREE: DriveItem[] = [STUDIO_DEMOS, ...OPEN_ITEMS.filter((i) => i.parentId === 'f-shoot' || i.parentId === 'f-studio-demos')]

/** A silent mono WAV (8-bit, 4 kHz) of `seconds`: the recordings' bytes, so the player really plays and pauses. */
export function silentWav(seconds: number): Uint8Array {
  const rate = 4000
  const n = rate * seconds
  const out = new Uint8Array(44 + n)
  const v = new DataView(out.buffer)
  const ascii = (at: number, t: string) => [...t].forEach((c, i) => v.setUint8(at + i, c.charCodeAt(0)))
  ascii(0, 'RIFF'); v.setUint32(4, 36 + n, true); ascii(8, 'WAVE'); ascii(12, 'fmt ')
  v.setUint32(16, 16, true); v.setUint16(20, 1, true); v.setUint16(22, 1, true); v.setUint32(24, rate, true)
  v.setUint32(28, rate, true); v.setUint16(32, 1, true); v.setUint16(34, 8, true); ascii(36, 'data'); v.setUint32(40, n, true)
  out.fill(128, 44)
  return out
}

/** `decrypting`: Studio plan.pdf's read stays in flight (DW-1516). `newer`: Anna's iPhone saves Meeting notes while it
 *  is open (DW-1519). `edit-conflict`: the first save of Meeting notes meets her newer version (DW-1526). */
export type OpenScenario = 'default' | 'decrypting' | 'newer' | 'edit-conflict'

/** A stand-in for the content part of Rust's tag (fsops::version_tag hashes the plaintext in Rust). Fixtures only: it
 *  tells two texts apart, nothing more, and no window code computes one. */
const contentOf = (text: string) => {
  let h = 5381
  for (let i = 0; i < text.length; i++) h = ((h * 33) ^ text.charCodeAt(i)) >>> 0
  return h.toString(16).padStart(8, '0')
}

/** How long the fixture engine takes to encrypt and upload a save, or to land a restore. */
export const FIXTURE_UPLOAD_MS = 600

/**
 * The files source plus the Open fixtures. They keep the real one's rules: a save names the content it was read from
 * and a changed file is a 'conflict', never an overwrite. Rust writes a save on this PC at once and answers its tag
 * with the server's number unchanged; the engine uploads it a moment later, which raises the version (drive_details'
 * versionCount) and says `changed`, as desktop's engine poller does. A restore is queued the same way.
 * `remote` plants a newer version.
 */
export function createOpenSource(opts: { remoteText?: string; scenario?: OpenScenario } = {}): DriveSource {
  const items = [...FILES_TREE, ...OPEN_TREE.filter((i) => i.isFolder), ...OPEN_ITEMS]
  const base = createFilesSource({ items })
  const texts = new Map(Object.entries(TEXTS))
  const version = new Map<string, number>()
  const devices = new Map<string, string>()
  const moving = new Set<string>()
  let remote = opts.remoteText
  const scenario = opts.scenario ?? 'default'
  let conflictPlanted = false
  let newerPlanted = false
  const made = Date.now()
  const listeners = new Set<Parameters<DriveSource['subscribe']>[0]>()
  const current = (id: string) => version.get(id) ?? 3
  const tag = (id: string) => `v${current(id)}-${contentOf(texts.get(id) ?? '')}`
  const changed = () => listeners.forEach((l) => l({ kind: 'changed', folderId: null }))
  /** A new version on the server: its number, who made it, and the engine's `changed`. */
  const land = (id: string, device: string) => {
    version.set(id, current(id) + 1)
    devices.set(`${id}:${current(id)}`, device)
    moving.delete(id)
    changed()
  }
  /** Anna's iPhone saves version 4 of Meeting notes: her text, the current version, and the engine's events. */
  const herSave = (id: string) => {
    version.set(id, 4)
    devices.set(`${id}:4`, 'Anna’s iPhone')
    texts.set(id, `${MD_TEXT}3. Book the terrace\n`)
    const item = items.find((i) => i.id === id)
    if (item) listeners.forEach((l) => l({ kind: 'item', item }))
    changed()
  }
  return {
    ...base,
    async details(id) {
      const d = await base.details(id)
      if (d.item.isFolder) return d
      return { ...d, item: moving.has(id) ? { ...d.item, local: 'syncing' } : d.item, versionCount: current(id) }
    },
    async readText(id): Promise<TextDocument> {
      // server.log has no text here: its read fails, as a download that is cut off does. The window's first read fails
      // at once (DW-1518); a later Try again downloads again first, so it decrypts for a moment before it fails (DW-1516).
      if (id === 'o-log' && Date.now() - made > 1500) await new Promise((r) => setTimeout(r, 2000))
      const text = texts.get(id)
      if (text === undefined) throw new DriveSourceError('not-found')
      if (remote !== undefined) { version.set(id, 4); texts.set(id, remote); remote = undefined }
      if (scenario === 'newer' && id === 'o-md' && !newerPlanted) { newerPlanted = true; setTimeout(() => herSave(id), 300) }
      return { text: texts.get(id)!, versionId: tag(id) }
    },
    async saveText(id, text, baseVersionId): Promise<TextDocument> {
      if (scenario === 'edit-conflict' && id === 'o-md' && !conflictPlanted) { conflictPlanted = true; herSave(id) }
      if (baseVersionId.slice(baseVersionId.indexOf('-') + 1) !== contentOf(texts.get(id) ?? '')) throw new DriveSourceError('conflict')
      texts.set(id, text)
      moving.add(id)
      setTimeout(() => land(id, 'This PC'), FIXTURE_UPLOAD_MS)
      return { text, versionId: tag(id) }
    },
    async restoreVersion(id) {
      // Queued, as restore_file_version is: the server's restore lands a moment later as the next version.
      moving.add(id)
      setTimeout(() => land(id, 'This PC'), FIXTURE_UPLOAD_MS)
    },
    async readBytes(id) {
      if (scenario === 'decrypting' && id === 'o-pdf') return new Promise<never>(() => {})
      const item = OPEN_ITEMS.find((i) => i.id === id)
      if (item?.mime === 'audio/wav') return { bytes: silentWav(30), versionId: tag(id) }
      const text = texts.get(id)
      return { bytes: new TextEncoder().encode(text ?? ''), versionId: tag(id) }
    },
    async thumbnail(id, size) {
      const item = OPEN_ITEMS.find((i) => i.id === id)
      return item?.hasThumbnail ? thumbnailFor(item.name) : base.thumbnail(id, size)
    },
    subscribe(listener) {
      listeners.add(listener)
      const stop = base.subscribe(listener)
      return () => { listeners.delete(listener); stop() }
    },
    async versions(id): Promise<VersionEntry[]> {
      const n = current(id)
      return Array.from({ length: n }, (_, i) => ({
        id: `v${n - i}`, createdAt: i === 0 ? '2026-11-08T09:14:00' : `2026-11-0${Math.max(1, 6 - i)}T10:00:00`, size: 612, current: i === 0,
        deviceName: devices.get(`${id}:${n - i}`) ?? 'Anna-Laptop',
      }))
    },
  }
}
