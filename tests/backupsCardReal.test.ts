// The Backups status card is the engine's backup view and the backup problems list.
// A missing read says so. The board's pass clock and size (09:30, 412 MB) are never drawn.
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { createElement, type ReactNode } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { DriveSourceProvider, type DriveSource } from '@beebeeb/shared/drive-ui'
import { registerBoard } from '../src/v2/board'
import { Backups, cardWords, lastBackupText } from '../src/v2/sections/settings/backups'
import { Ctx, type SettingsCtx } from '../src/v2/sections/settings/shell/context'
import type { BackupView, SyncProblem } from '../src/v2/source/extras'

const BOARD_TIME = '09:30'
const BOARD_SIZE = '412 MB'
const now = new Date('2026-11-09T12:00:00')

function view(over: Partial<BackupView> & Pick<BackupView, 'state'>): BackupView {
  return { foldersOn: 0, foldersGettingReady: 0, ...over }
}

/** Retryable backup problems only: the count `cardWords` is given. */
function failedOf(problems: SyncProblem[]): number {
  return problems.filter((p) => p.backup && p.retryable).length
}

function problem(over: Partial<SyncProblem> & Pick<SyncProblem, 'id' | 'backup' | 'retryable'>): SyncProblem {
  return { name: over.id, folder: 'Desktop', reason: 'it could not be read', ...over }
}

const textOf = (html: string) => html.replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ').trim()

function pane(state?: 'idle' | 'running'): string {
  const settings: SettingsCtx = { pane: 'backups', go: () => {}, dialog: null, open: () => {}, close: () => {} }
  const source = { kind: 'desktop', subscribe: () => () => {} } as unknown as DriveSource
  const page: ReactNode = createElement(Backups, state ? { state } : {})
  const tree = createElement(DriveSourceProvider, { source }, createElement(Ctx.Provider, { value: settings }, page))
  return renderToStaticMarkup(tree)
}

describe('the Backups card shows the engine, not the board', () => {
  test('title and line come only from the backup view', () => {
    const at = '2026-11-09T07:30:00'
    const seen = view({ state: 'on', lastBackupAt: at, deviceFolder: 'This PC', foldersOn: 4, foldersGettingReady: 0 })
    const words = cardWords(seen, failedOf([]), now)
    expect(words).toEqual({ title: 'All backed up', line: `Last backup ${lastBackupText(at, now)}` })
    expect(words.line).not.toContain(BOARD_TIME)
    expect(words.line).not.toContain(BOARD_SIZE)
    expect(words.line).not.toMatch(/\b(MB|GB)\b/)
  })

  test('a problems list is the only failed count', () => {
    const at = '2026-11-09T07:30:00'
    const seen = view({ state: 'on', lastBackupAt: at, foldersOn: 2, foldersGettingReady: 0 })
    const problems = [
      problem({ id: 'open-note', backup: true, retryable: true }),
      problem({ id: 'too-big', backup: true, retryable: false }),
      problem({ id: 'sync-only', backup: false, retryable: true }),
    ]
    const one = cardWords(seen, failedOf(problems), now)
    expect(failedOf(problems)).toBe(1)
    expect(one).toEqual({ title: 'Backed up, except a few items', line: `Last backup ${lastBackupText(at, now)}` })
    const two = cardWords(seen, failedOf([...problems, problem({ id: 'second', backup: true, retryable: true })]), now)
    expect(two.title).toBe('Backed up, except a few items')
    expect(two.line).toBe(one.line)
    expect(two.line).not.toMatch(/\b(MB|GB)\b/)
  })

  test('off, stopped and a running pass use the view and no made-up size', () => {
    expect(cardWords(view({ state: 'off' }), 0, now)).toEqual({ title: 'Backups are off', line: null })
    const yesterday = '2026-11-08T07:30:00'
    const stopped = cardWords(view({ state: 'stopped', lastBackupAt: yesterday, foldersOn: 2, foldersGettingReady: 0 }), 0, now)
    expect(stopped).toEqual({ title: 'Backups stopped', line: `Last backup ${lastBackupText(yesterday, now)}` })
    expect(stopped.line).not.toContain(BOARD_TIME)
    expect(stopped.line).not.toMatch(/\b(MB|GB)\b/)

    const running = cardWords(view({ state: 'getting-ready', foldersOn: 3, foldersGettingReady: 3 }), 0, now)
    expect(running).toEqual({ title: 'Backing up', line: '3 folders' })
    expect(running.line).not.toContain(BOARD_TIME)
    expect(running.line).not.toMatch(/\b(MB|GB|min)\b/)

    const oneFolder = cardWords(view({ state: 'getting-ready', foldersOn: 1, foldersGettingReady: 1 }), 0, now)
    expect(oneFolder).toEqual({ title: 'Backing up', line: '1 folder' })

    const counted = cardWords(view({ state: 'getting-ready', foldersOn: 3, foldersGettingReady: 1, done: 2, total: 5 }), 0, now)
    expect(counted).toEqual({ title: 'Backing up', line: '2 of 5' })
    expect(counted.line).not.toMatch(/\b(MB|GB|min)\b/)

    expect(cardWords(view({ state: 'getting-ready' }), 0, now)).toEqual({ title: 'Backing up', line: null })
  })

  test('a missing read says so and never draws the board clock or size', () => {
    expect(cardWords(null, failedOf([problem({ id: 'stale', backup: true, retryable: true })]), now)).toEqual({ title: 'Not reported yet', line: null })

    registerBoard({
      backupCard: {
        idle: { last: BOARD_TIME, bytes: BOARD_SIZE, failed: 9 },
        running: { folder: 'Documents', left: 'about 20 min left', done: 612, total: 1820 },
      },
    })
    for (const state of ['idle', 'running'] as const) {
      const html = pane(state)
      const text = textOf(html)
      expect(text).toContain('Not reported yet')
      expect(text).toContain('Try again')
      expect(text).not.toContain(BOARD_TIME)
      expect(text).not.toContain(BOARD_SIZE)
      expect(text).not.toContain('about 20 min')
      expect(text).not.toContain('612')
      expect(text).not.toContain('1,820')
      expect(text).not.toContain('1820')
      expect(html).not.toContain('progressbar')
    }

    const src = readFileSync(new URL('../src/v2/sections/settings/backups/index.tsx', import.meta.url), 'utf8')
    expect(src).not.toContain('backupCard')
    expect(src).not.toContain('idle.last')
    expect(src).not.toContain('idle.bytes')
  })
})
