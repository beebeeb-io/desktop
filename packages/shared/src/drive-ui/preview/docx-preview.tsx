/**
 * DOCX preview — client-side rendering via mammoth (lazy-loaded).
 * Decrypted blob is converted to HTML and rendered in a sandboxed iframe
 * so the generated markup is fully isolated from the app shell.
 */

import { useEffect, useState } from 'react'
import { Icon } from '../../design'
import { BBButton } from '../../design'

// ── Preview banner ────────────────────────────────────────────────────────────

function PreviewBanner({ onDownload }: { onDownload: () => void }) {
  return (
    <div className="flex items-center gap-3 px-4 py-2 bg-paper-2 border-b border-line shrink-0">
      <Icon name="eye" size={12} className="text-ink-3 shrink-0" />
      <span className="text-[11.5px] text-ink-3 flex-1">
        Preview — some formatting may differ from the original.
      </span>
      <button
        onClick={onDownload}
        className="shrink-0 inline-flex items-center gap-1.5 text-[11.5px] font-medium text-amber-deep hover:underline"
      >
        <Icon name="download" size={11} />
        Download for full fidelity
      </button>
    </div>
  )
}

// ── Component ─────────────────────────────────────────────────────────────────

interface DocxPreviewProps {
  blob: Blob
  filename: string
}

// Minimal CSS injected into the iframe so the document looks reasonable.
const IFRAME_STYLES = `
  body {
    font-family: Inter, -apple-system, sans-serif;
    font-size: 14px;
    line-height: 1.6;
    color: #1a1714;
    max-width: 760px;
    margin: 0 auto;
    padding: 32px 24px;
    background: #fff;
  }
  h1, h2, h3, h4, h5, h6 { font-weight: 600; margin: 1.25em 0 0.5em; line-height: 1.3; }
  h1 { font-size: 1.75em; }
  h2 { font-size: 1.35em; }
  h3 { font-size: 1.15em; }
  p { margin: 0.6em 0; }
  table { border-collapse: collapse; width: 100%; margin: 1em 0; }
  th, td { border: 1px solid #ddd; padding: 6px 10px; text-align: left; font-size: 13px; }
  th { background: #f5f5f5; font-weight: 600; }
  ul, ol { padding-left: 1.5em; }
  li { margin: 0.3em 0; }
  a { color: #c98a00; }
  img { max-width: 100%; }
  pre, code { font-family: 'JetBrains Mono', monospace; font-size: 12px; background: #f5f5f5; padding: 2px 4px; border-radius: 3px; }
  pre { padding: 12px; overflow-x: auto; }
`.trim()

export function DocxPreview({ blob, filename }: DocxPreviewProps) {
  const [srcDoc, setSrcDoc] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false

    async function render() {
      try {
        const arrayBuffer = await blob.arrayBuffer()
        // Lazy-load mammoth — only fetched when a docx file is opened
        const mammoth = await import('mammoth')
        const result = await mammoth.convertToHtml({ arrayBuffer })

        if (cancelled) return

        // Wrap in a minimal HTML document with scoped styles
        const doc = `<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <style>${IFRAME_STYLES}</style>
</head>
<body>${result.value}</body>
</html>`
        setSrcDoc(doc)
      } catch (err) {
        if (!cancelled) setError(err instanceof Error ? err.message : 'Failed to render document')
      }
    }

    render()
    return () => { cancelled = true }
  }, [blob])

  function handleDownload() {
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = filename
    a.click()
    URL.revokeObjectURL(url)
  }

  if (error) {
    return (
      <div className="flex flex-col items-center gap-4 text-center py-16">
        <Icon name="file-text" size={36} className="text-ink-3" />
        <p className="text-sm text-ink-2">Could not render this document.</p>
        <p className="text-xs text-ink-3 font-mono">{error}</p>
        <BBButton variant="amber" size="md" onClick={handleDownload}>
          <Icon name="download" size={14} className="mr-1.5" />
          Download
        </BBButton>
      </div>
    )
  }

  if (!srcDoc) {
    return (
      <div className="flex flex-col items-center gap-3 py-16">
        <div className="h-6 w-6 animate-spin rounded-full border-2 border-line border-t-amber" />
        <span className="text-sm text-ink-3">Rendering document…</span>
      </div>
    )
  }

  return (
    <div className="flex flex-col h-full w-full">
      <PreviewBanner onDownload={handleDownload} />
      {/* No allow-same-origin: static srcDoc HTML does not need origin access. */}
      <iframe
        srcDoc={srcDoc}
        sandbox="allow-popups"
        className="flex-1 w-full border-0 bg-white"
        title={`Preview of ${filename}`}
      />
    </div>
  )
}
