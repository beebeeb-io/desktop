/**
 * Spec 2026-10-06 §6.2 and §10: every Finder-setup string lives in src/finderSetupCopy.ts, keyed
 * by reason; each reason is one sentence and one action; the design artefact draws exactly this
 * copy (§12). Strings use the house typographic apostrophe (plan "Spec issues" 15).
 */
import { afterEach, describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { copyFinderSetupDetails, FINDER_FAILURE_REASONS, finderOpenFailedToast, finderRepairFailedToast, finderShowFileFailedToast, type FinderFailureReason, type FinderSetupView } from '../src/finderSetup'
import {
  FINDER_ACTION_FAILED,
  FINDER_ACTION_LABEL,
  FINDER_ADDING_LINE,
  FINDER_OPEN_FAILED,
  FINDER_REPAIR_ENGINE_UNCONFIRMED,
  FINDER_REPAIR_FAILED,
  FINDER_REPAIR_PARTIAL,
  FINDER_READY_LINE,
  FINDER_REASON_COPY,
  FINDER_RESTING_LINE,
  FINDER_SETUP_TITLE,
  FINDER_SHOW_FILE_FAILED,
  FINDER_STATUS_PILL,
  FINDER_STATUS_PILL_LOADING,
  FINDER_STATUS_PILL_UNAVAILABLE,
  FINDER_UNAVAILABLE_LINE,
  finderSetupLoadPresentation,
  finderSetupPresentation,
  finderRepairWarningNote,
  finderStatusPill,
  type FinderSetupAction,
} from '../src/finderSetupCopy'
import { finderHint } from '../src/macSettingsModel'
import { rustStr } from './fixtures/rustConstants'

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
    expect(finderSetupPresentation(view({ setup: 'missing' }))).toEqual({ kind: 'resting', line: FINDER_RESTING_LINE })
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
      FINDER_RESTING_LINE,
      FINDER_SETUP_TITLE,
      FINDER_UNAVAILABLE_LINE,
      FINDER_OPEN_FAILED,
      FINDER_REPAIR_FAILED,
      FINDER_REPAIR_PARTIAL,
      FINDER_SHOW_FILE_FAILED,
      ...Object.values(FINDER_ACTION_LABEL),
      ...Object.values(FINDER_ACTION_FAILED),
      ...Object.values(FINDER_REASON_COPY).map((c) => c.sentence),
      ...Object.values(FINDER_STATUS_PILL),
      FINDER_STATUS_PILL_LOADING,
      FINDER_STATUS_PILL_UNAVAILABLE,
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

/**
 * Lead ruling FA-I2 (it corrects FT-I2's copy), spec §5.2 as amended: a loaded `Missing` means "no
 * check owns Finder right now; Beebeeb may or may not still be registered". It rests there after a
 * sign-out, after a Lock that interrupts a check, and while the keys are not on this Mac. So it is a
 * quiet row: no pill, no activity, no claim about presence, and one sentence where a slot needs text.
 */
describe('a loaded Missing is a quiet row (FA-I2)', () => {
  test('its one sentence, with the house apostrophe', () => {
    expect(FINDER_RESTING_LINE).toBe('Beebeeb adds itself to Finder when you’re signed in and the vault is unlocked.')
    expect(FINDER_RESTING_LINE.match(/[.!?]/g)).toHaveLength(1)
  })

  test('it claims no activity and nothing about presence', () => {
    expect(FINDER_RESTING_LINE).not.toMatch(/Adding|Checking|…|isn’t in Finder|is in Finder|not in Finder|appears/i)
  })

  test('a loaded Missing presents as resting; before any answer it is still quiet (nothing to say yet)', () => {
    expect(finderSetupLoadPresentation({ status: 'loaded', view: view({ setup: 'missing' }) })).toEqual({ kind: 'resting', line: FINDER_RESTING_LINE })
    expect(finderSetupLoadPresentation({ status: 'loading' })).toEqual({ kind: 'quiet', line: '' })
  })

  test('a Missing with a reason is still that reason\'s notice (D7), not the quiet row', () => {
    expect(finderSetupPresentation(view({ setup: 'missing', reason: 'not_in_applications' }))).toMatchObject({ kind: 'notice', reason: 'not_in_applications' })
  })

  test('it has no pill', () => {
    expect(finderStatusPill({ status: 'loaded', view: view({ setup: 'missing' }) })).toBeNull()
  })
})

/**
 * Must-render row 9 (FT-I5): when the engine start was refused, the reconciler's re-add fails as
 * `unknown`, and "Beebeeb couldn’t be added to Finder." names the wrong cause. A `failed` + `unknown`
 * with a refusal present says the refusal's sentence instead. The action stays Try again (Lane R's
 * retry re-identifies), except for `engine_stop_unconfirmed`, where only quitting and reopening helps,
 * so there is no action. One test per code.
 */
describe('an engine refusal replaces the generic unknown sentence (row 9)', () => {
  const refusal = (code: 'identity_unknown' | 'other_account_on_windows' | 'engine_stop_unconfirmed', file = 'account_binding.rs') => ({
    code,
    sentence: rustStr(file, code.toUpperCase()),
  })
  const unknownFailure = view({ setup: 'failed', reason: 'unknown' })

  test('identity_unknown: its sentence, and Try again', () => {
    const r = refusal('identity_unknown')
    expect(finderSetupPresentation(unknownFailure, r)).toEqual({
      kind: 'notice', tone: 'alert', reason: 'unknown', sentence: r.sentence, action: 'try_again', actionLabel: 'Try again',
    })
  })

  test('other_account_on_windows: its sentence, and Try again', () => {
    const r = refusal('other_account_on_windows')
    expect(finderSetupPresentation(unknownFailure, r)).toEqual({
      kind: 'notice', tone: 'alert', reason: 'unknown', sentence: r.sentence, action: 'try_again', actionLabel: 'Try again',
    })
  })

  test('engine_stop_unconfirmed: its sentence, and no action (only a relaunch helps)', () => {
    const r = refusal('engine_stop_unconfirmed')
    expect(finderSetupPresentation(unknownFailure, r)).toEqual({
      kind: 'notice', tone: 'alert', reason: 'unknown', sentence: r.sentence, action: null, actionLabel: null,
    })
  })

  test('a failure with no reason is unknown too, so the refusal applies', () => {
    expect(finderSetupPresentation(view({ setup: 'failed', reason: null }), refusal('identity_unknown'))).toMatchObject({ sentence: refusal('identity_unknown').sentence })
  })

  test('any other reason or state keeps its own presentation', () => {
    const r = refusal('identity_unknown')
    for (const v of [view({ setup: 'failed', reason: 'timeout' }), view({ setup: 'adding' }), view({ setup: 'ready' }), view({ setup: 'missing' }), view({ setup: 'user_disabled', reason: 'user_disabled' })]) {
      expect(finderSetupPresentation(v, r)).toEqual(finderSetupPresentation(v))
    }
  })

  test('the load presentation passes the refusal through, and the pill stays the notice\'s', () => {
    const r = refusal('engine_stop_unconfirmed')
    expect(finderSetupLoadPresentation({ status: 'loaded', view: unknownFailure }, r)).toEqual(finderSetupPresentation(unknownFailure, r))
    expect(finderStatusPill({ status: 'loaded', view: unknownFailure })).toEqual({ label: 'Setup blocked', tone: 'error' })
  })
})

describe('the Finder state could not be read (lead ruling 7a)', () => {
  test('it reuses the existing Sync-tab words: no new user-facing string', () => {
    // 'Couldn’t check Finder.' is what MacSettings' unavailable row has always said. Task 16 made
    // `finderHint` read this constant (one copy, in this module), so the literal below is the pin
    // and `finderHint` is held to it, instead of the two being compared with each other.
    expect(FINDER_UNAVAILABLE_LINE).toBe('Couldn’t check Finder.')
    expect(finderHint({ kind: 'unavailable' })).toBe('Couldn’t check Finder.')
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

/**
 * Task 17b (lead ruling T4-⚠2). On a Mac every error from the File Provider bridge that reaches
 * the frontend is redacted to a domain and a code (ruling T1-⚠4), so a failed "Open in Finder"
 * must say one sentence of its own. A failed action that gates nothing is a toast.
 */
describe('a failed Open in Finder on a Mac (task 17b)', () => {
  test('is one sentence, with the typographic apostrophe', () => {
    expect(FINDER_OPEN_FAILED).toBe('Beebeeb couldn’t open its Finder location.')
    expect(FINDER_OPEN_FAILED.match(/[.!?]/g)).toHaveLength(1)
  })

  test('is one error toast that carries the sentence and nothing else', () => {
    expect(finderOpenFailedToast()).toEqual({ variant: 'error', message: FINDER_OPEN_FAILED })
  })
})

/**
 * Task 17b, lead ruling on its concern 2: the four actions of `useFinderSetup().run` fail to one
 * fixed sentence each, never to `result.reason` (a redacted bridge code on a Mac).
 */
describe('a failed Finder repair on a Mac says one sentence (task 17b)', () => {
  test('is one sentence, with the typographic apostrophe', () => {
    expect(FINDER_REPAIR_FAILED).toBe('Beebeeb couldn’t repair its Finder location.')
    expect(FINDER_REPAIR_FAILED.match(/[.!?]/g)).toHaveLength(1)
  })

  test('is one error toast that carries the sentence and nothing else', () => {
    expect(finderRepairFailedToast()).toEqual({ variant: 'error', message: FINDER_REPAIR_FAILED })
  })
})

describe('a repair that succeeds with warnings says one sentence (task 17b, fix round 1)', () => {
  // Rust puts a bridge code (lib.rs:3326) and a cache-file path (lib.rs:3265) in `warnings`.
  const LEAKS = ['io.beebeeb.bridge 3', '/Users/sam/Library/x.db']

  test('is one sentence, with the typographic apostrophe', () => {
    expect(FINDER_REPAIR_PARTIAL).toBe('Beebeeb repaired its Finder location, but couldn’t remove everything it left behind.')
    expect(FINDER_REPAIR_PARTIAL.match(/[.!?]/g)).toHaveLength(1)
  })

  test('any non-blank warning gives the sentence and none of the warning text; blank or no warnings give nothing', () => {
    expect(finderRepairWarningNote({ warnings: LEAKS })).toBe(FINDER_REPAIR_PARTIAL)
    expect(finderRepairWarningNote({ warnings: ['  ', LEAKS[0]] })).toBe(FINDER_REPAIR_PARTIAL)
    expect(finderRepairWarningNote({ warnings: [] })).toBeNull()
    expect(finderRepairWarningNote({ warnings: ['', '   '] })).toBeNull()
    for (const leak of LEAKS) expect(finderRepairWarningNote({ warnings: LEAKS })).not.toContain(leak)
  })
})

/**
 * Must-render row 11, lead ruling FT-I3 (it corrects T17b-warnings, it does not overturn it): when
 * Repair's engine stop (or an earlier one) is unconfirmed, sync starts again only after Beebeeb is
 * quit and reopened. Lane R flags it as `engine_stop_unconfirmed` on the repair result; a Mac shows
 * one fixed sentence for it, AHEAD of the partial-cleanup sentence. No string matching on `warnings`.
 */
describe('a repair whose engine stop is unconfirmed says to quit and reopen (row 11)', () => {
  test('the one sentence, with the house apostrophes, the same words as the refusal Rust records', () => {
    expect(FINDER_REPAIR_ENGINE_UNCONFIRMED).toBe('Beebeeb’s sync didn’t confirm it stopped. Quit and reopen Beebeeb before syncing again.')
    expect(FINDER_REPAIR_ENGINE_UNCONFIRMED).toBe(rustStr('account_binding.rs', 'ENGINE_STOP_UNCONFIRMED'))
  })

  test('the flag alone: the sentence', () => {
    expect(finderRepairWarningNote({ warnings: [], engine_stop_unconfirmed: true })).toBe(FINDER_REPAIR_ENGINE_UNCONFIRMED)
  })

  test('the flag and warnings: the sentence first, then the partial-cleanup sentence', () => {
    expect(finderRepairWarningNote({ warnings: ['io.beebeeb.bridge 3'], engine_stop_unconfirmed: true })).toBe(
      `${FINDER_REPAIR_ENGINE_UNCONFIRMED} ${FINDER_REPAIR_PARTIAL}`,
    )
  })

  test('no flag: the warnings decide alone, and the warning text is never read for the engine', () => {
    const engineWarning = rustStr('lib.rs', 'REPAIR_ENGINE_UNCONFIRMED_WARNING')
    expect(finderRepairWarningNote({ warnings: [engineWarning], engine_stop_unconfirmed: false })).toBe(FINDER_REPAIR_PARTIAL)
    expect(finderRepairWarningNote({ warnings: [], engine_stop_unconfirmed: false })).toBeNull()
  })
})

describe('a failed "show that file in Finder" on a Mac says one sentence (task 17b, fix round 1)', () => {
  test('is one sentence, with the typographic apostrophe', () => {
    expect(FINDER_SHOW_FILE_FAILED).toBe('Beebeeb couldn’t show that file in Finder.')
    expect(FINDER_SHOW_FILE_FAILED.match(/[.!?]/g)).toHaveLength(1)
  })

  test('is one error toast that carries the sentence and nothing else', () => {
    expect(finderShowFileFailedToast()).toEqual({ variant: 'error', message: FINDER_SHOW_FILE_FAILED })
  })
})

describe('a failed Finder action on a Mac says one sentence (task 17b)', () => {
  const TABLE: Record<FinderSetupAction, string> = {
    try_again: 'Beebeeb couldn’t retry adding itself to Finder.',
    copy_details: 'Beebeeb couldn’t copy the details.',
    show_in_finder: 'Beebeeb couldn’t show itself in Finder.',
    open_system_settings: 'Beebeeb couldn’t open System Settings.',
  }

  test('each action has exactly its sentence, one sentence long', () => {
    expect(Object.keys(FINDER_ACTION_FAILED).sort()).toEqual(Object.keys(TABLE).sort())
    for (const [action, sentence] of Object.entries(TABLE)) {
      expect(FINDER_ACTION_FAILED[action as FinderSetupAction]).toBe(sentence)
      expect(sentence.match(/[.!?]/g)).toHaveLength(1)
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

/**
 * Task 17. The compact SyncFolder and Status pages show one short pill. Both read it from here so
 * the two cannot drift, and an unreadable state has a pill of its own (lead ruling 7a: it is never
 * "Adding").
 */
describe('the status pill of the compact pages', () => {
  test('one label and one tone per state', () => {
    const pill = (v: FinderSetupView) => finderStatusPill({ status: 'loaded', view: v })
    expect(finderStatusPill({ status: 'loading' })).toEqual({ label: 'Loading', tone: 'idle' })
    expect(finderStatusPill({ status: 'unavailable' })).toEqual({ label: 'Unknown', tone: 'warn' })
    expect(pill(view({ setup: 'ready' }))).toEqual({ label: 'Installed', tone: 'ok' })
    expect(pill(view({ setup: 'adding' }))).toEqual({ label: 'Adding', tone: 'warn' })
    expect(pill(view({ setup: 'failed', reason: 'timeout' }))).toEqual({ label: 'Setup blocked', tone: 'error' })
    expect(pill(view({ setup: 'user_disabled', reason: 'user_disabled' }))).toEqual({ label: 'Turned off', tone: 'warn' })
    expect(pill(view({ setup: 'missing' }))).toBeNull()
  })

  // FA-I2 / FT-I2: the pill is keyed on what the surface PRESENTS, not on the raw state, so a
  // `missing` that carries a reason (D7: running from the disk image) is a notice with a notice's
  // label, never "Checking" next to a red dot.
  test('a notice carries its own label whatever state it came from; no pill ever says "Checking"', () => {
    const pill = (v: FinderSetupView) => finderStatusPill({ status: 'loaded', view: v })
    expect(pill(view({ setup: 'missing', reason: 'not_in_applications', launch_location: 'disk_image' }))).toEqual({ label: 'Setup blocked', tone: 'error' })
    const labels = [
      ...Object.values(FINDER_STATUS_PILL),
      FINDER_STATUS_PILL_LOADING,
      FINDER_STATUS_PILL_UNAVAILABLE,
      ...(['ready', 'missing', 'adding', 'failed', 'user_disabled'] as const).flatMap((setup) =>
        [null, ...FINDER_FAILURE_REASONS].map((reason) => pill(view({ setup, reason }))?.label ?? null),
      ),
    ]
    expect(labels.filter((label) => label !== null && /Checking/.test(label))).toEqual([])
  })

  test('an unreadable state is not "Adding"', () => {
    expect(finderStatusPill({ status: 'unavailable' }).label).not.toMatch(/Add/)
    expect(FINDER_STATUS_PILL_UNAVAILABLE).not.toBe(FINDER_STATUS_PILL.adding)
  })
})
