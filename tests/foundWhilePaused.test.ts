// Task 2267: on Windows, local items Beebeeb could not prove were the account's own (found while sync was refused) are
// held with the pause reason `found_while_paused` until the person decides: "Upload to my Beebeeb" or "Keep on this PC
// only". These are the card's words, its result lines and when it shows, without React.
import { describe, expect, test } from 'bun:test'
import {
  FOUND_WHILE_PAUSED,
  FOUND_WHILE_PAUSED_COPY,
  KEPT_FOLDER_NAME,
  NOT_CONFIRMED_LINE,
  foundWhilePausedCardVisible,
  foundWhilePausedCount,
  heldKindsLine,
  keepConfirm,
  keepLabel,
  keepResultLines,
  parseKeepResult,
  parseReview,
  waitingForDecisionLine,
  type FoundWhilePausedReview,
  type KeepResult,
} from '../src/windows/foundWhilePaused'
import type { HeldGroup, SyncActivitySummary } from '../src/windows/syncActivityModel'

const KEPT = 'C:\\Users\\Fixture\\Beebeeb - kept on this PC\\2026-10-28'

function keep(patch: Partial<KeepResult> = {}): KeepResult {
  return { moved_files: 0, moved_items: 0, kept_path: null, not_moved: 0, stays_held: 0, synced_by: null, ...patch }
}

function review(patch: Partial<FoundWhilePausedReview> = {}): FoundWhilePausedReview {
  return { new_folders: 0, new_files: 0, edits: 0, deletions: 0, in_shared_folders: 0, op_ids: [], synced_by: null, ...patch }
}

function summaryWith(held: HeldGroup[]): SyncActivitySummary {
  return {
    paused: false,
    engine_state: 'idle',
    files_remaining: null,
    transfers_active: 0,
    last_tick_ok_at: 1_793_000_000,
    queue: { waiting: 0, gave_up: 0, held },
    startup_check: null,
  }
}

const group = (reason: string, count: number): HeldGroup => ({ reason, count, oldest_at: 1_791_115_500, newest_at: 1_791_115_500 })

describe('the card’s words (exact)', () => {
  test('title, buttons and the inline confirmation', () => {
    expect(FOUND_WHILE_PAUSED).toBe('found_while_paused')
    expect(KEPT_FOLDER_NAME).toBe('Beebeeb - kept on this PC')
    expect(FOUND_WHILE_PAUSED_COPY).toEqual({
      title: 'Files Beebeeb found while sync was paused. They haven’t been uploaded yet.',
      upload: 'Upload to my Beebeeb',
      keep: 'Keep on this PC only',
      confirm: 'They’ll be moved out of your Beebeeb folder into a folder named “Beebeeb - kept on this PC” next to it. Nothing is uploaded or deleted.',
      move: 'Move them',
      cancel: 'Cancel',
      showInExplorer: 'Show in Explorer',
      dismiss: 'Dismiss',
    })
  })

  test('the count line, singular and plural', () => {
    expect(waitingForDecisionLine(1)).toBe('1 change waiting for your decision.')
    expect(waitingForDecisionLine(3)).toBe('3 changes waiting for your decision.')
    expect(waitingForDecisionLine(1_200)).toBe('1,200 changes waiting for your decision.')
  })
})

describe('the keep result, line by line', () => {
  test('moved only: the dated folder, in full, with the sentence split around it for the mono path', () => {
    const lines = keepResultLines(keep({ moved_files: 3, moved_items: 2, kept_path: KEPT }))
    expect(lines.map((line) => line.text)).toEqual([`Moved 3 files to ${KEPT}. They stay on this PC and aren’t uploaded.`])
    expect(lines[0].key).toBe('moved')
    expect(lines[0].path).toEqual({ before: 'Moved 3 files to ', path: KEPT, after: '. They stay on this PC and aren’t uploaded.' })
  })

  test('moved, singular', () => {
    expect(keepResultLines(keep({ moved_files: 1, moved_items: 1, kept_path: KEPT })).map((line) => line.text)).toEqual([
      `Moved 1 file to ${KEPT}. It stays on this PC and isn’t uploaded.`,
    ])
  })

  test('files are counted, not top-level items: one folder holding 40 files is 40 files', () => {
    expect(keepResultLines(keep({ moved_files: 40, moved_items: 1, kept_path: KEPT }))[0].text).toBe(
      `Moved 40 files to ${KEPT}. They stay on this PC and aren’t uploaded.`,
    )
  })

  test('only empty folders moved (no files): the line counts the items, never "Moved 0 files"', () => {
    expect(keepResultLines(keep({ moved_items: 2, kept_path: KEPT })).map((line) => line.text)).toEqual([
      `Moved 2 items to ${KEPT}. They stay on this PC and aren’t uploaded.`,
    ])
    expect(keepResultLines(keep({ moved_items: 1, kept_path: KEPT })).map((line) => line.text)).toEqual([
      `Moved 1 item to ${KEPT}. It stays on this PC and isn’t uploaded.`,
    ])
  })

  test('moved but no path reported: the folder is named, never left out', () => {
    const [line] = keepResultLines(keep({ moved_files: 2, moved_items: 2, kept_path: null }))
    expect(line.text).toBe(`Moved 2 files to ${KEPT_FOLDER_NAME}. They stay on this PC and aren’t uploaded.`)
    expect(line.path?.path).toBe(KEPT_FOLDER_NAME)
  })

  test('not moved only, singular and plural', () => {
    expect(keepResultLines(keep({ not_moved: 1 })).map((line) => line.text)).toEqual([
      '1 item couldn’t be moved and stays where it is. It isn’t uploaded.',
    ])
    expect(keepResultLines(keep({ not_moved: 4 })).map((line) => line.text)).toEqual([
      '4 items couldn’t be moved and stay where they are. They aren’t uploaded.',
    ])
    expect(keepResultLines(keep({ not_moved: 4 }))[0]).toEqual({ key: 'not_moved', text: '4 items couldn’t be moved and stay where they are. They aren’t uploaded.' })
  })

  test('stays held only, singular and plural', () => {
    expect(keepResultLines(keep({ stays_held: 1 })).map((line) => line.text)).toEqual([
      '1 change can’t be kept on this PC only. It isn’t uploaded and stays on hold until you choose “Upload to my Beebeeb”.',
    ])
    expect(keepResultLines(keep({ stays_held: 2 })).map((line) => line.text)).toEqual([
      '2 changes can’t be kept on this PC only. They aren’t uploaded and stay on hold until you choose “Upload to my Beebeeb”.',
    ])
    expect(keepResultLines(keep({ stays_held: 2 }))[0].key).toBe('stays_held')
  })

  test('all three: moved first, then what stayed, then what stays held', () => {
    const lines = keepResultLines(keep({ moved_files: 12, moved_items: 3, kept_path: KEPT, not_moved: 2, stays_held: 5 }))
    expect(lines.map((line) => line.key)).toEqual(['moved', 'not_moved', 'stays_held'])
    expect(lines.map((line) => line.text)).toEqual([
      `Moved 12 files to ${KEPT}. They stay on this PC and aren’t uploaded.`,
      '2 items couldn’t be moved and stay where they are. They aren’t uploaded.',
      '5 changes can’t be kept on this PC only. They aren’t uploaded and stay on hold until you choose “Upload to my Beebeeb”.',
    ])
  })

  test('nothing moved, nothing left, nothing held: no line at all', () => {
    expect(keepResultLines(keep())).toEqual([])
    expect(keepResultLines(keep({ kept_path: KEPT }))).toEqual([])
  })

  test('only the lines with a count are shown', () => {
    expect(keepResultLines(keep({ not_moved: 1, stays_held: 1 })).map((line) => line.key)).toEqual(['not_moved', 'stays_held'])
    expect(keepResultLines(keep({ moved_files: 1, moved_items: 1, kept_path: KEPT, stays_held: 1 })).map((line) => line.key)).toEqual(['moved', 'stays_held'])
  })
})

describe('the keep answer’s shape', () => {
  test('the contract’s shape parses, field for field', () => {
    const good = { moved_files: 3, moved_items: 2, kept_path: KEPT, not_moved: 1, stays_held: 0, synced_by: null }
    expect(parseKeepResult(good)).toEqual(good)
    expect(parseKeepResult({ ...good, kept_path: null })).toEqual({ ...good, kept_path: null })
    expect(parseKeepResult({ ...good, synced_by: 'OneDrive' })).toEqual({ ...good, synced_by: 'OneDrive' })
  })

  test('anything outside it is no result, never a guess', () => {
    const good = { moved_files: 3, moved_items: 2, kept_path: KEPT, not_moved: 1, stays_held: 0, synced_by: null }
    for (const bad of [
      null,
      undefined,
      [],
      'moved',
      { ...good, moved_files: -1 },
      { ...good, moved_items: 1.5 },
      { ...good, not_moved: '1' },
      { ...good, stays_held: undefined },
      { ...good, kept_path: 7 },
      { ...good, synced_by: 1 },
      { moved_files: 3, moved_items: 2, kept_path: KEPT, not_moved: 1, stays_held: 0 },
      { moved_files: 3, moved_items: 2, not_moved: 1, stays_held: 0, synced_by: null },
    ]) {
      expect(parseKeepResult(bad)).toBeNull()
    }
  })
})

describe('when the card shows', () => {
  const idle = { busy: false, kept: null }

  test('no summary, no held group, or another reason only: hidden', () => {
    expect(foundWhilePausedCardVisible(undefined, idle)).toBe(false)
    expect(foundWhilePausedCardVisible(null, idle)).toBe(false)
    expect(foundWhilePausedCardVisible({ ...summaryWith([]), queue: null }, idle)).toBe(false)
    expect(foundWhilePausedCardVisible(summaryWith([]), idle)).toBe(false)
    expect(foundWhilePausedCardVisible(summaryWith([group('auth', 2_924), group('quota', 3)]), idle)).toBe(false)
  })

  test('a found-while-paused group with no changes: hidden', () => {
    expect(foundWhilePausedCardVisible(summaryWith([group(FOUND_WHILE_PAUSED, 0)]), idle)).toBe(false)
  })

  test('a found-while-paused group with changes: shown', () => {
    expect(foundWhilePausedCardVisible(summaryWith([group(FOUND_WHILE_PAUSED, 1)]), idle)).toBe(true)
    expect(foundWhilePausedCardVisible(summaryWith([group('auth', 5), group(FOUND_WHILE_PAUSED, 7)]), idle)).toBe(true)
  })

  test('a keep result with something to say keeps it shown after the group is gone, until dismissed', () => {
    const none = summaryWith([])
    expect(foundWhilePausedCardVisible(none, { busy: false, kept: keep({ moved_files: 2, moved_items: 1, kept_path: KEPT }) })).toBe(true)
    expect(foundWhilePausedCardVisible(none, { busy: false, kept: keep({ stays_held: 1 }) })).toBe(true)
    expect(foundWhilePausedCardVisible(none, { busy: false, kept: keep() })).toBe(false)
  })

  test('a command in flight keeps it shown, so a poll that lands mid-move cannot drop the result', () => {
    expect(foundWhilePausedCardVisible(summaryWith([]), { busy: true, kept: null })).toBe(true)
  })

  test('the count is the found-while-paused changes only', () => {
    expect(foundWhilePausedCount(undefined)).toBe(0)
    expect(foundWhilePausedCount(summaryWith([group('auth', 2_924)]))).toBe(0)
    expect(foundWhilePausedCount(summaryWith([group('auth', 2_924), group(FOUND_WHILE_PAUSED, 6)]))).toBe(6)
  })
})

// Lead review round 3 of #176: the card shows what is held by kind (P2-2) and Upload sends back exactly the changes it
// counted (P3-1); when OneDrive or Dropbox syncs the folder the kept folder is made in, the words never claim the
// items stay on this PC (P3-4).
describe('what is held, by kind', () => {
  test('each kind with a count, in order, joined as a sentence', () => {
    expect(heldKindsLine(review({ new_folders: 2, new_files: 5, edits: 1 }))).toBe('Waiting for your decision: 2 new folders, 5 new files and 1 edit.')
    expect(heldKindsLine(review({ new_files: 1 }))).toBe('Waiting for your decision: 1 new file.')
    expect(heldKindsLine(review({ new_folders: 1, deletions: 2, in_shared_folders: 1 }))).toBe(
      'Waiting for your decision: 1 new folder, 2 deletions and 1 change in a folder shared with you.',
    )
    expect(heldKindsLine(review({ edits: 3, deletions: 1, in_shared_folders: 4 }))).toBe(
      'Waiting for your decision: 3 edits, 1 deletion and 4 changes in folders shared with you.',
    )
    expect(heldKindsLine(review({ new_files: 1_200 }))).toBe('Waiting for your decision: 1,200 new files.')
  })

  test('nothing counted: no line', () => {
    expect(heldKindsLine(review())).toBe('')
  })

  test('the sentence under it, exact', () => {
    expect(NOT_CONFIRMED_LINE).toBe('Beebeeb couldn’t confirm these came from your account on this PC.')
  })

  test('the review parses field for field, and nothing outside its shape', () => {
    const good = { new_folders: 1, new_files: 2, edits: 0, deletions: 0, in_shared_folders: 1, op_ids: ['a', 'b', 'c', 'd'], synced_by: null }
    expect(parseReview(good)).toEqual(good)
    expect(parseReview({ ...good, synced_by: 'Dropbox' })).toEqual({ ...good, synced_by: 'Dropbox' })
    for (const bad of [
      null,
      [],
      { ...good, op_ids: 'a' },
      { ...good, op_ids: ['a', 2] },
      { ...good, edits: -1 },
      { ...good, deletions: undefined },
      { ...good, synced_by: 3 },
      { ...good, op_ids: ['a'] },
    ]) {
      expect(parseReview(bad)).toBeNull()
    }
  })
})

describe('a kept folder OneDrive or Dropbox syncs', () => {
  test('the button and the question never say "on this PC" then', () => {
    expect(keepLabel(null)).toBe(FOUND_WHILE_PAUSED_COPY.keep)
    expect(keepLabel('OneDrive')).toBe('Keep out of Beebeeb')
    expect(keepConfirm(null)).toBe(FOUND_WHILE_PAUSED_COPY.confirm)
    expect(keepConfirm('OneDrive')).toBe(
      'They’ll be moved out of your Beebeeb folder into a folder named “Beebeeb - kept on this PC” next to it. Nothing is uploaded to Beebeeb or deleted. This folder is synced by OneDrive.',
    )
  })

  test('the result says which service syncs them, never that they stay on this PC', () => {
    const lines = keepResultLines(keep({ moved_files: 2, moved_items: 1, kept_path: KEPT, stays_held: 1, synced_by: 'Dropbox' }))
    expect(lines.map((line) => line.text)).toEqual([
      `Moved 2 files to ${KEPT}. They aren’t uploaded to Beebeeb. This folder is synced by Dropbox.`,
      '1 change can’t be kept out of Beebeeb. It isn’t uploaded and stays on hold until you choose “Upload to my Beebeeb”.',
    ])
    expect(keepResultLines(keep({ moved_files: 1, moved_items: 1, kept_path: KEPT, synced_by: 'OneDrive' }))[0].text).toBe(
      `Moved 1 file to ${KEPT}. It isn’t uploaded to Beebeeb. This folder is synced by OneDrive.`,
    )
    expect(lines.map((line) => line.text).join(' ')).not.toContain('on this PC.')
  })
})
