/**
 * XLSX preview — client-side rendering via SheetJS (lazy-loaded).
 * Sheets are converted to HTML tables and rendered with sheet tab navigation.
 */

import { useEffect, useState } from 'react'
import DOMPurify from 'dompurify'
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

// ── Sheet tab bar ─────────────────────────────────────────────────────────────

function SheetTabs({
  names,
  active,
  onChange,
}: {
  names: string[]
  active: number
  onChange: (i: number) => void
}) {
  if (names.length <= 1) return null
  return (
    <div className="flex gap-0 border-t border-line bg-paper-2 shrink-0 overflow-x-auto">
      {names.map((name, i) => (
        <button
          key={name}
          onClick={() => onChange(i)}
          className={`px-3.5 py-2 text-[12px] border-r border-line whitespace-nowrap transition-colors ${
            i === active
              ? 'bg-paper font-semibold text-ink border-b-2 border-b-amber -mb-px'
              : 'text-ink-3 hover:text-ink-2 hover:bg-paper'
          }`}
        >
          {name}
        </button>
      ))}
    </div>
  )
}

// ── Spreadsheet styles injected into table iframe ─────────────────────────────

const TABLE_STYLES = `
  body { margin: 0; padding: 0; background: #fff; }
  table {
    border-collapse: collapse;
    width: 100%;
    font-family: 'JetBrains Mono', monospace;
    font-size: 12px;
  }
  th {
    background: #f5f4f2;
    font-weight: 600;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: #888;
    position: sticky;
    top: 0;
    z-index: 1;
  }
  th, td {
    border: 1px solid #e8e6e3;
    padding: 5px 8px;
    white-space: nowrap;
    max-width: 320px;
    overflow: hidden;
    text-overflow: ellipsis;
    color: #1a1714;
  }
  tr:nth-child(even) td { background: #faf9f7; }
  tr:hover td { background: #fff4da; }
`.trim()

// ── Component ─────────────────────────────────────────────────────────────────

interface SheetData {
  name: string
  html: string
}

interface XlsxPreviewProps {
  blob: Blob
  filename: string
}

export function XlsxPreview({ blob, filename }: XlsxPreviewProps) {
  const [sheets, setSheets] = useState<SheetData[] | null>(null)
  const [activeSheet, setActiveSheet] = useState(0)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false

    async function render() {
      try {
        const arrayBuffer = await blob.arrayBuffer()
        // Lazy-load xlsx (community-maintained @e965/xlsx fork) — only fetched when a spreadsheet is opened
        const XLSX = await import('@e965/xlsx')
        const workbook = XLSX.read(new Uint8Array(arrayBuffer), { type: 'array' })

        const parsed: SheetData[] = workbook.SheetNames.map(name => ({
          name,
          html: XLSX.utils.sheet_to_html(workbook.Sheets[name]),
        }))

        if (!cancelled) {
          setSheets(parsed)
          setActiveSheet(0)
        }
      } catch (err) {
        if (!cancelled) setError(err instanceof Error ? err.message : 'Failed to render spreadsheet')
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
        <Icon name="file-spreadsheet" size={36} className="text-ink-3" />
        <p className="text-sm text-ink-2">Could not render this spreadsheet.</p>
        <p className="text-xs text-ink-3 font-mono">{error}</p>
        <BBButton variant="amber" size="md" onClick={handleDownload}>
          <Icon name="download" size={14} className="mr-1.5" />
          Download
        </BBButton>
      </div>
    )
  }

  if (!sheets) {
    return (
      <div className="flex flex-col items-center gap-3 py-16">
        <div className="h-6 w-6 animate-spin rounded-full border-2 border-line border-t-amber" />
        <span className="text-sm text-ink-3">Rendering spreadsheet…</span>
      </div>
    )
  }

  const currentSheet = sheets[activeSheet]
  const sanitizedHtml = currentSheet ? DOMPurify.sanitize(currentSheet.html) : ''
  const srcDoc = currentSheet
    ? `<!DOCTYPE html><html><head><meta charset="utf-8"><style>${TABLE_STYLES}</style></head><body>${sanitizedHtml}</body></html>`
    : ''

  return (
    <div className="flex flex-col h-full w-full">
      <PreviewBanner onDownload={handleDownload} />

      {/* Sheet iframe — scrollable */}
      <div className="flex-1 overflow-hidden">
        {/* No allow-same-origin: static srcDoc HTML does not need origin access. */}
        <iframe
          key={activeSheet}
          srcDoc={srcDoc}
          sandbox="allow-popups"
          className="w-full h-full border-0 bg-white"
          title={`${currentSheet?.name ?? 'Sheet'} — ${filename}`}
        />
      </div>

      {/* Sheet tabs at bottom */}
      <SheetTabs
        names={sheets.map(s => s.name)}
        active={activeSheet}
        onChange={setActiveSheet}
      />
    </div>
  )
}
