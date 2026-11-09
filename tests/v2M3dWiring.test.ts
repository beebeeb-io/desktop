// M3d: the wiring of B06 (Photos, This PC), B07 (viewers, the text editor) and B08 (Office, desktop side): the photo
// pills and sources, a too-large file's state, Rust's error words, the New file flow's failure words, Sync history rows
// that open their file, and the fixture world those states are drawn from.
import { describe, expect, test } from 'bun:test'
import { DriveSourceError, type DriveItem } from '@beebeeb/shared/drive-ui'
import { devicesOf, shown, totalsOf, type PhotoCell } from '../src/v2/sections/photos/model'
import { failedWords } from '../src/v2/sections/photos/Dialogs'
import { rowsFromActivity } from '../src/v2/sections/sync-history/model'
import { errorFrom, isTooLarge } from '../src/v2/source/desktop'
import { createFailure, needsRoom } from '../src/v2/sections/open'
import { noRoomWords } from '../src/v2/sections/open/States'
import { OPEN_ITEMS, silentWav } from '../src/v2/source/fixtures/open'
import { createFixtureExtras } from '../src/v2/source/fixtures/extras'
import type { ActivityRow } from '../src/popoverContract'

const GB = 1024 ** 3
const item = (id: string, extra: Partial<DriveItem> = {}): DriveItem => ({
  id, parentId: 'f-backups', name: `${id}.heic`, isFolder: false, size: 1_000, modifiedAt: '2026-11-08T09:14:00', mime: 'image/heic',
  shared: { kind: 'none' }, local: 'online-only', hasThumbnail: true, ...extra,
})
const cell = (id: string, device?: string, extra: Partial<DriveItem> = {}): PhotoCell =>
  ({ entry: { item: item(id, extra) }, meta: device ? { device } : undefined }) as PhotoCell

describe('Photos: the device pills and what Photos shows (DW-0601, DW-0604)', () => {
  const cells = [cell('a', 'Anna’s iPhone'), cell('b', 'Anna’s iPhone'), cell('c', 'iPad'), cell('d', 'Old iPhone'), cell('e', 'Old iPhone'), cell('f', 'Old iPhone'), cell('g', undefined, { parentId: 'f-lisbon' })]
  test('one pill per device, most photos first; a photo outside Backups adds none', () => {
    expect(devicesOf(cells)).toEqual([{ name: 'Old iPhone', photos: 3 }, { name: 'Anna’s iPhone', photos: 2 }, { name: 'iPad', photos: 1 }])
  })
  test('a hidden device or folder hides its photos, and only those', () => {
    expect(shown(cells, new Set()).length).toBe(7)
    expect(shown(cells, new Set(['device:Old iPhone'])).map((c) => c.entry.item.id)).toEqual(['a', 'b', 'c', 'g'])
    expect(shown(cells, new Set(['folder:f-lisbon'])).map((c) => c.entry.item.id)).toEqual(['a', 'b', 'c', 'd', 'e', 'f'])
  })
  test('the totals count what is shown: photos, videos and bytes', () => {
    const mixed = [cell('p', 'iPad'), cell('v', 'iPad', { mime: 'video/quicktime', size: 5_000 })]
    expect(totalsOf(mixed)).toEqual({ photos: 1, videos: 1, bytes: 6_000 })
  })
  test('a failed action names what failed, never a code', () => {
    expect(failedWords(new DriveSourceError('offline'), 'move it')).toBe('Couldn’t move it. Check your connection.')
    expect(failedWords(new DriveSourceError('unsupported'), 'rename it')).toBe('Couldn’t rename it. This isn’t available in this version of Beebeeb.')
  })
})

describe('Rust’s error text becomes the error the viewer reads', () => {
  test('a file over the read cap is too large, not a failure to decrypt (DW-1517)', () => {
    const e = errorFrom('unsupported: too large', 'drive_read_bytes')
    expect([e.code, isTooLarge(e)]).toEqual(['unsupported', true])
  })
  test('any other unsupported answer stays unsupported', () => {
    const e = errorFrom('unsupported: drive', 'drive_read_bytes')
    expect([e.code, e.message, isTooLarge(e)]).toEqual(['unsupported', 'drive_read_bytes', false])
    expect(isTooLarge(errorFrom('unknown command drive_read_bytes', 'drive_read_bytes'))).toBe(false)
  })
  test('the codes pass through; unknown text is failed', () => {
    expect(errorFrom('quota: drive', 'x').code).toBe('quota')
    expect(errorFrom('name-taken: drive', 'x').code).toBe('name-taken')
    expect(errorFrom('boom', 'x').code).toBe('failed')
  })
})

describe('Too large to preview here (DW-1517)', () => {
  test('a file not on this PC that needs more than the free space', () => {
    expect(needsRoom({ local: 'online-only', size: 18.2 * GB }, 12.4 * GB)).toBe(true)
  })
  test('on this PC already, room enough, or the free space unknown: it opens', () => {
    expect(needsRoom({ local: 'downloaded', size: 18.2 * GB }, 12.4 * GB)).toBe(false)
    expect(needsRoom({ local: 'available-offline', size: 18.2 * GB }, 12.4 * GB)).toBe(false)
    expect(needsRoom({ local: 'online-only', size: 2 * GB }, 12.4 * GB)).toBe(false)
    expect(needsRoom({ local: 'online-only', size: 18.2 * GB }, undefined)).toBe(false)
    expect(needsRoom({ local: 'online-only', size: null }, 12.4 * GB)).toBe(false)
  })
  test('the status line says the numbers, as the board does', () => {
    expect(noRoomWords(18.2 * GB, 12.4 * GB)).toBe('Not enough space to decrypt · 18.2 GB needed · 12.4 GB free on this PC')
  })
  test('the fixture world agrees: the walkthrough needs more than the PC has free', async () => {
    const walkthrough = OPEN_ITEMS.find((i) => i.id === 'o-walkthrough')!
    const free = (await createFixtureExtras('default').status()).freeSpaceBytes
    expect(needsRoom(walkthrough, free)).toBe(true)
  })
})

describe('New file: Create file says what went wrong (DW-1522)', () => {
  test('each refusal has its words; nothing was made', () => {
    expect(createFailure(new DriveSourceError('name-taken'))).toBe('There’s already a file with that name here.')
    expect(createFailure(new DriveSourceError('read-only'))).toBe('This folder is read-only. Nothing was made.')
    expect(createFailure(new DriveSourceError('offline'))).toBe('You’re offline. Nothing was made. Try again when you’re connected.')
    expect(createFailure(new DriveSourceError('unsupported'))).toBe('Making a file isn’t available in this version of Beebeeb. Nothing was made.')
    expect(createFailure(new Error('x'))).toBe('Couldn’t make the file. Nothing was made. Try again.')
  })
  test('with no host command the fixture refuses: it never fakes a file (the old fixture did)', async () => {
    // A browser's dev host answers "unavailable" (the audit's command-unsupported); bun has no host at all.
    const e = await createFixtureExtras('default').createFile('f-documents', 'Untitled.txt', new Uint8Array()).then(() => null, (x: unknown) => x)
    expect(e instanceof DriveSourceError).toBe(true)
  })
})

describe('Sync history: a row opens its file (DW-1301)', () => {
  const row = (id: string): ActivityRow => ({ id, name: 'Contract draft.pdf', folder: 'Documents', path: 'Documents/Contract draft.pdf', direction: 'up', state: 'done', at: 1_762_590_000, size_bytes: 1 } as ActivityRow)
  test('the activity id is the file id; a finished transfer has none', () => {
    const [file, transfer] = rowsFromActivity([row('i-contract'), row('transfer-7')])
    expect(file.itemId).toBe('i-contract')
    expect(transfer.itemId).toBeUndefined()
  })
})

describe('the audio fixture is a real WAV (DW-1503 plays it)', () => {
  test('RIFF/WAVE header, 8-bit mono at 4 kHz, the length it says', () => {
    const w = silentWav(30)
    const ascii = (at: number) => String.fromCharCode(...w.slice(at, at + 4))
    const v = new DataView(w.buffer)
    expect([ascii(0), ascii(8), ascii(12), ascii(36)]).toEqual(['RIFF', 'WAVE', 'fmt ', 'data'])
    expect([v.getUint32(24, true), v.getUint16(34, true), v.getUint32(40, true), w.length]).toEqual([4000, 8, 120_000, 44 + 120_000])
  })
})
