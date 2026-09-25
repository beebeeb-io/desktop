/**
 * Task 1546 finding 2. `diffLines` is the pure line-level diff ConflictWindow
 * renders instead of the old hardcoded placeholder ("Content from this
 * device…" / "Content from other device…" for EVERY conflict, verbatim,
 * regardless of the files' actual content).
 */
import { describe, expect, test } from 'bun:test'
import { diffLines, DIFF_MAX_CELLS } from '../src/diffLines'

describe('diffLines', () => {
  test('identical content is all "same" lines', () => {
    const ops = diffLines('a\nb\nc', 'a\nb\nc')
    expect(ops).toEqual([
      { type: 'same', line: 'a' },
      { type: 'same', line: 'b' },
      { type: 'same', line: 'c' },
    ])
  })

  test('an appended line shows as add, the rest stay same', () => {
    const ops = diffLines('a\nb', 'a\nb\nc')
    expect(ops).toEqual([
      { type: 'same', line: 'a' },
      { type: 'same', line: 'b' },
      { type: 'add', line: 'c' },
    ])
  })

  test('a removed middle line shows as remove, surrounding lines stay same', () => {
    const ops = diffLines('a\nb\nc', 'a\nc')
    expect(ops).toEqual([
      { type: 'same', line: 'a' },
      { type: 'remove', line: 'b' },
      { type: 'same', line: 'c' },
    ])
  })

  test('completely different content reconstructs both sides from remove+add lines', () => {
    const ops = diffLines('one\ntwo', 'three\nfour')
    expect(ops).not.toBeNull()
    const reconstructedA = ops!.filter((op) => op.type !== 'add').map((op) => op.line).join('\n')
    const reconstructedB = ops!.filter((op) => op.type !== 'remove').map((op) => op.line).join('\n')
    expect(reconstructedA).toBe('one\ntwo')
    expect(reconstructedB).toBe('three\nfour')
  })

  test('falls back to null (no line-level highlighting) above the cell cap, never hangs or OOMs', () => {
    // Construct two distinct-enough line sets whose product exceeds the cap.
    const perSide = Math.ceil(Math.sqrt(DIFF_MAX_CELLS)) + 50
    const a = Array.from({ length: perSide }, (_, i) => `a-line-${i}`).join('\n')
    const b = Array.from({ length: perSide }, (_, i) => `b-line-${i}`).join('\n')
    expect(diffLines(a, b)).toBeNull()
  })
})
