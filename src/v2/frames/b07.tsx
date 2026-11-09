// Batch B07's frames (design/desktop-v2/BATCHES.md): Open files, states, text and Markdown editing. Owned by B07 only.
// Each frame renders the real viewer or dialog against the B07 fixtures (src/v2/source/fixtures/open.ts).
import type { ReactElement, ReactNode } from 'react'
import { DriveSourceProvider, type SyncSummary } from '@beebeeb/shared/drive-ui'
import type { FrameRegistry } from './types'
import { CaptionBarView } from '../chrome/CaptionBar'
import { Sidebar } from '../chrome/Sidebar'
import { StatusLine } from '../chrome/StatusLine'
import { ListBackdrop, STORAGE_240, SYNC_OK } from './FrameWindow'
import { Menu } from '../sections/files/ui'
import { newMenu } from '../sections/files/menus'
import { thumbnailFor } from '../source/fixtures/files'
import { AGREEMENT_PAGES, AUDIO_QUEUE, CSV_48, MD_TEXT, MD_TEXT_SAVED, PDF_PAGES, PY_TEXT, RAW_FACTS, ZIP_ROWS, createOpenSource } from '../source/fixtures/open'
import { DetailsRail, FindBar, MoreButton, NewerBanner, OpenToolbar, ToolbarButton } from '../sections/open/Chrome'
import { EditConflictDialog, NewFileNameDialog, NewFileTypeDialog, SaveButton, SavedMessage, SavedStatus, TextEditor, UnsavedAlert, editMeta } from '../sections/open/Editing'
import { DecryptFailedView, DecryptingView, NoRoomStatus, OpenInAppNote, TooLargeView } from '../sections/open/States'
import { ArchiveViewer, AudioPlayer, CodeViewer, CsvViewer, MarkdownViewer, PdfViewer, PdfZoom, QuickSettingsMedia, RawViewer, pdfMatches } from '../sections/open/Viewers'

const SRC = createOpenSource()
const noop = () => {}
const GB = 1024 ** 3

function Win({ children, aside, status, overlay, sync = SYNC_OK }: { children: ReactNode; aside?: ReactNode; status?: ReactNode; overlay?: ReactNode; sync?: SyncSummary | null }): ReactElement {
  return (
    <DriveSourceProvider source={SRC}>
      <div data-frame-window className="relative flex h-screen flex-col overflow-hidden bg-paper font-sans text-ink">
        <CaptionBarView title="Beebeeb" />
        <div className="flex min-h-0 flex-1">
          <Sidebar active="files" onGo={noop} storage={STORAGE_240} accountName="Anna" counts={{ shared: 1, trash: 12 }} />
          <main className="flex min-w-0 flex-1 flex-col">{children}</main>
          {aside}
        </div>
        {status ?? <StatusLine sync={sync} />}
        {overlay}
      </div>
    </DriveSourceProvider>
  )
}

const stdActions = (
  <>
    <ToolbarButton label="Share" /><ToolbarButton label="Offline" /><ToolbarButton label="Details" /><ToolbarButton label="Trash" /><MoreButton />
  </>
)

const photo = thumbnailFor('DSC04182.ARW')
const strip = [1, 2, 3, 4, 5].map((n) => ({ id: `s${n}`, src: photo, current: n === 3 }))

const editorToolbar = (dirty: boolean, version: number) => (
  <OpenToolbar name="Meeting notes 4 Nov.md" dirty={dirty} meta={editMeta(dirty, version)}>
    <SaveButton disabled={!dirty} /><MoreButton />
  </OpenToolbar>
)

const CONFLICT_MINE = { label: 'Your edit', when: 'this PC', note: 'not saved · +1 line', preview: MD_TEXT_SAVED }
const CONFLICT_THEIRS = { label: 'Anna’s iPhone', when: '09:14', note: 'version 4', preview: MD_TEXT }

const audioView = (
  <>
    <OpenToolbar name="Harbour sessions, take 3.wav" meta="Decrypted on this PC">{stdActions}</OpenToolbar>
    <AudioPlayer tracks={AUDIO_QUEUE} index={0} playing elapsed={84} />
  </>
)

const pdfToolbar = (name: string, page: string) => (
  <OpenToolbar name={name} meta={page}>
    <PdfZoom zoom={1} onZoom={noop} /><ToolbarButton label="Find" shortcut="Ctrl+F" /><ToolbarButton label="Share" /><MoreButton />
  </OpenToolbar>
)

const agreementMatches = pdfMatches(AGREEMENT_PAGES, 'terrace')

export const FRAMES: FrameRegistry = {
  'DW-1501': () => (
    <Win>
      <OpenToolbar name="DSC04182.ARW" meta="Shoot selects · Sunday 14:20">{stdActions}</OpenToolbar>
      <RawViewer src={photo} strip={strip} />
    </Win>
  ),
  'DW-1502': () => (
    <Win aside={<DetailsRail title="Details" rows={RAW_FACTS} />}>
      <OpenToolbar name="DSC04182.ARW" meta="Shoot selects · Sunday 14:20">{stdActions}</OpenToolbar>
      <RawViewer src={photo} strip={strip} />
    </Win>
  ),
  'DW-1503': () => <Win>{audioView}</Win>,
  'DW-1504': () => <Win overlay={<QuickSettingsMedia title="Harbour sessions, take 3" artist="Northwind Studio" />}>{audioView}</Win>,
  'DW-1505': () => (
    <Win>
      {pdfToolbar('Studio plan.pdf', 'Page 2 of 6')}
      <PdfViewer pages={PDF_PAGES} page={1} zoom={1} />
    </Win>
  ),
  'DW-1506': () => (
    <Win>
      {pdfToolbar('Service agreement.pdf', 'Page 1 of 3')}
      <FindBar query="terrace" position={`2 of ${agreementMatches.length}`} />
      <PdfViewer pages={AGREEMENT_PAGES} page={0} zoom={1} query="terrace" current={1} />
    </Win>
  ),
  'DW-1512': () => (
    <Win>
      <OpenToolbar name="export_invoices.py" meta="Python · 25 lines · UTF-8">
        <ToolbarButton label="Wrap" /><ToolbarButton label="Copy" /><ToolbarButton label="Edit" shortcut="Ctrl+E" /><MoreButton />
      </OpenToolbar>
      <CodeViewer text={PY_TEXT} wrap={false} />
    </Win>
  ),
  'DW-1513': () => (
    <Win>
      <OpenToolbar name="Meeting notes 4 Nov.md">
        <span role="group" aria-label="View" className="flex rounded-md border border-line p-0.5 text-[12px]">
          <button type="button" aria-pressed="true" className="h-7 rounded bg-paper-3 px-3 text-ink">Rendered</button>
          <button type="button" aria-pressed="false" className="h-7 rounded px-3 text-ink-3">Source</button>
        </span>
        <ToolbarButton label="Edit" shortcut="Ctrl+E" /><MoreButton />
      </OpenToolbar>
      <MarkdownViewer text={MD_TEXT} mode="rendered" />
    </Win>
  ),
  'DW-1514': () => (
    <Win>
      <OpenToolbar name="Invoices Q3.csv"><ToolbarButton label="Edit" shortcut="Ctrl+E" /><MoreButton /></OpenToolbar>
      <CsvViewer text={CSV_48} />
    </Win>
  ),
  'DW-1515': () => (
    <Win>
      <OpenToolbar name="Raw footage 2024.zip"><ToolbarButton label="Open in another app…" /><MoreButton /></OpenToolbar>
      <ArchiveViewer rows={ZIP_ROWS} note={'214 files · 3.8 GB unpacked · files inside can’t be opened here'} />
    </Win>
  ),
  'DW-1516': () => (
    <Win>
      <OpenToolbar name="Studio walkthrough 4K.mov" />
      <DecryptingView name="Studio walkthrough 4K.mov" done={46 * 1024 ** 2} total={120 * 1024 ** 2} />
    </Win>
  ),
  'DW-1517': () => (
    <Win status={<NoRoomStatus needed={18.2 * GB} free={12.4 * GB} />}>
      <OpenToolbar name="Studio walkthrough 4K.mov" />
      <TooLargeView name="Studio walkthrough 4K.mov" size={18.2 * GB} free={12.4 * GB} />
    </Win>
  ),
  'DW-1518': () => (
    <Win>
      <OpenToolbar name="Studio walkthrough 4K.mov" />
      <DecryptFailedView />
    </Win>
  ),
  'DW-1519': () => (
    <Win>
      <OpenToolbar name="Meeting notes 4 Nov.md"><ToolbarButton label="Edit" shortcut="Ctrl+E" /><MoreButton /></OpenToolbar>
      <NewerBanner text={'A newer version arrived from Anna’s iPhone, 09:14.'} />
      <MarkdownViewer text={MD_TEXT} mode="rendered" />
    </Win>
  ),
  'DW-1520': () => (
    <Win overlay={<Menu x={window.innerWidth - 232} y={92} entries={newMenu()} onPick={noop} label="New" />}>
      <div className="flex h-12 flex-none items-center gap-3 border-b border-line px-4"><h1 className="m-0 text-[15px] font-semibold text-ink">Files</h1><span className="flex-1" /><span className="inline-flex h-8 items-center rounded-md bg-amber px-4 text-[13px] font-medium text-amber-ink">New</span></div>
      <ListBackdrop />
    </Win>
  ),
  'DW-1521': () => (
    <Win overlay={<NewFileTypeDialog selected="md" />}>
      <ListBackdrop dim />
    </Win>
  ),
  'DW-1522': () => (
    <Win overlay={<NewFileNameDialog name="export_invoices" ext=".py" />}>
      <ListBackdrop dim />
    </Win>
  ),
  'DW-1523': () => (
    <Win>
      {editorToolbar(true, 3)}
      <TextEditor name="Meeting notes 4 Nov.md" initialDoc={MD_TEXT_SAVED} onChange={noop} onSave={noop} />
    </Win>
  ),
  'DW-1524': () => (
    <Win status={<SavedStatus version={4} time="09:41" />}>
      <OpenToolbar name="Meeting notes 4 Nov.md"><ToolbarButton label="Edit" shortcut="Ctrl+E" /><MoreButton /></OpenToolbar>
      <SavedMessage version={4} />
      <MarkdownViewer text={MD_TEXT_SAVED} mode="rendered" />
    </Win>
  ),
  'DW-1525': () => (
    <Win overlay={<UnsavedAlert name="Meeting notes 4 Nov.md" />}>
      {editorToolbar(true, 3)}
      <TextEditor name="Meeting notes 4 Nov.md" initialDoc={MD_TEXT_SAVED} onChange={noop} onSave={noop} />
    </Win>
  ),
  'DW-1526': () => (
    <Win overlay={<EditConflictDialog name="Meeting notes 4 Nov.md" changedOn={'Changed on Anna’s iPhone while you were editing'} mine={CONFLICT_MINE} theirs={CONFLICT_THEIRS} />}>
      {editorToolbar(true, 3)}
      <TextEditor name="Meeting notes 4 Nov.md" initialDoc={MD_TEXT_SAVED} onChange={noop} onSave={noop} />
    </Win>
  ),
  'DW-1527': () => {
    const lines = Array.from({ length: 500 }, (_, i) => `2026-11-08T09:${String(i % 60).padStart(2, '0')}:00Z INFO request ${i + 1} served in ${(i * 7) % 90} ms`).join('\n')
    return (
      <Win>
        <OpenToolbar name="server.log" meta="Plain text · 41,206 lines · UTF-8"><ToolbarButton label="Wrap" /><ToolbarButton label="Copy" /><MoreButton /></OpenToolbar>
        <OpenInAppNote>Read-only: this file is larger than 2 MB.</OpenInAppNote>
        <CodeViewer text={lines} wrap={false} limit={500} />
        <div className="flex-none border-t border-line px-4 py-1.5 font-mono text-[12px] text-ink-3">Showing the first 500 of 41,206 lines</div>
      </Win>
    )
  },
}
