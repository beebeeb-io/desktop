// The live PDF view (DW-1505, DW-1506): pdf.js draws the decrypted bytes into a canvas, page by page, with real
// thumbnails and find. No frame, no network: the worker is a bundled asset and the standard fonts are self-hosted.
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { GlobalWorkerOptions, type PDFDocumentProxy, type RenderTask } from 'pdfjs-dist'
import workerUrl from 'pdfjs-dist/build/pdf.worker.min.mjs?url'
import { FOCUS } from '../../files/ui'
import { FindBar, ToolbarButton } from '../Chrome'
import { Centered } from '../Viewers'
import { useOwnFind } from '../find'
import { allPageText, openPdf, renderPage, type PdfHandle } from './render'
import { findMatches } from './find'

// Vite emits the worker as one asset; `script-src 'self'` allows the same-origin worker, so the CSP stays as it is.
GlobalWorkerOptions.workerSrc = workerUrl

export function PdfDocument({ bytes, name, zoom, findOpen, onFindOpen, onPageChange, onOpenElsewhere }: {
  bytes: Uint8Array
  name: string
  zoom: number
  findOpen: boolean
  onFindOpen: (open: boolean) => void
  onPageChange?: (p: { page: number; total: number }) => void
  onOpenElsewhere?: () => void
}) {
  const [handle, setHandle] = useState<PdfHandle | null>(null)
  const [pages, setPages] = useState<string[]>([])
  const [page, setPage] = useState(0)
  const [error, setError] = useState(false)
  const [query, setQuery] = useState('')
  const [current, setCurrent] = useState(0)
  const canvasRef = useRef<HTMLCanvasElement>(null)

  // Parse the bytes once per open. pdf.js gets a copy (render.ts), so the window's own array stays whole.
  useEffect(() => {
    let live = true
    let opened: PdfHandle | null = null
    setHandle(null)
    setPages([])
    setPage(0)
    setError(false)
    openPdf(bytes).then(async (h) => {
      opened = h
      if (!live) { void h.destroy(); return }
      setHandle(h)
      const text = await allPageText(h.doc)
      if (live) setPages(text)
    }).catch(() => { if (live) setError(true) })
    return () => { live = false; if (opened) void opened.destroy() }
  }, [bytes])

  // The toolbar's page indicator (DW-1505#2): "Page 3 of 6".
  useEffect(() => {
    if (handle) onPageChange?.({ page: page + 1, total: handle.doc.numPages })
  }, [handle, page, onPageChange])

  // Draw the current page at zoom × devicePixelRatio, so a zoomed page is sharper, not a stretched bitmap.
  useEffect(() => {
    const canvas = canvasRef.current
    if (!handle || !canvas) return
    let cancelled = false
    let task: RenderTask | null = null
    const dpr = typeof window === 'undefined' ? 1 : window.devicePixelRatio || 1
    renderPage(handle.doc, page + 1, canvas, zoom * dpr).then(({ task: render, width, height }) => {
      task = render
      if (cancelled) { render.cancel(); return }
      canvas.style.width = `${Math.round(width / dpr)}px`
      canvas.style.height = `${Math.round(height / dpr)}px`
      return render.promise
    }).catch(() => { /* a cancelled render is not an error */ })
    return () => { cancelled = true; task?.cancel() }
  }, [handle, page, zoom])

  const matches = useMemo(() => findMatches(pages, query), [pages, query])
  const noText = pages.length > 0 && pages.every((text) => !text.trim())

  // A new query starts at its first match and jumps to that page (DW-1506).
  useEffect(() => {
    setCurrent(0)
    if (matches.length) setPage(matches[0].page)
  }, [matches])

  const go = (delta: number) => {
    if (!matches.length) return
    const next = (current + delta + matches.length) % matches.length
    setCurrent(next)
    setPage(matches[next].page)
  }

  // Ctrl+F is this view's find, never Search (find.ts).
  useOwnFind(true, useCallback(() => onFindOpen(true), [onFindOpen]))

  if (error) {
    return (
      <Centered>
        <p className="m-0 text-[13px] text-ink-2">{name} can’t be drawn in this window.</p>
        {onOpenElsewhere && <ToolbarButton label="Open in another app…" onClick={onOpenElsewhere} />}
      </Centered>
    )
  }
  if (!handle) return <Centered><p className="m-0 text-[13px] text-ink-3">Drawing the pages…</p></Centered>

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {findOpen && (
        <FindBar
          query={query}
          onQuery={setQuery}
          position={matches.length ? `${current + 1} of ${matches.length}` : query.trim() ? 'No matches' : ''}
          message={noText ? 'This PDF has no text to search (it’s a scan)' : undefined}
          onPrev={() => go(-1)}
          onNext={() => go(1)}
          onDone={() => onFindOpen(false)}
        />
      )}
      <div data-viewer="pdf" className="flex min-h-0 flex-1 bg-paper-3">
        <ol aria-label="Pages" className="m-0 flex w-32 flex-none list-none flex-col items-center gap-3 overflow-auto border-r border-line bg-paper-2 p-3">
          {Array.from({ length: handle.doc.numPages }, (_, i) => (
            <li key={i}><PageThumb doc={handle.doc} page={i} current={i === page} onPick={() => setPage(i)} /></li>
          ))}
        </ol>
        <div className="flex min-w-0 flex-1 justify-center overflow-auto p-6">
          <article data-pdf-page aria-label={`Page ${page + 1}`} className="flex-none bg-paper shadow-lg">
            <canvas ref={canvasRef} className="block" />
          </article>
        </div>
      </div>
    </div>
  )
}

/** One page thumbnail, 80 px wide, drawn from the same document; the current page is outlined (DW-1505#0). */
function PageThumb({ doc, page, current, onPick }: { doc: PDFDocumentProxy; page: number; current: boolean; onPick: () => void }) {
  const ref = useRef<HTMLCanvasElement>(null)
  useEffect(() => {
    const canvas = ref.current
    if (!canvas) return
    let cancelled = false
    let task: RenderTask | null = null
    doc.getPage(page + 1).then((p) => {
      if (cancelled) return
      const base = p.getViewport({ scale: 1 })
      const viewport = p.getViewport({ scale: 80 / base.width })
      canvas.width = Math.max(1, Math.floor(viewport.width))
      canvas.height = Math.max(1, Math.floor(viewport.height))
      task = p.render({ canvas, viewport })
      return task.promise
    }).catch(() => { /* a cancelled thumbnail is not an error */ })
    return () => { cancelled = true; task?.cancel() }
  }, [doc, page])
  return (
    <button type="button" data-thumb aria-current={current} aria-label={`Page ${page + 1}`} onClick={onPick} className={`flex w-20 flex-col items-center gap-1 ${FOCUS}`}>
      <span className={`flex h-24 w-20 items-center justify-center overflow-hidden rounded-sm bg-paper ${current ? 'outline outline-2 outline-ink' : 'outline outline-1 outline-line'}`}>
        <canvas ref={ref} className="max-h-full max-w-full" />
      </span>
      <span className="font-mono text-[11px] text-ink-3">{page + 1}</span>
    </button>
  )
}
