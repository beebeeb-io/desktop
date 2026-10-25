/**
 * A button's accessible name when the same word repeats down a list (task 1683 slice 4, review of #142): the visible word
 * first (WCAG 2.5.3, label in name), then what it acts on. Nothing here renders; the panes' own tests pin where it is used.
 */
import { describe, expect, test } from 'bun:test'
import { actionNamedFor } from '../src/actionName'

describe('actionNamedFor', () => {
  test('the visible word comes first, then the subject, so the name contains what is on the button', () => {
    expect(actionNamedFor('Restore', 'Draft 3, interview questions.docx')).toBe('Restore, Draft 3, interview questions.docx')
    expect(actionNamedFor('Delete…', 'Old scans')).toBe('Delete…, Old scans')
    expect(actionNamedFor('Decide…', 'Board notes.md').startsWith('Decide…')).toBe(true)
  })
})
