// pdf.js reads the decrypted PDF bytes in the window: no frame, no network, no server. The worker URL is set by
// PdfDocument.tsx from Vite's bundled asset; this module holds the parsing, text and read-cap logic the tests drive.
import { getDocument, type PDFDocumentProxy, type RenderTask } from 'pdfjs-dist'
import { READ_CAP_BYTES } from '../../../source/desktop'

/** The self-hosted standard fonts (public/pdfjs/standard_fonts, Apache-2.0): pdf.js never fetches them from a CDN. */
export const STANDARD_FONT_DATA_URL = '/pdfjs/standard_fonts/'

/** A parsed PDF and the loading task that owns its worker; `destroy` ends both. */
export interface PdfHandle {
  doc: PDFDocumentProxy
  destroy: () => Promise<void>
}

/**
 * Parse `bytes` with pdf.js. The caller's array is copied: pdf.js transfers (detaches) the buffer it is given, and
 * the window may still need its own copy. `useWorkerFetch: false` keeps it offline; pdfjs-dist 6 has no eval path
 * (the old `isEvalSupported` option is gone), so nothing here evaluates a string as code.
 */
export async function openPdf(bytes: Uint8Array): Promise<PdfHandle> {
  const task = getDocument({
    data: bytes.slice(),
    useWorkerFetch: false,
    standardFontDataUrl: STANDARD_FONT_DATA_URL,
  })
  const doc = await task.promise
  return { doc, destroy: () => task.destroy() }
}

/** One page's text, items joined with a space and whitespace collapsed, as find reads it. */
export async function pageText(doc: PDFDocumentProxy, pageNumber: number): Promise<string> {
  const page = await doc.getPage(pageNumber)
  const content = await page.getTextContent()
  return content.items
    .map((item) => ('str' in item ? item.str : ''))
    .join(' ')
    .replace(/\s+/g, ' ')
    .trim()
}

/** Every page's text, in page order (1-based pages, 0-based array). */
export async function allPageText(doc: PDFDocumentProxy): Promise<string[]> {
  const pages: string[] = []
  for (let page = 1; page <= doc.numPages; page += 1) pages.push(await pageText(doc, page))
  return pages
}

/** A file over Rust's read cap (drive_read_bytes) is the too-large state before any read (DW-1517). */
export const overReadCap = (size: number | null | undefined) => (size ?? 0) > READ_CAP_BYTES

/**
 * Draw one page into `canvas` at `scale` (zoom × devicePixelRatio). The returned task can be cancelled when the
 * page or zoom changes before it finishes; the canvas is sized in device pixels, the caller sets its CSS size.
 */
export async function renderPage(doc: PDFDocumentProxy, pageNumber: number, canvas: HTMLCanvasElement, scale: number): Promise<{ task: RenderTask; width: number; height: number }> {
  const page = await doc.getPage(pageNumber)
  const viewport = page.getViewport({ scale })
  canvas.width = Math.max(1, Math.floor(viewport.width))
  canvas.height = Math.max(1, Math.floor(viewport.height))
  const task = page.render({ canvas, viewport })
  return { task, width: viewport.width, height: viewport.height }
}
