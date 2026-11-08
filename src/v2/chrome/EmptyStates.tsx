// Empty and loading states of the Files area (DW-0114, DW-0118, DW-0119, DW-0121). No spinners: a file list never
// shows a loading state, because it is already on this PC.

import type { ReactNode } from 'react'

function Tile({ children }: { children: ReactNode }) {
  return <div className="grid h-16 w-16 place-items-center rounded-2xl border border-line bg-paper-2 text-ink-3">{children}</div>
}

const outline = { width: 28, height: 28, viewBox: '0 0 28 28', fill: 'none', stroke: 'currentColor', strokeWidth: 1.4, strokeLinecap: 'round', strokeLinejoin: 'round', 'aria-hidden': true } as const

export function FolderTile() {
  return <Tile><svg {...outline}><path d="M3.5 8.5a2 2 0 0 1 2-2h5l2.5 3h9a2 2 0 0 1 2 2v9.5a2 2 0 0 1-2 2h-16.5a2 2 0 0 1-2-2z" /></svg></Tile>
}

export function CloudTile() {
  return <Tile><svg {...outline}><path d="M8 21h11.5a4.5 4.5 0 0 0 .7-8.9A6.5 6.5 0 0 0 7.7 10.7 5.2 5.2 0 0 0 8 21z" /></svg></Tile>
}

/** DW-0114: a new account has no files yet: one way forward. Add files is not amber here: New in the toolbar is
 *  the screen's one amber action. */
export function EmptyFiles({ onAddFiles }: { onAddFiles?: () => void }) {
  return (
    <div data-empty="files" className="flex flex-1 flex-col items-center justify-center gap-3 px-8 text-center">
      <FolderTile />
      <h2 className="m-0 text-[16px] font-semibold leading-6 text-ink">Nothing here yet</h2>
      <p className="m-0 max-w-[340px] text-[13px] leading-5 text-ink-3">Add files or photos. They're encrypted on this PC before they upload.</p>
      <button type="button" onClick={onAddFiles} className="h-8 rounded-md border border-line bg-paper px-4 text-[13px] font-medium text-ink hover:bg-paper-2 focus-visible:outline-2 focus-visible:outline-ink-3">Add files</button>
      <p className="m-0 text-[12px] leading-4 text-ink-3">or drop files here from File Explorer</p>
    </div>
  )
}

export interface ListLoading { done: number; total: number; minutesLeft: number; thumbnails: 'waiting' | 'in-progress' | 'done' }

const nf = new Intl.NumberFormat('en-US')

/** DW-0118: the slim strip above the list after the first sign-in; the window works at once. */
export function GettingReadyStrip({ loading }: { loading: ListLoading }) {
  const pct = Math.min(100, Math.round((loading.done / Math.max(1, loading.total)) * 100))
  return (
    <div data-strip="getting-ready" role="status" className="flex flex-none items-center gap-4 border-b border-line bg-paper-2 px-4 py-2 text-[12px] leading-4 text-ink-3">
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline gap-2">
          <span className="text-[13px] font-medium text-ink">Getting your files ready</span>
          <span className="font-mono">{nf.format(loading.done)} of {nf.format(loading.total)}</span>
          <span>· about {loading.minutesLeft} min left</span>
        </div>
        <div className="mt-1.5 h-1 overflow-hidden rounded-full bg-paper-3" role="progressbar" aria-valuenow={pct} aria-valuemin={0} aria-valuemax={100} aria-label="File list">
          <div className="h-full bg-ink-3" style={{ width: `${pct}%` }} />
        </div>
      </div>
      <dl className="m-0 flex flex-none gap-5">
        <div><dt className="inline">File list</dt> <dd className="m-0 inline text-ink-2">In progress</dd></div>
        <div><dt className="inline">Thumbnails</dt> <dd className="m-0 inline text-ink-2">{loading.thumbnails === 'waiting' ? 'Waiting for the file list' : loading.thumbnails === 'done' ? 'Done' : 'In progress'}</dd></div>
      </dl>
    </div>
  )
}

/** DW-0119: a plain tile where a thumbnail has not arrived; the name and size are already there. */
export function PlainTile({ name, size }: { name: string; size: string }) {
  return (
    <div data-tile="plain" className="flex flex-col gap-2 rounded-xl border border-line bg-paper p-2">
      <div className="aspect-[4/3] rounded-lg bg-paper-2" />
      <div className="min-w-0 px-1 pb-1">
        <div className="truncate text-[13px] leading-5 text-ink">{name}</div>
        <div className="font-mono text-[11px] leading-4 text-ink-3">{size}</div>
      </div>
    </div>
  )
}

/** DW-0121: offline, and the file is not on this PC: say so instead of opening nothing. */
export function NeedsConnection({ fileName, onTryAgain, onClose }: { fileName: string; onTryAgain?: () => void; onClose?: () => void }) {
  return (
    <aside role="dialog" aria-label="Preview" className="flex w-[360px] flex-none flex-col border-l border-line bg-paper">
      <div className="flex h-10 flex-none items-center justify-between border-b border-line px-4">
        <span className="truncate text-[13px] font-medium text-ink">{fileName}</span>
        <button type="button" aria-label="Close (Esc)" title="Close (Esc)" onClick={onClose} className="grid h-7 w-7 place-items-center rounded-md text-ink-3 hover:bg-paper-3">
          <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.2" aria-hidden="true"><path d="M.5.5l9 9M9.5.5l-9 9" /></svg>
        </button>
      </div>
      <div data-empty="needs-connection" className="flex flex-1 flex-col items-center justify-center gap-3 px-8 text-center">
        <CloudTile />
        <h2 className="m-0 text-[16px] font-semibold leading-6 text-ink">Needs a connection</h2>
        <p className="m-0 text-[13px] leading-5 text-ink-3">{fileName} isn't on this PC yet. Connect to open it.</p>
        <button type="button" onClick={onTryAgain} className="h-8 rounded-md border border-line bg-paper px-4 text-[13px] font-medium text-ink hover:bg-paper-2 focus-visible:outline-2 focus-visible:outline-ink-3">Try again</button>
      </div>
    </aside>
  )
}
