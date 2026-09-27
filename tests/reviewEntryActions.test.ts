import { describe, expect, test } from 'bun:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import type { VersionConflictEntry } from '../src/desktopApi'
import { ReviewEntryActions, uploadReviewChoices } from '../src/pages/ReviewEntryActions'

const staleEntry: VersionConflictEntry = {
  id: 'op:op-stale-b',
  file_id: 'file-1',
  file_name: 'shared.txt',
  kind: 'stale_base',
  status: 'stale base kept local',
  updated_at: 1790359495,
  detail: 'Another device saved a newer version first.',
  action: 'review_upload',
  op_id: 'op-stale-b',
  version_id: null,
  base_version: 1,
  last_error: 'upload init rejected (409 Conflict): stale base version for replacement upload',
  resolutions: ['keep_both', 'keep_mine', 'discard'],
}

function render(entry: VersionConflictEntry, confirmingDiscard = false): string {
  return renderToStaticMarkup(
    createElement(ReviewEntryActions, {
      entry,
      busy: null,
      confirmingDiscard,
      onOpenConflict: () => {},
      onResolve: () => {},
    }),
  )
}

describe('version center review actions', () => {
  test('a review_upload entry renders Keep both / Keep mine / Discard buttons', () => {
    const html = render(staleEntry)
    expect((html.match(/<button/g) ?? []).length).toBe(3)
    expect(html).toContain('>Keep both</button>')
    expect(html).toContain('>Keep mine</button>')
    expect(html).toContain('>Discard</button>')
  })

  test('discard asks for a second click before deleting the queued edit', () => {
    expect(render(staleEntry, true)).toContain('>Confirm discard</button>')
  })

  test('conflict rows keep the Review button; entries without resolutions render nothing', () => {
    expect(render({ ...staleEntry, action: 'open_conflict', resolutions: [] })).toContain('>Review</button>')
    expect(render({ ...staleEntry, kind: 'metadata', resolutions: [] })).toBe('')
    expect(uploadReviewChoices({ ...staleEntry, resolutions: ['keep_both', 'rm_rf'] })).toEqual(['keep_both'])
  })
})
