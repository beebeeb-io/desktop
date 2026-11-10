// PDF pages draw from the real file (P1b, M3d fork 1): pdf.js parses the decrypted bytes, the window draws
// real pages, and find counts every match on every page. The fixture is written by make.mjs, never downloaded.
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { DriveSourceError } from '@beebeeb/shared/drive-ui'
import { READ_CAP_BYTES, isTooLarge } from '../src/v2/source/desktop'
import { allPageText, openPdf, overReadCap } from '../src/v2/sections/open/pdf/render'
import { findMatches } from '../src/v2/sections/open/pdf/find'

const ROOT = join(import.meta.dir, '..')
const FIXTURE = join(ROOT, 'tests/fixtures/pdf/two-pages.pdf')
const source = (rel: string) => readFileSync(join(ROOT, rel), 'utf8')

/** The bytes the window would get from source.readBytes: a copy, because pdf.js detaches its buffer. */
const fixtureBytes = () => new Uint8Array(readFileSync(FIXTURE))

describe('PDF pages draw from the real file', () => {
  test('(a) a PDF opens with its real page count and text', async () => {
    const handle = await openPdf(fixtureBytes())
    try {
      expect(handle.doc.numPages).toBe(2)
      const pages = await allPageText(handle.doc)
      expect(pages).toHaveLength(2)
      expect(pages[0]).toContain('The terrace is open.')
      expect(pages[0]).toContain('Terrace view from the room.')
      expect(pages[1]).toContain('The terrace is closed today.')
      expect(pages[1]).not.toContain('Terrace view')
    } finally {
      await handle.destroy()
    }
  })

  test('(b) find counts every match on every page', async () => {
    const handle = await openPdf(fixtureBytes())
    try {
      const pages = await allPageText(handle.doc)
      const matches = findMatches(pages, 'terrace')
      expect(matches).toHaveLength(3)
      expect(matches.map((m) => m.page)).toEqual([0, 0, 1])
    } finally {
      await handle.destroy()
    }
  })

  test('(c) find is case-insensitive', async () => {
    const handle = await openPdf(fixtureBytes())
    try {
      const pages = await allPageText(handle.doc)
      expect(findMatches(pages, 'TERRACE')).toHaveLength(3)
      expect(findMatches(pages, 'Terrace')).toHaveLength(3)
      expect(findMatches(pages, 'terrace').map((m) => m.page)).toEqual(findMatches(pages, 'TERRACE').map((m) => m.page))
    } finally {
      await handle.destroy()
    }
  })

  test('(d) a file over READ_CAP_BYTES takes the too-large state, not “couldn’t decrypt”', () => {
    expect(READ_CAP_BYTES).toBe(256 * 1024 * 1024)
    expect(overReadCap(READ_CAP_BYTES + 1)).toBe(true)
    expect(overReadCap(READ_CAP_BYTES)).toBe(false)
    expect(overReadCap(null)).toBe(false)
    expect(isTooLarge(new DriveSourceError('unsupported', 'too large'))).toBe(true)
    expect(isTooLarge(new DriveSourceError('failed', 'drive_read_bytes'))).toBe(false)
    const open = source('src/v2/sections/open/index.tsx')
    expect(open).toContain('overReadCap(item.size)')
    expect(open).toMatch(/isTooLarge\(e\)[\s\S]{0,200}name: 'too-large'/)
  })

  test('(e) the live PDF branch renders PdfDocument and never the “opens in another app” fallback', () => {
    const open = source('src/v2/sections/open/index.tsx')
    expect(open).toContain("lazy(() => import('./pdf/PdfDocument')")
    expect(open).toContain("else if (kind === 'pdf') body = bytes ? <Suspense")
    expect(open).toContain('<PdfDocument bytes={bytes}')
    expect(open).not.toContain("kind === 'other' || kind === 'raw' || kind === 'pdf' ? { kind: 'none' }")
    expect(open).toContain("kind === 'other' || kind === 'raw' ? { kind: 'none' }")
    const pdf = open.indexOf("else if (kind === 'pdf') body = bytes ? <Suspense")
    const fallback = open.indexOf('opens in another app.')
    expect(pdf).toBeGreaterThan(-1)
    expect(fallback).toBeGreaterThan(pdf)
  })

  test('(f) the worker is a bundled asset and no blob: iframe remains, with a planted positive control', () => {
    /** A scanner for a decrypted frame: an iframe whose src is a blob: URL (literal or a blob-named expression). */
    const blobFrames = (src: string) => [...src.matchAll(/<iframe[^>]*\bsrc=(?:\{[^}]*blob[^}]*\}|["'`]blob:[^"'`]*["'`])/gi)].map((m) => m[0])
    /** A scanner for a Vite-bundled asset reference: an import ending in `?url`. */
    const urlAssets = (src: string) => [...src.matchAll(/from\s+['"]([^'"]+\?url)['"]/g)].map((m) => m[1])

    // The scanners prove they can fail: a planted frame and a planted asset reference are both found.
    expect(blobFrames('<iframe src={blobUrl} title="Document" />')).toHaveLength(1)
    expect(urlAssets("import workerUrl from 'pdfjs-dist/build/pdf.worker.min.mjs?url'")).toEqual(['pdfjs-dist/build/pdf.worker.min.mjs?url'])

    const pdf = source('src/v2/sections/open/pdf/PdfDocument.tsx')
    expect(urlAssets(pdf)).toEqual(['pdfjs-dist/build/pdf.worker.min.mjs?url'])
    expect(pdf).toContain('GlobalWorkerOptions.workerSrc = workerUrl')
    for (const rel of ['src/v2/sections/open/index.tsx', 'src/v2/sections/open/pdf/PdfDocument.tsx', 'src/v2/sections/open/Viewers.tsx']) {
      expect({ rel, frames: blobFrames(source(rel)) }).toEqual({ rel, frames: [] })
    }
  })
})
