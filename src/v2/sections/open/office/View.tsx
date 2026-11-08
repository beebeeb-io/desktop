// The Office editor's frame inside the Drive window's content area: the window's toolbar (back, the path and the
// name with its unsaved dot), the ribbon, an optional note, the canvas, and the editor's status bar.
import type { ReactNode } from 'react'
import type { OfficeApp } from '@beebeeb/shared/drive-ui'
import { OpenToolbar } from '../Chrome'
import { Ribbon, type Control } from './Ribbon'

export const APP_LABEL: Record<OfficeApp, string> = { writer: 'Word document', calc: 'Excel workbook', impress: 'PowerPoint presentation' }

export interface ShellProps {
  app: OfficeApp
  path: string[]
  name: string
  meta?: string
  dirty: boolean
  saving?: boolean
  tab: string
  onTab?: (t: string) => void
  onRun?: (c: Control) => void
  onSave?: () => void
  onPalette?: () => void
  onShare?: () => void
  onBack?: () => void
  dimmed?: boolean
  layout?: { current: string; onPick: (id: string) => void }
  note?: ReactNode
  status: ReactNode
  children: ReactNode
}

const noop = () => {}

export function OfficeShell({ app, path, name, meta, dirty, saving = false, tab, onTab = noop, onRun = noop, onSave = noop, onPalette = noop, onShare, onBack, dimmed, layout, note, status, children }: ShellProps) {
  return (
    <div data-office={app} className="flex min-h-0 flex-1 flex-col">
      <OpenToolbar name={[...path, name].join(' › ')} dirty={dirty} meta={meta} onBack={onBack} />
      <Ribbon app={app} tab={tab} onTab={onTab} onRun={onRun} dirty={dirty} saving={saving} onSave={onSave} onPalette={onPalette} onShare={onShare} dimmed={dimmed} layout={layout} />
      {note}
      <div className="flex min-h-0 flex-1">{children}</div>
      {status}
    </div>
  )
}
