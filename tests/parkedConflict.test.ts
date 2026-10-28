/**
 * Task 2190 (plan 2026-10-25-2190-macos-parked-conflict §3.4, D1): the conflict window decides a
 * parked conflict by its queued write, and says "a copy of your changes stays" only on the path
 * whose engine call copies first.
 */
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { PARKED_MOVED, PARKED_REPLACE_QUESTION, parkedResolutionNotice } from '../src/parkedConflict'

const WINDOW = readFileSync(join(import.meta.dir, '..', 'src', 'ConflictWindow.tsx'), 'utf8').replace(/\r\n/g, '\n')
const CENTER = readFileSync(join(import.meta.dir, '..', 'src', 'pages', 'VersionCenter.tsx'), 'utf8').replace(/\r\n/g, '\n')

describe('parked conflict', () => {
  test('every outcome has a notice; only a moved write keeps the window open', () => {
    expect(parkedResolutionNotice({ outcome: 'rebased', version: 2 }).done).toBe(true)
    expect(parkedResolutionNotice({ outcome: 'kept_other', kept_copy: '/k/notes.txt' }).done).toBe(true)
    const both = parkedResolutionNotice({ outcome: 'kept_both', file_id: 'n', name: 'notes (Mac, 10.00).txt' })
    expect(both.done).toBe(true)
    expect(both.message).toContain('notes (Mac, 10.00).txt')
    expect(parkedResolutionNotice({ outcome: 'moved' })).toEqual({ done: false, message: PARKED_MOVED })
  })

  test('the window decides a parked conflict by its op id, with the parked commands', () => {
    expect(WINDOW).toContain("params.get('opId')")
    expect(WINDOW).toContain('opId ? parkedConflictPreview(opId) : conflictContentPreview(fileId)')
    expect(WINDOW).toContain('resolveParkedConflict(opId, choice)')
    expect(CENTER).toContain("opId: entry.kind === 'stale_base' ? (entry.op_id ?? null) : null")
  })

  test('D1: the replace question is asked only on the parked path, whose remote choice copies first', () => {
    // The question shows only after the first press on a parked conflict, and the second press is
    // the parked `remote` decision (the engine copies into Changes not sent before it drops the write).
    expect(WINDOW).toContain("onClick={() => (opId && !confirmReplace ? setConfirmReplace(true) : resolve('remote'))}")
    expect(WINDOW).toContain('if (opId) {\n      await resolveParked(choice)')
    expect(WINDOW.match(/PARKED_REPLACE_QUESTION/g)?.length).toBe(2) // the import and the one render
    expect(WINDOW).toContain('{confirmReplace && (')
    expect(PARKED_REPLACE_QUESTION).toBe(
      'Replace this Mac’s version? A copy of your changes stays in the Changes not sent folder.',
    )
  })
})
