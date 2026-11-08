// Batch B08's frames (design/desktop-v2/BATCHES.md): Office on Windows, Docs, Sheets and Slides. Owned by B08 only.
// Each frame renders the editor's real parts around the board's documents (src/v2/source/fixtures/office.ts).
import type { ReactElement, ReactNode } from 'react'
import { DriveSourceProvider, type OfficeApp } from '@beebeeb/shared/drive-ui'
import type { FrameRegistry } from './types'
import { CaptionBarView } from '../chrome/CaptionBar'
import { Sidebar } from '../chrome/Sidebar'
import { StatusLine } from '../chrome/StatusLine'
import { ListBackdrop, STORAGE_240, SYNC_OK, Toolbar } from '../chrome/FrameWindow'
import { Menu } from '../sections/files/ui'
import { fileMenu } from '../sections/files/menus'
import { SavedMessage } from '../sections/open/Editing'
import { DocPage, Filmstrip, FormulaBar, OutlinePane, PresentView, SheetGrid, SheetTabs, SlideCanvas } from '../sections/open/office/Canvas'
import { withOfficeOpen } from '../sections/open/office/menu'
import { AboutPanel, CommandPalette, ConflictDialog, FailedView, FloatingToolbar, LegacyNote, NewDocumentDialog, OfficeStatus, PreparingView, type PaletteRow } from '../sections/open/office/Parts'
import { OfficeShell } from '../sections/open/office/View'
import { createOfficeSource, DOC_2012, DOC_AGREEMENT, INSERTED_TABLE, OFFICE_ITEMS, SHEET_COLS, SHEET_ROWS, SHEET_TABS, SLIDES, type DocFixture } from '../source/fixtures/office'

const SRC = createOfficeSource()
const noop = () => {}
const MB = 1024 * 1024

function Win({ children, overlay, nav = 'files' }: { children: ReactNode; overlay?: ReactNode; nav?: 'files' }): ReactElement {
  return (
    <DriveSourceProvider source={SRC}>
      <div data-frame-window className="relative flex h-screen flex-col overflow-hidden bg-paper font-sans text-ink">
        <CaptionBarView title="Beebeeb" />
        <div className="flex min-h-0 flex-1">
          <Sidebar active={nav} onGo={noop} storage={STORAGE_240} accountName="Anna" counts={{ shared: 1, trash: 12 }} />
          <main className="flex min-w-0 flex-1 flex-col">{children}</main>
        </div>
        {overlay}
      </div>
    </DriveSourceProvider>
  )
}

const DOCS_LEFT = ['Page 1 of 4', '1,284 words', 'English (UK)']
const AGREEMENT = 'Service agreement.docx'

interface DocsOpts { tab?: string; dirty?: boolean; saving?: boolean; status?: ReactNode; note?: ReactNode; overlay?: ReactNode; floating?: boolean; table?: boolean; doc?: DocFixture; name?: string; path?: string[]; meta?: string; dimmed?: boolean; body?: ReactNode }

function docs(o: DocsOpts = {}): ReactElement {
  const doc = o.doc ?? DOC_AGREEMENT
  return (
    <Win overlay={o.overlay}>
      <OfficeShell app="writer" path={o.path ?? ['Files', 'Documents']} name={o.name ?? AGREEMENT} meta={o.meta ?? 'Word document · 48 KB · opens in the editor'} dirty={!!o.dirty} saving={o.saving} tab={o.tab ?? 'Home'} dimmed={o.dimmed} note={o.note}
        status={o.status ?? <OfficeStatus left={DOCS_LEFT} dirty={o.dirty} version={7} time="09:12" />}>
        {o.body ?? (
          <>
            <OutlinePane items={doc.outline} current={1} />
            <DocPage doc={doc} selection={o.floating} table={o.table ? INSERTED_TABLE : undefined}>
              {o.floating && <FloatingToolbar x="calc(50% - 21px)" y={146} />}
            </DocPage>
          </>
        )}
      </OfficeShell>
    </Win>
  )
}

const SHEET_LEFT: ReactNode[] = ['Sheet 1 of 3', <>Sum <span className="font-mono">13,220</span></>, <>Average <span className="font-mono">1,652.50</span></>, 'Count 8']

function sheets(tab: string, go?: boolean): ReactElement {
  return (
    <Win>
      <OfficeShell app="calc" path={['Files']} name="Budget 2026.xlsx" meta="Excel workbook · 96 KB · opens in the editor" dirty={false} tab={tab}
        status={<OfficeStatus left={SHEET_LEFT} version={12} time="09:12" />}>
        <div className="flex min-w-0 flex-1 flex-col">
          <FormulaBar cell={go ? 'B5' : 'B10'} formula="=SUM(B2:B9)" focused={go} />
          <SheetGrid cols={SHEET_COLS} rows={SHEET_ROWS} selection={go ? undefined : { col: 1, from: 1, to: 8 }} current={{ col: 1, row: go ? 4 : 9 }} />
          <SheetTabs tabs={SHEET_TABS} active={0} />
        </div>
      </OfficeShell>
    </Win>
  )
}

function slides(tab: string): ReactElement {
  return (
    <Win>
      <OfficeShell app="impress" path={['Files', 'Lisbon trip']} name="Lisbon trip recap.pptx" meta="PowerPoint presentation · 18.4 MB · opens in the editor" dirty={false} tab={tab}
        layout={{ current: 'layout-title-content', onPick: noop }}
        status={<OfficeStatus left={['Slide 2 of 3', 'English (UK)']} version={5} time="Yesterday 16:10" />}>
        <Filmstrip slides={SLIDES} current={1} />
        <SlideCanvas slide={SLIDES[1]} />
      </OfficeShell>
    </Win>
  )
}

const PALETTE_ROWS: PaletteRow[] = [
  { id: 'style-h1', label: 'Heading 1', group: 'Styles' },
  { id: 'style-h2', label: 'Heading 2', group: 'Styles' },
  { id: 'style-h3', label: 'Heading 3', group: 'Styles' },
  { id: 'style-normal', label: 'Normal text', group: 'Styles' },
  { id: 'bold', label: 'Bold', group: 'Format', shortcut: 'Ctrl+B', pinned: true },
  { id: 'italic', label: 'Italic', group: 'Format', shortcut: 'Ctrl+I' },
  { id: 'file-brand', label: 'Northwind brand guide.pdf', group: 'Files', pinned: true },
]

function newDocument(kind: OfficeApp): ReactElement {
  return (
    <Win overlay={<NewDocumentDialog kind={kind} template="blank" name="Untitled document.docx" onKind={noop} onTemplate={noop} onName={noop} onCreate={noop} onCancel={noop} />}>
      <Toolbar /><ListBackdrop dim />
    </Win>
  )
}

const agreementItem = OFFICE_ITEMS[0]

export const FRAMES: FrameRegistry = {
  'DW-1507': () => docs({ floating: true }),
  'DW-1550': () => docs({ tab: 'Insert', table: true, dirty: true, status: <OfficeStatus left={DOCS_LEFT} dirty version={7} time="09:12" /> }),
  'DW-1551': () => docs({ overlay: <CommandPalette rows={PALETTE_ROWS} initialQuery="head" onRun={noop} onClose={noop} /> }),
  'DW-1552': () => docs({ dirty: true, status: <OfficeStatus left={DOCS_LEFT} dirty version={7} time="09:12" /> }),
  'DW-1553': () => docs({ note: <SavedMessage version={8} onVersions={noop} />, status: <OfficeStatus left={DOCS_LEFT} version={8} time="09:41" /> }),
  'DW-1554': () => docs({ dirty: true, status: <OfficeStatus left={DOCS_LEFT} dirty conflict version={7} time="09:12" />, overlay: <ConflictDialog latest={8} opened={7} onChoose={noop} onCancel={noop} /> }),
  'DW-1555': () => docs({ dimmed: true, status: <OfficeStatus left={[]} loading version={7} time="" />, body: <PreparingView loaded={96 * MB} total={257 * MB} /> }),
  'DW-1556': () => docs({ dimmed: true, status: <OfficeStatus left={[]} loading version={7} time="" />, body: <FailedView onRetry={noop} onOtherApp={noop} /> }),
  'DW-1557': () => docs({ overlay: <AboutPanel onClose={noop} noticesAvailable /> }),
  'DW-1511': () => docs({ doc: DOC_2012, name: 'Proposal 2012.doc', meta: 'Word 97–2003 document · 31 KB · opens in the editor', note: <LegacyNote ext="doc" />, status: <OfficeStatus left={['Page 1 of 2', '412 words', 'English (UK)']} version={3} time="2 Nov" /> }),
  'DW-1508': () => sheets('Home'),
  'DW-1559': () => sheets('Data', true),
  'DW-1509': () => slides('Home'),
  'DW-1560': () => slides('Design'),
  'DW-1561': () => <DriveSourceProvider source={SRC}><PresentView slides={SLIDES} index={1} onIndex={noop} onExit={noop} /></DriveSourceProvider>,
  'DW-1558': () => newDocument('writer'),
  'DW-1562': () => (
    <Win overlay={<Menu x={420} y={210} label="File" entries={withOfficeOpen(fileMenu([agreementItem]), [agreementItem], ['Pages', 'Microsoft Word'], true)} onPick={noop} />}>
      <Toolbar /><ListBackdrop />
      <StatusLine sync={SYNC_OK} />
    </Win>
  ),
}
