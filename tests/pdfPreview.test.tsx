// DW-0408 (Files › Space): the preview is the file that was selected. Service agreement.pdf has no page renderer in
// this build, so its body is the honest NoPreview (DW-0408b), never another file's text and never a thumbnail standing
// in for a page. Meeting notes.md keeps its own text.
import { describe, expect, test } from 'bun:test'
import { createElement, Fragment } from 'react'
import { NoPreview, PreviewPanel } from '../src/v2/sections/files/Panels'
import { FILES_STATES } from '../src/v2/dev/states-files'
import { mount, textOf } from './fixtures/componentHarness'

const PDF = {
  id: 'i-agreement', parentId: null, name: 'Service agreement.pdf', isFolder: false, size: 640_000,
  modifiedAt: '2026-10-12T14:05:00', shared: { kind: 'none' }, local: 'available-offline',
  hasThumbnail: true, storageCountry: 'DE', mime: 'application/pdf',
}

function hostFor(item: Record<string, unknown>) {
  const calls: string[] = []
  const source = {
    async readText() { calls.push('readText'); return { text: '# Meeting notes\n\n- Desktop v2 board review\n', versionId: 'v3' } },
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

describe('DW-0408: the live Files route previews the PDF the board names', () => {
  test('DW-0408 selects Service agreement.pdf, not the markdown stand-in', () => {
    const recipe = FILES_STATES['DW-0408']
    expect(recipe.steps).toEqual([{ click: 'row:i-agreement' }, { key: 'Space' }])
    expect(recipe.gap ?? '').toContain('PDF pages')
  })

  test('Space on a PDF names that PDF and draws NoPreview, reading no text or thumbnail', async () => {
    const { m, calls } = hostFor(PDF)
    try {
      await m.flush()
      const dialog = m.elements().find((el) => el.props.role === 'dialog')
      expect(dialog?.props['aria-label']).toBe('Preview Service agreement.pdf')
      const body = textOf(m.elements())
      expect(body).toContain('No preview for this kind of file')
      expect(body).toContain('Open in another app…')
      expect(body).not.toContain('Desktop v2 board review')
      expect(calls).toEqual([])
    } finally {
      m.unmount()
      m.close()
    }
  })
})
