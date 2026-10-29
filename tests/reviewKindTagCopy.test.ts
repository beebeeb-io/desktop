/**
 * Review kind tags on the Versions page follow the approved macOS spec
 * (docs/specs/2026-10-24-macos-two-surface-app.md, copy table row 16), and an
 * expired session reads "Signed out" (Guus, 2026-10-28: "work it out, you got it").
 */
import { describe, expect, test } from 'bun:test'
import { reviewKindLabel } from '../src/reviewKindLabel'

describe('reviewKindLabel', () => {
  test('spec row 16 tags replace the engine words', () => {
    expect(reviewKindLabel('stale_base')).toBe('Changed elsewhere')
    expect(reviewKindLabel('metadata')).toBe('Rename or move')
    expect(reviewKindLabel('quota_failure')).toBe('Storage full')
    expect(reviewKindLabel('permission_failure')).toBe('No permission')
  })

  test('an expired session names the state', () => {
    expect(reviewKindLabel('auth_failure')).toBe('Signed out')
  })

  test('the tags the spec keeps are unchanged', () => {
    expect(reviewKindLabel('failed_upload')).toBe('Upload')
    expect(reviewKindLabel('delete')).toBe('Delete')
    expect(reviewKindLabel('restore')).toBe('Restore')
    expect(reviewKindLabel('kept_copy')).toBe('Kept copy')
    expect(reviewKindLabel('held_edit')).toBe('Not sent')
    expect(reviewKindLabel('held_unknown')).toBe('Not sent')
    expect(reviewKindLabel('conflict')).toBe('Conflict')
  })
})
