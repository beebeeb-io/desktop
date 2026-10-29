import { describe, expect, test } from 'bun:test'
import * as copy from '../src/signOutCopy'
import { rustStr } from './fixtures/rustConstants'

const ask = (over: Partial<copy.AskCounts> = {}): copy.AskCounts => ({
  askId: 'a', files: 0, copiedFiles: 0, others: 0, missingFiles: 0, earlierFolders: [], ...over,
})

describe('the sign-out copy (spec 2026-10-10 §11)', () => {
  /** F6: every string equals Rust's. */
  test('the TypeScript constants equal the Rust ones', () => {
    const names = [
      'P0_TITLE', 'P1_ONE', 'P1_MANY', 'P2_HINT', 'P3_STOP', 'A0_TITLE', 'A0S_TITLE', 'A1S_LINE', 'A2_ONE', 'A2_MANY',
      'A2_COPY_IT', 'A2_COPY_THEM', 'A2_COPY_SOME', 'A2S_ONE', 'A2S_MANY', 'A2S_COPY_IT', 'A2S_COPY_THEM', 'A2S_COPY_SOME',
      'A3_ONE_OF_ONE', 'A3_ONE_OF_MANY', 'A3_MANY', 'A3E_ONE', 'A3E_MANY', 'A4_ONE', 'A4_MANY', 'A4S_ONE', 'A4S_MANY',
      'A5_OFFLINE', 'A5_SESSION', 'A5_PAUSED', 'A5_HELD', 'A5_ENGINE_STOPPED', 'A5_TIME_LIMIT', 'A5_ACCOUNT',
      'ASK_CANCEL', 'ASK_SIGN_OUT', 'ASK_SWITCH', 'K0_TITLE', 'K1_ONE', 'K1_MANY', 'K2_ROW', 'K2_SHOW', 'K2_DISMISS',
      'K3_TOAST', 'E1_UNSENT_CHANGES', 'E2_UNSENT_COPY_FAILED', 'E3_UNSENT_COUNT_FAILED',
    ] as const
    for (const name of names) {
      expect((copy as Record<string, unknown>)[name], name).toBe(rustStr('sign_out/copy.rs', name))
    }
  })

  /** F2. */
  test('the ask reads exactly, for every case', () => {
    expect(copy.askLines(ask({ files: 1, copiedFiles: 1 }), null, false).paragraphs).toEqual([
      '1 file you changed in Finder hasn’t reached the server. Before signing out, Beebeeb copies it to a folder on this Mac and shows you where.',
    ])
    expect(copy.askLines(ask({ others: 2 }), 'offline', false).paragraphs).toEqual([
      '2 other changes (renames, moves, deletions, new folders or restores) haven’t reached the server. Signing out drops them, and those items stay as they are on the server.',
      'This Mac can’t reach the server right now.',
    ])
    expect(copy.askLines(ask({ files: 3, copiedFiles: 2, others: 1 }), 'time_limit', false).paragraphs).toEqual([
      '3 files you changed in Finder haven’t reached the server. Before signing out, Beebeeb copies 2 of them to a folder on this Mac and shows you where.',
      '1 other change (a rename, move, deletion, new folder or restore) hasn’t reached the server either. Signing out drops it, and that item stays as it is on the server.',
      'Sending took longer than a minute. Choose Cancel to let it finish, then sign out again.',
    ])
    // n = k = c = 1: an earlier-build write is missing, a newer one is copied: A2 keeps its second sentence, then A3.
    expect(copy.askLines(ask({ files: 1, copiedFiles: 1, missingFiles: 1 }), null, false).paragraphs).toEqual([
      '1 file you changed in Finder hasn’t reached the server. Before signing out, Beebeeb copies it to a folder on this Mac and shows you where.',
      'Beebeeb’s copy of a change to that file is missing, so that change can’t be kept.',
    ])
    expect(copy.askLines(ask({ files: 2, missingFiles: 2 }), null, false).paragraphs[0]).toBe('2 files you changed in Finder haven’t reached the server.')
    // A3e: none, one path, two paths (oldest first, one per line).
    expect(copy.askLines(ask({ files: 1, missingFiles: 1 }), null, false).foldersLine).toBeNull()
    const one = copy.askLines(ask({ files: 1, missingFiles: 1, earlierFolders: ['/k/1'] }), null, false)
    expect([one.foldersLine, one.folders]).toEqual([copy.A3E_ONE, ['/k/1']])
    expect(one.foldersAfter).toBe(2) // after A2 and A3, before A4 and A5
    const two = copy.askLines(ask({ files: 1, missingFiles: 1, earlierFolders: ['/k/1', '/k/2'] }), null, false)
    expect([two.foldersLine, two.folders]).toEqual([copy.A3E_MANY, ['/k/1', '/k/2']])
    // Each reason, and none for `stopped`.
    for (const [reason, line] of [['offline', copy.A5_OFFLINE], ['session', copy.A5_SESSION], ['paused', copy.A5_PAUSED], ['held', copy.A5_HELD], ['engine_stopped', copy.A5_ENGINE_STOPPED], ['time_limit', copy.A5_TIME_LIMIT], ['account', copy.A5_ACCOUNT]] as const) {
      expect(copy.reasonLine(reason)).toBe(line)
    }
    expect(copy.reasonLine('stopped')).toBeNull()
    // The switch: its own sentences, and no reason line.
    expect(copy.askLines(ask({ files: 1, copiedFiles: 1, others: 1 }), 'offline', true).paragraphs).toEqual([
      '1 file changed in Finder while that account was signed in hasn’t reached the server. Before switching, Beebeeb copies it to a folder on this Mac and shows you where.',
      '1 other change (a rename, move, deletion, new folder or restore) hasn’t reached the server either. Switching drops it, and that item stays as it is on the server.',
    ])
    // The plural forms: c = n > 1, A3 for k = 1 < n and for k > 1, and the switch's own plural sentences.
    expect(copy.askLines(ask({ files: 2, copiedFiles: 2 }), null, false).paragraphs).toEqual([
      '2 files you changed in Finder haven’t reached the server. Before signing out, Beebeeb copies them to a folder on this Mac and shows you where.',
    ])
    expect(copy.askLines(ask({ files: 3, copiedFiles: 2, missingFiles: 1 }), null, false).paragraphs).toEqual([
      '3 files you changed in Finder haven’t reached the server. Before signing out, Beebeeb copies 2 of them to a folder on this Mac and shows you where.',
      'Beebeeb’s copy of a change to 1 of these files is missing, so that change can’t be kept.',
    ])
    expect(copy.askLines(ask({ files: 2, missingFiles: 2 }), null, false).paragraphs).toEqual([
      '2 files you changed in Finder haven’t reached the server.',
      'Beebeeb’s copies of changes to 2 of these files are missing, so those changes can’t be kept.',
    ])
    expect(copy.askLines(ask({ files: 2, copiedFiles: 2, others: 3 }), 'held', true).paragraphs).toEqual([
      '2 files changed in Finder while that account was signed in haven’t reached the server. Before switching, Beebeeb copies them to a folder on this Mac and shows you where.',
      '3 other changes (renames, moves, deletions, new folders or restores) haven’t reached the server either. Switching drops them, and those items stay as they are on the server.',
    ])
    expect(copy.askLines(ask({ files: 3, copiedFiles: 1, missingFiles: 2 }), null, true).paragraphs).toEqual([
      '3 files changed in Finder while that account was signed in haven’t reached the server. Before switching, Beebeeb copies 1 of them to a folder on this Mac and shows you where.',
      'Beebeeb’s copies of changes to 2 of these files are missing, so those changes can’t be kept.',
    ])
    // A3e sits under A3, before A4 and A5, when those follow.
    const under = copy.askLines(ask({ files: 1, missingFiles: 1, others: 1, earlierFolders: ['/k/1'] }), 'offline', false)
    expect([under.foldersAfter, under.paragraphs.length]).toEqual([2, 4])
  })

  /** F7. */
  test('a progress line never reads 0', () => {
    expect(copy.progressLine(0)).toBeNull()
    expect(copy.progressLine(1)).toBe('Sending 1 change before signing out…')
    expect(copy.progressLine(4)).toBe('Sending 4 changes before signing out…')
  })

  test('a code prefix is never shown to a person', () => {
    expect(copy.stripCode(`unsent_copy_failed: ${copy.E2_UNSENT_COPY_FAILED}`)).toBe(copy.E2_UNSENT_COPY_FAILED)
    expect(copy.stripCode('Sign-out paused: x')).toBe('Sign-out paused: x')
    // Each of the three codes, as Rust spells it.
    for (const [code, sentence] of [
      ['UNSENT_CHANGES_CODE', copy.E1_UNSENT_CHANGES],
      ['UNSENT_COPY_FAILED_CODE', copy.E2_UNSENT_COPY_FAILED],
      ['UNSENT_COUNT_FAILED_CODE', copy.E3_UNSENT_COUNT_FAILED],
    ] as const) {
      expect(copy.stripCode(`${rustStr('sign_out/copy.rs', code)}: ${sentence}`), code).toBe(sentence)
    }
  })
})
