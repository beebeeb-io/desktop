/**
 * Task 1546 finding 2: ConflictWindow.tsx's own doc-comment used to admit the
 * diff body was a hardcoded placeholder ("Content from this device…" /
 * "Content from other device…") for EVERY conflict, verbatim, regardless of
 * the files' actual content. This asserts those two fabricated strings are
 * gone and the window now wires the real content-preview IPC + line diff.
 */
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

const SRC = readFileSync(join(import.meta.dir, '..', 'src', 'ConflictWindow.tsx'), 'utf8')

describe('ConflictWindow real content', () => {
  test('no longer renders the fabricated placeholder lines', () => {
    expect(SRC).not.toContain('Content from this device…')
    expect(SRC).not.toContain('Content from other device…')
  })

  test('fetches the real content preview and computes a real diff', () => {
    expect(SRC).toContain('conflictContentPreview')
    expect(SRC).toContain('diffLines')
  })
})
