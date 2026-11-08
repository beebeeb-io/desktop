// B02b: the mappings from the extras source to what the screens draw. Absent stays absent.
import { describe, expect, test } from 'bun:test'
import { backupMetaFor, transfersByItem, uploadRows, uploadSummary } from '../src/v2/sections/files/liveExtras'
import { fixtureStatus, FIXTURE_TRANSFERS } from '../src/v2/source/fixtures/extras'
import { extrasFromStatus } from '../src/v2/tray'
import { cellMeta } from '../src/v2/sections/photos/model'
import { problemId, rowsFromProblems } from '../src/v2/sections/sync-history/model'

const ITEMS = [{ id: 'a', name: 'Anna’s iPhone' }, { id: 'b', name: 'Anna-Laptop' }]

describe('backupMetaFor', () => {
  test('only This PC’s own folder gets a last backup and a status', () => {
    expect(backupMetaFor(fixtureStatus('default'), ITEMS)).toEqual({ b: { last: '2026-11-08T11:52:00', status: 'Backed up', thisPc: true } })
  })
  test('no device folder known: no row gets one', () => {
    const s = fixtureStatus('default')
    expect(backupMetaFor({ ...s, backup: { ...s.backup, deviceFolder: undefined } }, ITEMS)).toEqual({})
    expect(backupMetaFor(null, ITEMS)).toEqual({})
  })
})

describe('transfers', () => {
  test('per-row progress by item id; a stopped row has no bar', () => {
    const t = transfersByItem(FIXTURE_TRANSFERS)
    expect(Object.keys(t).sort()).toEqual(['p-miradouro', 'p-ponte', 'u3'])
    expect(t['p-miradouro']).toEqual({ kind: 'uploading', done: 2_100_000, total: 3_600_000 })
    expect(t['p-ponte'].kind).toBe('waiting')
  })
  test('the Uploads panel groups and sums only what it knows', () => {
    const rows = uploadRows(FIXTURE_TRANSFERS)
    expect(rows.map((r) => r.state)).toEqual(['attention', 'uploading', 'uploading', 'waiting'])
    expect(rows[3].to).toBe('Lisbon trip')
    expect(uploadSummary(rows)).toBe('2 of 4 · 52.7 MB left · Continues in the background')
    expect(uploadSummary(rows.map((r) => ({ ...r, total: r.state === 'waiting' ? 0 : r.total })))).toBe('2 of 4 · Continues in the background')
    expect(uploadSummary([])).toBe('Nothing is uploading')
  })
})

describe('flyout extras', () => {
  test('the engine fields map one to one; unknown stays unknown', () => {
    expect(extrasFromStatus(fixtureStatus('syncing'))).toEqual({ metered: undefined, stoppedFiles: 0, totalFiles: 1_284, uploadBps: 1_400_000, etaSeconds: 60 })
    expect(extrasFromStatus(fixtureStatus('default')).uploadBps).toBeNull()
    expect(extrasFromStatus(null)).toEqual({})
  })
})

describe('photo meta', () => {
  test('dimensions and video length in the board’s form; nothing invented', () => {
    expect(cellMeta({ width: 4032, height: 3024, durationSeconds: 125, uploading: false, camera: 'Apple iPhone 15' })).toEqual({ camera: 'Apple iPhone 15', uploading: undefined, dims: '4032 × 3024', duration: '2:05' })
    expect(cellMeta({ uploading: true })).toEqual({ camera: undefined, uploading: true, dims: undefined, duration: undefined })
    expect(cellMeta(undefined)).toBeUndefined()
  })
})

describe('sync problems', () => {
  test('a problem row says why and keeps its change id for Try again', () => {
    const [row] = rowsFromProblems([{ id: 'op1', name: 'a.txt', folder: 'Docs', reason: 'Stopped after 5 tries.', retryable: true, backup: false, at: '2026-11-08T10:00:00Z' }], new Date())
    expect(row.problem).toBe('Stopped after 5 tries. Nothing was lost.')
    expect(row.what).toBe('Didn’t sync · Docs')
    expect(problemId(row)).toBe('op1')
  })
})
