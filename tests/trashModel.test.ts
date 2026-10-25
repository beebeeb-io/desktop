/**
 * Trash's pure decisions (task 1683 slice 4; spec `docs/specs/2026-10-24-macos-two-surface-app.md` §7.7, hi-fi
 * section 3): the intro the Windows shell has, the row's second line, and when a file was deleted.
 * Nothing here renders; tests/trashPane.test.tsx runs the pane.
 */
import { describe, expect, test } from 'bun:test'
import type { DesktopTrashItem } from '../src/desktopApi'
import {
  TRASH_EMPTY_HINT,
  TRASH_EMPTY_TITLE,
  TRASH_INTRO,
  trashDeletedWhen,
  trashDeleteSheetText,
  trashRowLine,
} from '../src/trashModel'

const now = new Date(2026, 9, 25, 14, 30)
const secs = (y: number, mo: number, d: number, h = 12, mi = 0) => Math.floor(new Date(y, mo, d, h, mi).getTime() / 1000)

const item = (over: Partial<DesktopTrashItem> = {}): DesktopTrashItem => ({
  id: 't1',
  name: 'Draft 3, interview questions.docx',
  is_folder: false,
  size_bytes: 48_000,
  updated_at: String(secs(2026, 9, 24)),
  ...over,
})

describe('the words the Windows shell has, unchanged (§7.7)', () => {
  test('intro and empty state', () => {
    expect(TRASH_INTRO).toBe('Deleted files in your vault. Restore puts them back through sync; permanent delete removes them from Trash.')
    expect(TRASH_EMPTY_TITLE).toBe('Trash is empty')
    expect(TRASH_EMPTY_HINT).toBe('Deleted files will appear here.')
  })
})

describe('when a file was deleted', () => {
  test('today, yesterday, then the day', () => {
    expect(trashDeletedWhen(String(secs(2026, 9, 25, 9, 0)), now)).toBe('today')
    expect(trashDeletedWhen(String(secs(2026, 9, 24, 23, 59)), now)).toBe('yesterday')
    expect(trashDeletedWhen(String(secs(2026, 9, 12)), now)).toBe('12 Oct')
    expect(trashDeletedWhen(String(secs(2025, 2, 3)), now)).toBe('3 Mar 2025')
  })

  test('the server sends seconds, milliseconds or an ISO date; all three read the same', () => {
    const base = new Date(2026, 9, 12, 12, 0)
    expect(trashDeletedWhen(String(Math.floor(base.getTime() / 1000)), now)).toBe('12 Oct')
    expect(trashDeletedWhen(String(base.getTime()), now)).toBe('12 Oct')
    expect(trashDeletedWhen(base.toISOString(), now)).toBe('12 Oct')
  })

  test('a time nobody can read is no text (the Windows shell printed a dash)', () => {
    expect(trashDeletedWhen('', now)).toBeNull()
    expect(trashDeletedWhen('not a date', now)).toBeNull()
  })
})

describe('the row\'s second line (hi-fi section 3)', () => {
  test('a file: kind, size, when', () => {
    expect(trashRowLine(item(), now, 'en')).toBe('File · 48 KB · deleted yesterday')
  })

  test('a folder with a size shows it, in the Mac\'s decimal mark', () => {
    expect(trashRowLine(item({ name: 'Old scans', is_folder: true, size_bytes: 1_400_000_000, updated_at: String(secs(2026, 9, 12)) }), now, 'nl')).toBe('Folder · 1,4 GB · deleted 12 Oct')
  })

  test('a folder with no size says none: the server sends 0 for folders, and "0 B" would be a claim', () => {
    expect(trashRowLine(item({ is_folder: true, size_bytes: 0 }), now, 'en')).toBe('Folder · deleted yesterday')
  })

  test('an unknown deletion time drops that part, leaving no dangling separator', () => {
    expect(trashRowLine(item({ updated_at: '' }), now, 'en')).toBe('File · 48 KB')
  })
})

describe('the delete sheet (§7.7: "Delete permanently", as today)', () => {
  test('the sentence names the file and says it cannot be undone', () => {
    expect(trashDeleteSheetText('Old scans')).toBe('Old scans will be removed from Trash. This cannot be undone.')
  })
})
