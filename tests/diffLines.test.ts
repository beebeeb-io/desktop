/**
 * Task 1546 finding 2. `diffLines` is the pure line-level diff ConflictWindow
 * renders instead of the old hardcoded placeholder ("Content from this
 * device…" / "Content from other device…" for EVERY conflict, verbatim,
 * regardless of the files' actual content).
 *
 * Task 1546 Codex round 2, finding 4: `diffLines` also caps the total
 * number of ops it returns (`DIFF_MAX_OPS`) — `DIFF_MAX_CELLS` alone doesn't
 * bound a highly ASYMMETRIC pair (many lines on one side, few on the other),
 * which stays under the DP-cell budget yet can still produce hundreds of
 * thousands of ops, each rendered as its own React element.
 */
import { describe, expect, test } from 'bun:test'
import { diffLines, DIFF_MAX_CELLS, DIFF_MAX_OPS } from '../src/diffLines'

describe('diffLines', () => {
  test('identical content is all "same" lines', () => {
    const result = diffLines('a\nb\nc', 'a\nb\nc')
    expect(result?.ops).toEqual([
      { type: 'same', line: 'a' },
      { type: 'same', line: 'b' },
      { type: 'same', line: 'c' },
    ])
    expect(result?.truncated).toBe(false)
  })

  test('an appended line shows as add, the rest stay same', () => {
    const result = diffLines('a\nb', 'a\nb\nc')
    expect(result?.ops).toEqual([
      { type: 'same', line: 'a' },
      { type: 'same', line: 'b' },
      { type: 'add', line: 'c' },
    ])
    expect(result?.truncated).toBe(false)
  })

  test('a removed middle line shows as remove, surrounding lines stay same', () => {
    const result = diffLines('a\nb\nc', 'a\nc')
    expect(result?.ops).toEqual([
      { type: 'same', line: 'a' },
      { type: 'remove', line: 'b' },
      { type: 'same', line: 'c' },
    ])
    expect(result?.truncated).toBe(false)
  })

  test('completely different content reconstructs both sides from remove+add lines', () => {
    const result = diffLines('one\ntwo', 'three\nfour')
    expect(result).not.toBeNull()
    const ops = result!.ops
    const reconstructedA = ops.filter((op) => op.type !== 'add').map((op) => op.line).join('\n')
    const reconstructedB = ops.filter((op) => op.type !== 'remove').map((op) => op.line).join('\n')
    expect(reconstructedA).toBe('one\ntwo')
    expect(reconstructedB).toBe('three\nfour')
    expect(result!.truncated).toBe(false)
  })

  test('falls back to null (no line-level highlighting) above the cell cap, never hangs or OOMs', () => {
    // Construct two distinct-enough line sets whose product exceeds the cap.
    const perSide = Math.ceil(Math.sqrt(DIFF_MAX_CELLS)) + 50
    const a = Array.from({ length: perSide }, (_, i) => `a-line-${i}`).join('\n')
    const b = Array.from({ length: perSide }, (_, i) => `b-line-${i}`).join('\n')
    expect(diffLines(a, b)).toBeNull()
  })

  test('a highly asymmetric pair under the cell cap is still truncated at DIFF_MAX_OPS', () => {
    // Codex round 2, finding 4's exact failure scenario: one side has many
    // more lines than the other, so (n+1)*(m+1) stays small (well under
    // DIFF_MAX_CELLS) even though n alone is far past DIFF_MAX_OPS.
    const manyLines = Array.from({ length: DIFF_MAX_OPS * 2 }, (_, i) => `line-${i}`).join('\n')
    const oneLine = 'single line'
    const result = diffLines(manyLines, oneLine)
    expect(result).not.toBeNull()
    expect(result!.truncated).toBe(true)
    expect(result!.ops.length).toBeLessThanOrEqual(DIFF_MAX_OPS)
  })

  test('an op count exactly at DIFF_MAX_OPS is not marked truncated', () => {
    // Boundary check: truncation means "more ops existed than we kept," not
    // "we hit the cap exactly." Every A-line is distinct from the single
    // B-line, so there are no "same" matches to complicate the count:
    // (DIFF_MAX_OPS - 1) removes + 1 add = exactly DIFF_MAX_OPS ops, all
    // produced by the loops running to natural completion.
    const a = Array.from({ length: DIFF_MAX_OPS - 1 }, (_, i) => `only-in-a-${i}`).join('\n')
    const b = 'only-in-b'
    const result = diffLines(a, b)
    expect(result).not.toBeNull()
    expect(result!.ops.length).toBe(DIFF_MAX_OPS)
    expect(result!.truncated).toBe(false)
  })
})
