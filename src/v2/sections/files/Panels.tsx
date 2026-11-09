// Preview, no-preview, the uploads panel and the system file pickers drawn for reference (DW-0408, DW-0408b, DW-0415 to DW-0416, DW-0427, DW-0480, DW-0481).
import { useEffect, useRef, type ReactNode } from 'react'
import { Icon } from '@beebeeb/shared/design'
import { FileIcon, getFileType } from '@beebeeb/shared/drive-ui'
import { BTN, CloseX } from './ui'
import { fmtSize } from './model'

/** DW-0408: a preview panel over the window. */
export function PreviewPanel({ name, meta, children, onOpenInApp, onShare, onClose }: { name: string; meta?: string; children: ReactNode; onOpenInApp?: () => void; onShare?: () => void; onClose?: () => void }) {
  return (
    <div role="presentation" className="fixed inset-0 z-50 flex items-center justify-center bg-ink/25 p-10" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose?.() }}>
      <section role="dialog" aria-modal="true" aria-label={`Preview ${name}`} data-preview className="flex h-full max-h-[640px] w-[760px] max-w-full flex-col overflow-hidden rounded-xl border border-line bg-paper shadow-2xl">
        <header className="flex h-12 flex-none items-center gap-3 border-b border-line px-4">
          <FileIcon type={getFileType(name, false)} size={22} />
          <h2 className="m-0 min-w-0 truncate text-[14px] font-semibold text-ink">{name}</h2>
          {meta && <span className="font-mono text-[12px] text-ink-3">{meta}</span>}
          <span className="flex-1" />
          <button type="button" className={BTN} onClick={onOpenInApp}>Open in its app</button>
          <button type="button" className={BTN} onClick={onShare}>Share</button>
          <CloseX onClick={onClose} />
        </header>
        <div className="flex min-h-0 flex-1 items-start justify-center overflow-auto bg-paper-2 p-6">{children}</div>
      </section>
    </div>
  )
}

/** DW-0408b: say so, instead of a blank panel. Nothing is downloaded until a button is used. */
export function NoPreview({ name, size, onOpenElsewhere, onKeep, onClose }: { name: string; size: number; onOpenElsewhere?: () => void; onKeep?: () => void; onClose?: () => void }) {
  return (
    <div role="presentation" className="fixed inset-0 z-50 flex items-center justify-center bg-ink/25 p-10" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose?.() }}>
      <section role="dialog" aria-modal="true" aria-label={`Preview ${name}`} data-preview="none" className="flex w-[560px] max-w-full flex-col overflow-hidden rounded-xl border border-line bg-paper shadow-2xl">
        <header className="flex h-12 flex-none items-center gap-3 border-b border-line px-4">
          <h2 className="m-0 min-w-0 flex-1 truncate text-[14px] font-semibold text-ink">{name}</h2>
          <CloseX onClick={onClose} />
        </header>
        <div className="flex flex-col items-center gap-3 px-10 py-12 text-center">
          <div className="grid h-16 w-16 place-items-center rounded-2xl border border-line bg-paper-2 text-ink-3"><Icon name="file" size={28} /></div>
          <h3 className="m-0 text-[16px] font-semibold leading-6 text-ink">No preview for this kind of file</h3>
          <p className="m-0 max-w-[380px] text-[13px] leading-5 text-ink-3"><span className="font-mono text-[12px]">{name} · {fmtSize(size)}.</span> Open it in another app, or keep it on this PC to open it offline.</p>
          <div className="mt-2 flex gap-2">
            <button type="button" className={BTN} onClick={onOpenElsewhere}>Open in another app…</button>
            <button type="button" className={BTN} onClick={onKeep}>Make available offline</button>
          </div>
        </div>
      </section>
    </div>
  )
}

/** A page of a document, drawn from text, for the preview body. */
export function Page({ lines, title, compact }: { lines: string[]; title: string; compact?: boolean }) {
  return (
    <article aria-label="Page 1" data-page className={`${compact ? 'w-full p-4' : 'w-[520px] max-w-full flex-none p-10'} rounded-sm border border-line bg-page text-[12px] leading-5 text-page-ink shadow-sm`}>
      <h3 className="m-0 mb-4 text-[16px] font-semibold">{title}</h3>
      {lines.map((l, i) => <p key={i} className="m-0 mb-2">{l}</p>)}
    </article>
  )
}

export interface UploadRow { id: string; name: string; state: 'attention' | 'uploading' | 'waiting'; done?: number; total: number; note?: string; to?: string; continuing?: boolean }

/** DW-0427: Uploads. A panel over the status line. */
export function UploadsPanel({ rows, summary, onTryAgain, onPauseAll, onClose, onCancel }: { rows: UploadRow[]; summary: string; onTryAgain?: (id: string) => void; onPauseAll?: () => void; onClose?: () => void; onCancel?: (id: string) => void }) {
  const group = (state: UploadRow['state'], label: string) => {
    const list = rows.filter((r) => r.state === state)
    if (!list.length) return null
    return (
      <section aria-label={label} data-uploads-group={state}>
        <h3 className="m-0 px-4 pb-1 pt-3 text-[11px] font-medium uppercase tracking-wide text-ink-3">{label}</h3>
        <ul className="m-0 list-none p-0">
          {list.map((r) => (
            <li key={r.id} data-upload={r.id} onContextMenu={(e) => { e.preventDefault(); onCancel?.(r.id) }} className="px-4 py-1.5 text-[13px]">
              <div className="flex items-center gap-2">
                <span className="min-w-0 flex-1 truncate text-ink">{r.name}</span>
                {r.state === 'attention' && <button type="button" className={`${BTN} h-7 px-3`} onClick={() => onTryAgain?.(r.id)}>Try again</button>}
                {r.state === 'uploading' && <span className="font-mono text-[12px] text-ink-3">{r.continuing ? `Continuing, ${((r.done ?? 0) / 1e6).toFixed(0)} of ${(r.total / 1e6).toFixed(0)} MB` : `${((r.done ?? 0) / 1e6).toFixed(1)} of ${(r.total / 1e6).toFixed(1)} MB`}</span>}
                {r.state === 'waiting' && <span className="font-mono text-[12px] text-ink-3">{fmtSize(r.total)} · To {r.to}</span>}
              </div>
              {r.state === 'attention' && <div className="text-[12px] leading-4 text-ink-3">{r.note}</div>}
              {r.state === 'uploading' && (
                <div role="progressbar" aria-label={`Uploading ${r.name}`} aria-valuenow={Math.round(((r.done ?? 0) / r.total) * 100)} aria-valuemin={0} aria-valuemax={100} className="mt-1 h-1 overflow-hidden rounded-full bg-paper-3">
                  <div className="h-full bg-amber" style={{ width: `${((r.done ?? 0) / r.total) * 100}%` }} />
                </div>
              )}
            </li>
          ))}
        </ul>
      </section>
    )
  }
  // Esc or a click outside closes it (DW-0427), wherever the focus is; the status line's own click is its toggle.
  const panel = useRef<HTMLElement>(null)
  useEffect(() => {
    if (!onClose) return
    const key = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose() }
    const down = (e: MouseEvent) => {
      const t = e.target as Node
      if (panel.current?.contains(t) || (t instanceof Element && t.closest('[data-open-uploads]'))) return
      onClose()
    }
    window.addEventListener('keydown', key)
    window.addEventListener('mousedown', down)
    return () => { window.removeEventListener('keydown', key); window.removeEventListener('mousedown', down) }
  }, [onClose])
  return (
    <div role="presentation" className="absolute bottom-8 left-3 z-40 w-[440px]">
      <section ref={panel} role="dialog" aria-label="Uploads" data-panel="uploads" className="overflow-hidden rounded-xl border border-line bg-paper shadow-2xl">
        <header className="flex items-start justify-between gap-3 border-b border-line px-4 py-3">
          <div><h2 className="m-0 text-[14px] font-semibold text-ink">Uploads</h2><p className="m-0 text-[12px] leading-4 text-ink-3">{summary}</p></div>
          <CloseX onClick={onClose} />
        </header>
        {group('attention', 'Needs attention')}
        {group('uploading', 'Uploading')}
        {group('waiting', 'Waiting')}
        <footer className="flex justify-end border-t border-line px-4 py-2.5"><button type="button" className={BTN} onClick={onPauseAll}>Pause all</button></footer>
      </section>
    </div>
  )
}

/** The drag badge of DW-0481. */
export function DragBadge({ count, target, x, y }: { count: number; target: string; x: number; y: number }) {
  return (
    <div data-drag-badge className="pointer-events-none fixed z-50 flex flex-col gap-1" style={{ left: x, top: y }}>
      <span className="inline-flex w-fit items-center rounded-md bg-ink px-2 py-1 font-mono text-[12px] text-paper shadow-lg">{count} items</span>
      <span className="inline-flex w-fit items-center rounded-md border border-line bg-paper px-2 py-1 text-[12px] text-ink shadow-lg">Upload to {target}</span>
    </div>
  )
}

type PickerKind = 'files' | 'photos' | 'folder'

const NAV = ['Home', 'Gallery', 'Desktop', 'Downloads', 'Documents', 'Pictures', 'Music', 'Videos', 'This PC', 'Network']

/** DW-0415, DW-0416, DW-0480: Windows draws these (system greys); Beebeeb sets only the title and the button's label. */
export function SystemPicker({ kind, title, button }: { kind: PickerKind; title: string; button: string }) {
  const where = kind === 'photos' ? ['This PC', 'Pictures'] : kind === 'folder' ? ['This PC', 'Documents'] : ['This PC', 'Documents']
  const files = kind === 'files' ? ['Contract draft.pdf', 'Budget 2026.xlsx', 'Service agreement.pdf'] : kind === 'folder' ? ['Projects', 'Receipts', 'Travel'] : ['Alfama.heic', 'Belém tower.heic', 'Miradouro.heic', 'Ponte.heic', 'Harbour.heic', 'Tram 28.jpg']
  const picked = kind === 'files' ? 3 : kind === 'folder' ? 1 : 3
  return (
    <div className="fixed inset-0 z-50 grid place-items-center bg-ink/25">
      <div role="dialog" aria-label={title} data-system-ui={kind} className="flex h-[480px] w-[800px] flex-col overflow-hidden rounded-lg border border-line bg-win-surface text-[12px] text-win-ink shadow-2xl">
        <div className="flex h-8 flex-none items-center justify-between bg-white px-3"><span>{title}</span><span className="text-win-ink-2">✕</span></div>
        <div className="flex h-10 flex-none items-center gap-2 border-b border-win-line bg-white px-3">
          <span className="flex h-7 flex-1 items-center gap-1.5 rounded border border-win-field px-2">{where.map((w, i) => <span key={w} className="flex items-center gap-1.5">{i > 0 && '›'}{w}</span>)}</span>
          <span className="flex h-7 w-52 items-center rounded border border-win-field px-2 text-win-ink-2">Search {where[1]}</span>
        </div>
        <div className="flex min-h-0 flex-1 bg-white">
          <ul className="m-0 w-44 flex-none list-none border-r border-win-line p-2">
            {NAV.map((n) => <li key={n} className={`rounded px-2 py-1.5 ${n === where[1] ? 'bg-win-selected' : ''}`}>{n}</li>)}
          </ul>
          <div className={`min-w-0 flex-1 p-3 ${kind === 'photos' ? 'grid grid-cols-3 content-start gap-3' : ''}`}>
            {files.map((f, i) => {
              const on = i < picked && kind !== 'folder' ? true : kind === 'folder' && i === 0
              return kind === 'photos'
                ? <div key={f} data-picker-item className={`flex flex-col items-center gap-1 rounded p-2 ${on ? 'bg-win-selected outline outline-1 outline-win-selected-line' : ''}`}><div className="aspect-[4/3] w-full rounded bg-sample-thumb" /><span className="truncate">{f}</span></div>
                : <div key={f} data-picker-item className={`flex h-8 items-center gap-2 rounded px-2 ${on ? 'bg-win-selected' : ''}`}><FileIcon type={getFileType(f, kind === 'folder')} size={18} />{f}</div>
            })}
          </div>
        </div>
        <div className="flex flex-none flex-col gap-2 border-t border-win-line bg-win-surface p-3">
          <div className="flex items-center gap-2"><span className="w-20">{kind === 'folder' ? 'Folder:' : 'File name:'}</span><span className="flex h-7 flex-1 items-center rounded border border-win-field bg-white px-2">{kind === 'folder' ? 'Projects' : files.slice(0, picked).map((f) => `"${f}"`).join(' ')}</span></div>
          <div className="flex justify-end gap-2"><span data-picker-ok className="inline-flex h-7 min-w-24 items-center justify-center rounded border border-win-button-line bg-win-selected px-3">{button}</span><span className="inline-flex h-7 min-w-24 items-center justify-center rounded border border-win-field bg-white px-3">Cancel</span></div>
        </div>
      </div>
    </div>
  )
}

