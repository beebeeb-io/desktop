// DW-0408 (Files › Space): the preview shows the selected file's own text. A text file whose bytes arrive renders that
// text inside the dialog named after the file; a text file whose read fails says there is no preview instead of drawing
// an empty panel or another file's text.
import { describe, expect, test } from 'bun:test'
import { createElement, Fragment } from 'react'
import { NoPreview, PreviewPanel } from '../src/v2/sections/files/Panels'
import { mount, textOf } from './fixtures/componentHarness'

const TEXT = {
  id: 'i-quarterly', parentId: null, name: 'quarterly.txt', isFolder: false, size: 42,
  modifiedAt: '2026-11-01T10:00:00', shared: { kind: 'none' }, local: 'downloaded',
  hasThumbnail: false, mime: 'text/plain',
}

function hostFor(item: Record<string, unknown>, readText: (id: string) => Promise<{ text: string; versionId: string }>) {
  const calls: string[] = []
  const source = {
    async readText(id: string) { calls.push(id); return readText(id) },
    async thumbnail() { calls.push('thumbnail'); return 'data:image/svg+xml,planted' },
  }
  const m = mount('v2/sections/files/FolderView.tsx', 'PreviewHost', {
    backend: {},
    bindings: { useDriveSource: () => source, NoPreview, PreviewPanel, React: { createElement, Fragment } },
    props: { item, items: [item], onClose: () => {}, onSwitch: () => {}, onOpen: () => {}, onShare: () => {}, onKeep: () => {} },
    expand: true,
    honestRender: true,
  })
  return { m, calls }
}

describe('DW-0408: the live Files route previews the selected text file', () => {
  test('Space on quarterly.txt names that file and draws its own text, reading it once', async () => {
    const { m, calls } = hostFor(TEXT, async () => ({ text: 'quarterly line 1', versionId: 'v1' }))
    try {
      await m.flush()
      const dialog = m.elements().find((el) => el.props.role === 'dialog')
      expect(dialog?.props['aria-label']).toBe('Preview quarterly.txt')
      const pre = m.elements().find((el) => el.type === 'pre')
      expect(textOf(pre?.props.children)).toBe('quarterly line 1')
      expect(calls).toEqual(['i-quarterly'])
    } finally {
      m.unmount()
      m.close()
    }
  })

  test('a text file whose read fails says there is no preview, never a blank panel', async () => {
    const { m, calls } = hostFor(TEXT, async () => { throw new Error('bytes did not arrive') })
    try {
      await m.flush()
      const dialog = m.elements().find((el) => el.props.role === 'dialog')
      expect(dialog?.props['aria-label']).toBe('Preview quarterly.txt')
      const body = textOf(m.elements())
      expect(body).toContain('No preview for this kind of file')
      expect(m.elements().some((el) => el.type === 'pre')).toBe(false)
      expect(calls).toEqual(['i-quarterly'])
    } finally {
      m.unmount()
      m.close()
    }
  })
})
