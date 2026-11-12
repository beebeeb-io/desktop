// Task 2445: an account whose files have not been loaded on this PC is not an empty folder. The rule and the words
// are Rust's (src-tauri/src/drive_v2/first_sync.rs); the window only shows what `drive_list` answers.
import { describe, expect, test } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { filesEmptyState } from '../src/v2/chrome/EmptyStates'

const base = { folder: { id: null, name: 'Files' }, path: [], items: [], asOf: '2026-11-11T11:30:00Z', stale: false }
const paused = { reason: 'paused', title: 'Sync is paused', detail: 'Your files have not been loaded on this PC yet.', action: 'Resume' }
const html = (listing: unknown, addable = true) => renderToStaticMarkup(<>{filesEmptyState({ listing, addable })}</>)

describe('Files, no rows', () => {
  test('never synced and paused: the notice with Resume, never the empty-folder message', () => {
    const out = html({ ...base, whyEmpty: 'never-synced', neverSynced: paused })
    expect(out).toContain('data-empty="never-synced"')
    expect(out).toContain('data-reason="paused"')
    expect(out).toContain('Sync is paused')
    expect(out).toContain('Your files have not been loaded on this PC yet.')
    expect(out).toContain('>Resume</button>')
    expect(out).not.toContain('Nothing here yet')
    expect(out).not.toContain('Add files')
  })

  test('never synced with no action (still loading): no button', () => {
    const out = html({ ...base, whyEmpty: 'never-synced', neverSynced: { reason: 'checking', title: 'Getting your files ready', detail: 'Your file list is downloading and being decrypted on this PC.', action: null } })
    expect(out).toContain('Getting your files ready')
    expect(out).not.toContain('<button')
    expect(out).not.toContain('Nothing here yet')
  })

  test('a loaded, empty folder keeps the empty-folder message', () => {
    for (const listing of [{ ...base, whyEmpty: 'empty-folder', neverSynced: null }, base]) {
      const out = html(listing)
      expect(out).toContain('data-empty="files"')
      expect(out).toContain('Nothing here yet')
      expect(out).not.toContain('data-empty="never-synced"')
    }
  })

  test('a loaded, empty folder that cannot take files shows neither', () => {
    expect(html({ ...base, neverSynced: null }, false)).toBe('')
  })
})
