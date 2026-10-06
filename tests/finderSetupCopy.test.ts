/**
 * Spec 2026-10-06 §6.2 and §10: every Finder-setup string lives in src/finderSetupCopy.ts, keyed
 * by reason; each reason is one sentence and one action; the design artefact draws exactly this
 * copy (§12). Strings use the house typographic apostrophe (plan "Spec issues" 15).
 */
import { afterEach, describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { copyFinderSetupDetails, FINDER_FAILURE_REASONS, type FinderFailureReason, type FinderSetupView } from '../src/finderSetup'
import {
  FINDER_ACTION_FAILED,
  FINDER_ACTION_LABEL,
  FINDER_ADDING_LINE,
  FINDER_READY_LINE,
  FINDER_REASON_COPY,
  FINDER_SETUP_TITLE,
  FINDER_STATUS_PILL,
  FINDER_UNAVAILABLE_LINE,
  finderSetupLoadPresentation,
  finderSetupPresentation,
} from '../src/finderSetupCopy'
import { finderHint } from '../src/macSettingsModel'

const view = (over: Partial<FinderSetupView> = {}): FinderSetupView => ({
  setup: 'missing',
  reason: null,
  launch_location: 'applications',
  attempt: 1,
  max_attempts: 1,
  last_failure: null,
  ...over,
})

const TABLE: Record<FinderFailureReason, [string, string]> = {
  extension_loading: ['macOS hasn’t finished loading Beebeeb’s Finder extension.', 'Try again'],
  user_disabled: ['Beebeeb is turned off in System Settings.', 'Open System Settings'],
  not_in_applications: ['Beebeeb is running from the disk image. Move it to Applications, then open it again.', 'Show in Finder'],
  folder_taken: ['An older Beebeeb installation still holds Beebeeb’s place in Finder. Contact support and we’ll help you clear it.', 'Copy details'],
  signing: ['This copy of Beebeeb can’t add itself to Finder. Download it again from beebeeb.io.', 'Copy details'],
  timeout: ['macOS didn’t finish adding Beebeeb to Finder.', 'Try again'],
  unknown: ['Beebeeb couldn’t be added to Finder.', 'Try again'],
}

describe('finderSetupCopy (spec §6.2)', () => {
  test('every reason has exactly the sentence and the action of the table', () => {
    expect(Object.keys(FINDER_REASON_COPY).sort()).toEqual([...FINDER_FAILURE_REASONS].sort())
    for (const reason of FINDER_FAILURE_REASONS) {
      const copy = FINDER_REASON_COPY[reason]
      expect([copy.sentence, FINDER_ACTION_LABEL[copy.action]]).toEqual(TABLE[reason])
    }
  })

  test('the fixed lines', () => {
    expect(FINDER_ADDING_LINE).toBe('Adding Beebeeb to Finder…')
    expect(FINDER_READY_LINE).toBe('Your vault appears under Locations in Finder.')
    expect(FINDER_SETUP_TITLE).toBe('Beebeeb in Finder')
  })

  test('each reason presents one notice with its action; user_disabled is a neutral status', () => {
    for (const reason of FINDER_FAILURE_REASONS) {
      const setup = reason === 'user_disabled' ? 'user_disabled' : 'failed'
      expect(finderSetupPresentation(view({ setup, reason }))).toEqual({
        kind: 'notice',
        tone: reason === 'user_disabled' ? 'status' : 'alert',
        reason,
        sentence: TABLE[reason][0],
        action: FINDER_REASON_COPY[reason].action,
        actionLabel: TABLE[reason][1],
      })
    }
  })

  test('the states that are not failures', () => {
    expect(finderSetupPresentation(null)).toEqual({ kind: 'quiet', line: '' })
    expect(finderSetupPresentation(view({ setup: 'missing' }))).toEqual({ kind: 'quiet', line: '' })
    expect(finderSetupPresentation(view({ setup: 'adding' }))).toEqual({ kind: 'adding', line: FINDER_ADDING_LINE })
    expect(finderSetupPresentation(view({ setup: 'ready' }))).toEqual({ kind: 'ready', line: FINDER_READY_LINE })
    expect(finderSetupPresentation(view({ setup: 'failed' }))).toMatchObject({ kind: 'notice', reason: 'unknown' })
  })

  test('Try again appears only inside a failure', () => {
    for (const v of [view({ setup: 'ready' }), view({ setup: 'adding' }), view({ setup: 'missing' }), view({ setup: 'user_disabled', reason: 'user_disabled' })]) {
      const p = finderSetupPresentation(v)
      expect(p.kind === 'notice' ? p.actionLabel : null).not.toBe('Try again')
    }
  })

  test('from the disk image the reason shows before sign-in (D7)', () => {
    expect(finderSetupPresentation(view({ setup: 'missing', reason: 'not_in_applications', launch_location: 'disk_image' }))).toMatchObject({
      kind: 'notice',
      action: 'show_in_finder',
    })
  })

  test('no Finder-setup copy offers to add or install (R5)', () => {
    const all = [
      FINDER_ADDING_LINE,
      FINDER_READY_LINE,
      FINDER_SETUP_TITLE,
      FINDER_UNAVAILABLE_LINE,
      ...Object.values(FINDER_ACTION_LABEL),
      ...Object.values(FINDER_ACTION_FAILED),
      ...Object.values(FINDER_REASON_COPY).map((c) => c.sentence),
      ...Object.values(FINDER_STATUS_PILL),
    ]
    for (const text of all) expect(text).not.toMatch(/Add to Finder|\bInstall\b/)
  })

  // The artefact's own amendment note says the "Finder not added" state is gone (Task 13, ruling R5).
  // That sentence records the removal; it does not draw the state, so it is set aside before
  // looking for the phrase. Anything else that still names it (an image alt, a caption, a board)
  // is a drawing of the state and fails.
  const withoutAmendmentNote = (html: string) => html.replace(/<p class="sub"><strong>Amended 6 Oct 2026<\/strong>.*?<\/p>/s, '')

  test('only the amendment note may name "Finder not added"; a drawing of it still fails', () => {
    const note = '<p class="sub"><strong>Amended 6 Oct 2026</strong> (ruling R5): the “Finder not added” state is gone.</p>'
    expect(withoutAmendmentNote(note)).not.toContain('Finder not added')
    expect(withoutAmendmentNote(`${note}<img alt="Popover, Finder not added state">`)).toContain('Finder not added')
    expect(withoutAmendmentNote(`${note}<figcaption>Finder not added</figcaption>`)).toContain('Finder not added')
  })

  test('the design artefact draws exactly this copy and no "Finder not added" state (spec §12)', () => {
    const html = readFileSync(new URL('../design/hifi/macos-menubar-popover.html', import.meta.url), 'utf8')
    expect(withoutAmendmentNote(html)).not.toContain('Finder not added')
    expect(html).toContain(FINDER_ADDING_LINE)
    for (const reason of FINDER_FAILURE_REASONS) {
      expect(html).toContain(`<span class="board-tag">${reason}</span><p>${FINDER_REASON_COPY[reason].sentence}</p>`)
      expect(html).toContain(`<span class="board-btn">${FINDER_ACTION_LABEL[FINDER_REASON_COPY[reason].action]}</span>`)
    }
  })
})

describe('the Finder state could not be read (lead ruling 7a)', () => {
  test('it reuses the existing Sync-tab words: no new user-facing string', () => {
    // 'Couldn’t check Finder.' is what MacSettings' unavailable row already says
    // (macSettingsModel.finderHint), and 'Try again' is the button it already shows.
    expect(FINDER_UNAVAILABLE_LINE).toBe(finderHint({ kind: 'unavailable' }))
    expect(FINDER_UNAVAILABLE_LINE).toBe('Couldn’t check Finder.')
    expect(FINDER_ACTION_LABEL.try_again).toBe('Try again')
  })

  test('a failed load is an unavailable notice with ONE action, never "Adding"', () => {
    expect(finderSetupLoadPresentation({ status: 'unavailable' })).toEqual({
      kind: 'unavailable',
      line: FINDER_UNAVAILABLE_LINE,
      actionLabel: 'Try again',
    })
  })

  test('a load that has not settled is quiet; a loaded view presents as before', () => {
    expect(finderSetupLoadPresentation({ status: 'loading' })).toEqual({ kind: 'quiet', line: '' })
    for (const v of [view({ setup: 'adding' }), view({ setup: 'ready' }), view({ setup: 'failed', reason: 'timeout' })]) {
      expect(finderSetupLoadPresentation({ status: 'loaded', view: v })).toEqual(finderSetupPresentation(v))
    }
  })
})

describe('copyFinderSetupDetails', () => {
  const previous = (globalThis as any).window
  afterEach(() => { (globalThis as any).window = previous })
  const backend = (answer: () => unknown) => {
    ;(globalThis as any).window = { __TAURI_INTERNALS__: { invoke: async (name: string) => (name === 'finder_setup_copy_details' ? answer() : undefined) } }
  }

  test('puts the command text on the pasteboard', async () => {
    backend(() => 'Beebeeb 0.8.12 on macOS 26.0')
    const written: string[] = []
    const result = await copyFinderSetupDetails({ writeClipboard: async (text) => { written.push(text) } })
    expect(result.ok).toBe(true)
    expect(written).toEqual(['Beebeeb 0.8.12 on macOS 26.0'])
  })

  test('a refused pasteboard is a plain failed result, for the surface to toast', async () => {
    backend(() => 'x')
    const result = await copyFinderSetupDetails({ writeClipboard: async () => { throw new Error('denied') } })
    expect(result).toEqual({ ok: false, reason: 'The details could not be put on the pasteboard.', unsupported: false })
  })
})
