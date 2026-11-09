// Storage insights draws only what `account_storage_breakdown` returned. The chart is those rows
// (no padded Other), an empty answer is the empty state, and a failed read says so.
import { describe, expect, test } from 'bun:test'
import { createElement, type ReactElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { DriveSourceProvider, type DriveSource } from '@beebeeb/shared/drive-ui'
import type { StorageBreakdown, StorageCategory } from '../src/desktopApi'
import { Ctx, type SettingsCtx } from '../src/v2/sections/settings/shell/context'
import { fmtBytes } from '../src/v2/sections/settings/shell/kit'
import * as insights from '../src/v2/sections/settings/today/insights'

const GB = 1024 ** 3
const QUOTA = 5 * GB

const textOf = (html: string) => html
  .replace(/<[^>]+>/g, ' ')
  .replace(/&#x27;|&#39;|&apos;/g, "'")
  .replace(/&amp;/g, '&')
  .replace(/&quot;/g, '"')
  .replace(/\s+/g, ' ')
  .trim()

const ctx: SettingsCtx = { pane: 'insights', go: () => {}, dialog: null, open: () => {}, close: () => {} }

const source = { storage: async () => ({ usedBytes: 0, quotaBytes: QUOTA, filesBytes: 0, photosBytes: 0, backupsBytes: 0 }) } as unknown as DriveSource

function page(node: ReactElement, withSource = false): string {
  const inner = withSource ? createElement(DriveSourceProvider, { source }, node) : node
  return renderToStaticMarkup(createElement(Ctx.Provider, { value: ctx }, inner))
}

type Read = { ok: true; value: StorageBreakdown } | { ok: false; unsupported: boolean }

type ScreenProps = { model: { kind: string }; onRetry: () => void; onOpen?: (id: string, path: string) => void }

const from = (insights as { insightsFrom?: (read: Read, quotaBytes: number | null) => { kind: string; text?: string; totalBytes?: number; quotaBytes?: number | null; slices?: { label: string; bytes: number; files: number }[]; largest?: { id: string; shown: string; kind: string; section: string; bytes: number }[]; sections?: { id: string; label: string; bytes: number; files: number }[] } }).insightsFrom
const Screen = (insights as { InsightsScreen?: (props: ScreenProps) => ReactElement }).InsightsScreen

function row(file_id: string, path: string, size_bytes: number, category: StorageCategory) {
  return { file_id, path, size_bytes, category }
}

const emptyBreakdown: StorageBreakdown = {
  by_category: [
    { category: 'media', bytes: 0, file_count: 0 },
    { category: 'documents', bytes: 0, file_count: 0 },
    { category: 'other', bytes: 0, file_count: 0 },
  ],
  largest_files: [],
  total_files: 0,
  total_bytes: 0,
}

describe('Storage insights shows the breakdown it read', () => {
  test('kind, section and cut path come from the command’s own rows', () => {
    const video = row('v', '/Backups/pc/clip.mov', 8192, 'media')
    const pdf = row('d', '\\docs\\report.pdf', 4096, 'documents')
    const blob = row('z', 'notes/blob', 10, 'other')
    expect(insights.kindOf(video.path, video.category)).toBe('Video')
    expect(insights.sectionOfPath(video.path)).toBe('backups')
    expect(insights.cutPath(video.path)).toBe('Backups/pc/clip.mov')
    expect(insights.kindOf(pdf.path, pdf.category)).toBe('PDF')
    expect(insights.sectionOfPath(pdf.path)).toBe('files')
    expect(insights.cutPath(pdf.path)).toBe('docs/report.pdf')
    expect(insights.kindOf(blob.path, blob.category)).toBe('Other')
    expect(insights.sectionOfPath(blob.path)).toBe('files')
  })

  test('slices are only the returned rows, a count under 10 still shows, and the total is their sum', () => {
    expect(typeof from).toBe('function')
    expect(typeof Screen).toBe('function')
    if (!from || !Screen) return
    const breakdown: StorageBreakdown = {
      by_category: [
        { category: 'documents', bytes: 10000, file_count: 3 },
        { category: 'media', bytes: 50000, file_count: 1 },
      ],
      largest_files: [
        row('d', '\\docs\\report.pdf', 4096, 'documents'),
        row('v', '/Backups/pc/clip.mov', 8192, 'media'),
      ],
      total_files: 99,
      total_bytes: 999999,
    }
    const model = from({ ok: true, value: breakdown }, QUOTA)
    expect(model.kind).toBe('ready')
    expect(model.slices?.map((s) => s.label)).toEqual(['Documents', 'Media'])
    expect(model.slices?.[0]).toEqual({ label: 'Documents', bytes: 10000, files: 3 })
    expect(model.slices?.[1]).toEqual({ label: 'Media', bytes: 50000, files: 1 })
    expect(model.totalBytes).toBe(60000)
    expect(model.totalBytes).not.toBe(999999)
    expect(model.sections).toEqual([
      { id: 'files', label: 'Files', bytes: 4096, files: 1 },
      { id: 'backups', label: 'Backups', bytes: 8192, files: 1 },
    ])
    const pdf = breakdown.largest_files[0]!
    const mov = breakdown.largest_files[1]!
    expect(model.largest?.[0]).toMatchObject({ id: 'd', shown: insights.cutPath(pdf.path), kind: insights.kindOf(pdf.path, pdf.category), section: insights.sectionOfPath(pdf.path), bytes: 4096 })
    expect(model.largest?.[1]).toMatchObject({ id: 'v', shown: insights.cutPath(mov.path), kind: insights.kindOf(mov.path, mov.category), section: insights.sectionOfPath(mov.path), bytes: 8192 })
    const html = page(createElement(Screen, { model, onRetry: () => {} }))
    const text = textOf(html)
    expect(text).toContain('Documents')
    expect(text).toContain('Media')
    expect(text).toContain(`${fmtBytes(10000)}`)
    expect(text).toContain('3 files')
    expect(text).toContain('Backups')
    expect(text).toContain(fmtBytes(8192))
    expect(text).toContain(`${fmtBytes(60000)} of ${fmtBytes(QUOTA)}`)
    expect(text).not.toContain('Other')
    expect(text).not.toContain(fmtBytes(999999))
    expect(text).not.toContain('99 files')
    expect(html.match(/data-testid="section-row"/g)).toHaveLength(2)
    expect(html).toContain('docs/report.pdf')
    expect(html).toContain('Backups/pc/clip.mov')
    expect(html).toContain('?section=settings&amp;page=storage')
  })

  test('an empty answer renders the empty state rather than a chart', () => {
    let html: string
    if (from && Screen) {
      const model = from({ ok: true, value: emptyBreakdown }, QUOTA)
      expect(model.kind).toBe('empty')
      html = page(createElement(Screen, { model, onRetry: () => {} }))
    } else {
      html = page(createElement(insights.Insights, { state: 'empty' }), true)
    }
    const text = textOf(html)
    expect(text).toContain('Nothing stored yet')
    expect(text).not.toContain('Nothing is stored yet')
    expect(text).toContain(`${fmtBytes(0)} of ${fmtBytes(QUOTA)}`)
    expect(text).toContain('Storage')
    expect(html).not.toContain('progressbar')
    expect(html).not.toContain('data-testid="type-row"')
    expect(html).not.toContain('data-testid="section-row"')
    expect(text).not.toContain('Media')
    expect(text).not.toContain('Other')
  })

  test('a failed read says so and offers Try again, not a chart', () => {
    let retried = 0
    let html: string
    if (from && Screen) {
      const model = from({ ok: false, unsupported: false }, QUOTA)
      expect(model.kind).toBe('error')
      html = page(createElement(Screen, { model, onRetry: () => { retried += 1 } }))
      expect(model.text).toBeTruthy()
      expect(textOf(html)).toContain(model.text!)
    } else {
      html = page(createElement(insights.Insights, { state: 'error' }), true)
    }
    const text = textOf(html)
    expect(text).toContain('Try again')
    expect(text).not.toContain("didn't answer in time")
    expect(text).not.toContain('didn’t answer in time')
    expect(html).not.toContain('progressbar')
    expect(html).not.toContain('data-testid="type-row"')
    expect(text).not.toContain('Media')
    expect(retried).toBe(0)
  })
})
