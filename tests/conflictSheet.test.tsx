/**
 * The conflict sheet (task 1683 slice 4; spec §7.6 "Conflict sheet (520 pt, from ConflictWindow.tsx)", §10.2 row 35,
 * hi-fi section 3): the real `ConflictSheet` and `ConflictSide` declarations, executed against a scripted backend.
 * What this proves: what the sheet asks the engine for and in which words it shows it, the three choices and what
 * each sends, that a success closes it and the row leaves, that a failure leaves it open with one toast, that nothing
 * can close it while a choice is being applied, and that it opens on its close button so Enter on open is Cancel.
 * What it does NOT prove: the engine's resolve itself (src-tauri), focus in a real webview, layout.
 */
import { afterEach, beforeEach, describe, expect, test } from 'bun:test'
import { createElement, Fragment } from 'react'
import * as desktopApi from '../src/desktopApi'
import { diffLines, DIFF_MAX_OPS } from '../src/diffLines'
import * as parts from '../src/macSettingsParts'
import * as reviewModel from '../src/reviewModel'
import { formatStorageSize } from '../src/storageFormat'
import { loadComponent, mount, type Mounted } from './fixtures/componentHarness'
import { button, buttons, buttonLabel, find, lines, modalNow, modalStub, press, settle } from './fixtures/sheetHarness'

const React = { createElement, Fragment }
const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close() })
// The question "Use the other version here?" ignores an answer that comes within a moment of it (the same press, twice): the
// clock is ours, so a test says when the person answers.
const realNow = Date.now
let clock = 1_000_000
beforeEach(() => { clock = 1_000_000; Date.now = () => clock })
afterEach(() => { Date.now = realNow })
const aMomentLater = () => { clock += reviewModel.CONFLICT_REPLACE_GUARD_MS + 100 }

const entry: desktopApi.VersionConflictEntry = {
  id: 'conflict:f1', file_id: 'f1', file_name: 'Board notes, 29 Sept.md', kind: 'conflict', status: 'needs review',
  updated_at: 1000, detail: 'x', action: 'open_conflict',
}

const textPreview = (local: string, remote: string) => ({
  is_text: true,
  local: { size_bytes: local.length, text: local },
  remote: { size_bytes: remote.length, text: remote },
})
const binaryPreview = () => ({
  is_text: false,
  local: { size_bytes: 4096, text: null },
  remote: { size_bytes: 8192, text: null },
})

const ConflictLine = loadComponent('ConflictSheet.tsx', 'ConflictLine', { React })
const ConflictSide = loadComponent('ConflictSheet.tsx', 'ConflictSide', { React, ConflictLine, formatStorageSize })

const focused: string[] = []
beforeEach(() => { focused.length = 0 })

function open(opts: { preview?: () => unknown; resolve?: (args: any) => unknown } = {}) {
  const modal = modalStub()
  const closed: string[] = []
  const m = mount('ConflictSheet.tsx', 'ConflictSheet', {
    expand: true,
    backend: {
      conflict_content_preview: opts.preview ?? (() => textPreview('# Agenda\nitem 1\nitem 4 (added here)\n', '# Agenda\nitem 1\nitem 2 (moved)\n')),
      resolve_conflict: opts.resolve ?? (() => null),
    } as any,
    props: { entry, onClose: () => closed.push('closed'), onResolved: () => closed.push('resolved') },
    bindings: {
      ...desktopApi,
      ...reviewModel,
      diffLines, DIFF_MAX_OPS, formatStorageSize,
      Btn: parts.Btn,
      Modal: modal.Modal,
      ConflictSide, ConflictLine,
      focusById: (id: string) => { focused.push(id) },
    },
  })
  mounted.push(m)
  return { m, modal, closed }
}

describe('opening', () => {
  test('it asks the engine for both sides of THIS file and says it is loading until they arrive', async () => {
    const { m } = open()
    expect(lines(m)).toContain('Loading both versions…')
    await settle(m)
    expect(m.calls.filter((c) => c.name === 'conflict_content_preview').map((c) => c.args)).toEqual([{ fileId: 'f1' }])
    expect(lines(m)).not.toContain('Loading both versions…')
  })

  test('the sheet is 520 pt wide, named for the file, with the hi-fi\'s sentence and the two sides in its words', async () => {
    const { m, modal } = open()
    await settle(m)
    expect(modalNow(m, modal)).toMatchObject({ open: true, maxWidth: 520, title: 'Conflict: Board notes, 29 Sept.md', ariaLabel: 'Conflict: Board notes, 29 Sept.md' })
    const all = lines(m)
    expect(all).toContain('This file was modified on two devices. Choose which version to keep.')
    expect(all).toContain('This Mac')
    expect(all).toContain('Other device')
    for (const retired of ['This device', 'Keep Mine', 'Keep Theirs', 'Keep Both', 'Conflict window missing context']) expect(all.join(' ')).not.toContain(retired)
  })

  test('it opens on its close button, never on a choice: Enter on open is Cancel (§11)', async () => {
    const { m, modal } = open()
    await settle(m)
    expect(modalNow(m, modal).initialFocusRef).toBeUndefined()
  })
})

describe('the two sides', () => {
  test('a text file shows each side\'s own lines: what only this Mac has with "-", what only the other has with "+", shared lines on both', async () => {
    const { m } = open()
    await settle(m)
    const rows = find(m, (el) => typeof el.props['data-line'] === 'string')
    const byKind = (kind: string) => rows.filter((el) => el.props['data-line'] === kind).map((el) => String(el.props.children).replace(/\s+/g, ' ').trim())
    expect(byKind('remove').join('|')).toContain('item 4 (added here)')
    expect(byKind('add').join('|')).toContain('item 2 (moved)')
    expect(byKind('same').length).toBeGreaterThanOrEqual(2)
    // the line that exists on one side only is not shown on the other
    const sides = find(m, (el) => typeof el.props['data-side'] === 'string')
    const local = sides.find((el) => el.props['data-side'] === 'local')!
    const remote = sides.find((el) => el.props['data-side'] === 'remote')!
    const flat = (el: any) => JSON.stringify(expandText(el))
    expect(flat(local)).toContain('item 4 (added here)')
    expect(flat(local)).not.toContain('item 2 (moved)')
    expect(flat(remote)).toContain('item 2 (moved)')
    expect(flat(remote)).not.toContain('item 4 (added here)')
  })

  test('a binary file shows each side\'s size in the Mac\'s units, and no invented content', async () => {
    const { m } = open({ preview: binaryPreview })
    await settle(m)
    const all = lines(m).join(' | ')
    expect(all).toContain(`Binary file · ${formatStorageSize(4096)}`)
    expect(all).toContain(`Binary file · ${formatStorageSize(8192)}`)
  })

  test('a side that cannot be shown says why, in the engine\'s words', async () => {
    const { m } = open({
      preview: () => ({ is_text: true, local: { size_bytes: 9, text: 'mine' }, remote: { size_bytes: null, text: null, unavailable_reason: 'The other copy is still downloading.' } }),
    })
    await settle(m)
    expect(lines(m)).toContain('The other copy is still downloading.')
  })

  test('two texts too long to compare line by line are shown whole, and the sheet says so', async () => {
    const long = (tag: string) => Array.from({ length: 2500 }, (_, i) => `${tag} line ${i}`).join('\n')
    const { m } = open({ preview: () => textPreview(long('a'), long('b')) })
    await settle(m)
    expect(lines(m).join(' ')).toContain('too large to compare line by line')
  })

  test('a preview that cannot load is one sentence with the reason, and the three choices still work', async () => {
    const { m } = open({ preview: () => { throw new Error('download failed') } })
    await settle(m)
    expect(lines(m).join(' ')).toContain('Couldn’t load either version’s content: download failed. You can still choose a version below.')
    expect(buttons(m).map(buttonLabel)).toEqual(expect.arrayContaining(['Keep this Mac’s', 'Keep the other', 'Keep both']))
  })
})

describe('the three choices (§7.6)', () => {
  test('they are in this order, and only "Keep both" is the amber primary', async () => {
    const { m } = open()
    await settle(m)
    const choices = buttons(m).filter((el) => ['Keep this Mac’s', 'Keep the other', 'Keep both'].includes(buttonLabel(el)))
    expect(choices.map(buttonLabel)).toEqual(['Keep this Mac’s', 'Keep the other', 'Keep both'])
    expect(choices.map((el) => String(el.props.className).includes('ms-btn--primary'))).toEqual([false, false, true])
  })

  for (const [label, choice] of [['Keep this Mac’s', 'local'], ['Keep both', 'both']] as const) {
    test(`"${label}" sends "${choice}" for this file and nothing else`, async () => {
      const { m } = open()
      await settle(m)
      await press(m, label)
      expect(m.calls.filter((c) => c.name === 'resolve_conflict').map((c) => c.args)).toEqual([{ fileId: 'f1', choice }])
    })
  }

  test('"Keep the other" asks first, and the second press sends "remote" for this file and nothing else', async () => {
    const { m } = open()
    await settle(m)
    await press(m, 'Keep the other')
    expect(m.calls.filter((c) => c.name === 'resolve_conflict')).toEqual([])
    aMomentLater()
    await press(m, 'Use the other version')
    expect(m.calls.filter((c) => c.name === 'resolve_conflict').map((c) => c.args)).toEqual([{ fileId: 'f1', choice: 'remote' }])
  })

  test('success: one "Conflict resolved" toast, the row\'s owner told, then the sheet closes (in that order)', async () => {
    const { m, closed } = open()
    await settle(m)
    await press(m, 'Keep both')
    expect(m.toasts).toEqual([{ variant: 'success', message: 'Conflict resolved' }])
    expect(closed).toEqual(['resolved', 'closed'])
  })

  test('failure: the sheet stays, one error toast says why, and the choices work again', async () => {
    const { m, closed } = open({ resolve: () => { throw new Error('no state.db row for f1') } })
    await settle(m)
    await press(m, 'Keep the other')
    aMomentLater()
    await press(m, 'Use the other version')
    expect(closed).toEqual([])
    expect(m.toasts).toHaveLength(1)
    expect(m.toasts[0]).toMatchObject({ variant: 'error', title: 'Couldn’t resolve this conflict' })
    expect(String(m.toasts[0].message)).toContain('no state.db row for f1')
    expect(buttons(m).filter((el) => ['Keep this Mac’s', 'Keep the other', 'Keep both'].includes(buttonLabel(el))).every((el) => !el.props.disabled)).toBe(true)
  })

  test('a build without the command says so in the shared words, not in the transport\'s', async () => {
    const { m, closed } = open({ resolve: () => { throw new Error('unknown command resolve_conflict') } })
    await settle(m)
    await press(m, 'Keep both')
    expect(m.toasts).toEqual([{ variant: 'error', title: 'Couldn’t resolve this conflict', message: desktopApi.commandUnavailableLabel('resolve_conflict') }])
    expect(closed).toEqual([])
  })
})

describe('"Keep the other" can leave this Mac with nothing of its own version, so it asks first (review of #142)', () => {
  const noteOf = (m: Mounted) => find(m, (el) => el.props['data-note'] === 'replace')
  const footer = (m: Mounted) => buttons(m).map(buttonLabel).filter((label) => label !== 'Close')

  test('nothing is asked until it is pressed; the first press sends nothing and asks, aloud, a question that claims nothing about which file', async () => {
    const { m } = open()
    await settle(m)
    expect(noteOf(m)).toHaveLength(0)
    expect(focused, 'nothing is focused before it asks').toEqual([])
    await press(m, 'Keep the other')
    expect(m.calls.filter((c) => c.name === 'resolve_conflict')).toEqual([])
    const note = noteOf(m)
    expect(note).toHaveLength(1)
    expect(note[0].props.role).toBe('alert')
    expect(lines(m)).toContain('Use the other version here?')
    expect(lines(m).join(' ')).not.toMatch(/discard|Replace|overwrite|Finder/)
  })

  test('the answer is Cancel (first, and focused) or "Use the other version" (the one red button): the three choices are put away', async () => {
    const { m } = open()
    await settle(m)
    await press(m, 'Keep the other')
    expect(footer(m)).toEqual(['Cancel', 'Use the other version'])
    // The button that asked left the footer, so the focus is put on the safe answer (Return on it cancels).
    expect(button(m, 'Cancel').props.id).toBe('conflict-replace-cancel')
    expect(focused).toEqual(['conflict-replace-cancel'])
    expect(button(m, 'Use the other version').props.className).toContain('ms-btn--danger')
    expect(buttons(m).some((el) => el.props.className?.includes('ms-btn--primary'))).toBe(false)
  })

  test('Cancel sends nothing and puts the question away: the three choices are back, and "Keep the other" asks again', async () => {
    const { m } = open()
    await settle(m)
    await press(m, 'Keep the other')
    aMomentLater()
    await press(m, 'Cancel')
    expect(m.calls.filter((c) => c.name === 'resolve_conflict')).toEqual([])
    expect(noteOf(m)).toHaveLength(0)
    expect(footer(m)).toEqual(['Keep this Mac’s', 'Keep the other', 'Keep both'])
    await press(m, 'Keep the other')
    expect(noteOf(m)).toHaveLength(1)
  })

  test('the press that asked is not the answer: a second activation within a moment (a double click, a held Return) is ignored', async () => {
    const { m } = open()
    await settle(m)
    await press(m, 'Keep the other')
    clock += reviewModel.CONFLICT_REPLACE_GUARD_MS - 1
    await press(m, 'Use the other version')
    expect(m.calls.filter((c) => c.name === 'resolve_conflict')).toEqual([])
    expect(noteOf(m)).toHaveLength(1) // still asking
    clock += 1
    await press(m, 'Use the other version')
    expect(m.calls.filter((c) => c.name === 'resolve_conflict').map((c) => c.args)).toEqual([{ fileId: 'f1', choice: 'remote' }])
  })

  test('"Keep this Mac\'s" and "Keep both" never ask, and send their own choice', async () => {
    for (const [label, choice] of [['Keep this Mac’s', 'local'], ['Keep both', 'both']] as const) {
      const { m } = open()
      await settle(m)
      await press(m, label)
      expect(noteOf(m)).toHaveLength(0)
      expect(m.calls.filter((c) => c.name === 'resolve_conflict').map((c) => c.args)).toEqual([{ fileId: 'f1', choice }])
    }
  })

  test('a failed answer puts the question away: the three choices are back, and "Keep the other" asks again', async () => {
    let fail = true
    const { m } = open({ resolve: () => { if (fail) throw new Error('offline'); return null } })
    await settle(m)
    await press(m, 'Keep the other')
    aMomentLater()
    await press(m, 'Use the other version')
    expect(noteOf(m)).toHaveLength(0)
    expect(footer(m)).toEqual(['Keep this Mac’s', 'Keep the other', 'Keep both'])
    fail = false
    await press(m, 'Keep the other')
    expect(m.calls.filter((c) => c.name === 'resolve_conflict')).toHaveLength(1)
    aMomentLater()
    await press(m, 'Use the other version')
    expect(m.calls.filter((c) => c.name === 'resolve_conflict')).toHaveLength(2)
  })

  test('while the answer is being applied both buttons are off', async () => {
    let release: () => void = () => {}
    const { m } = open({ resolve: () => new Promise<null>((resolve) => { release = () => resolve(null) }) })
    await settle(m)
    await press(m, 'Keep the other')
    aMomentLater()
    await m.clickNoWait('Use the other version')
    expect(buttons(m).filter((el) => ['Cancel', 'Use the other version'].includes(buttonLabel(el))).every((el) => el.props.disabled === true)).toBe(true)
    release()
    await settle(m)
  })
})

describe('while a choice is being applied', () => {
  test('all three are disabled, and Esc and the close button do nothing (closing would hide the outcome)', async () => {
    let release: () => void = () => {}
    const { m, modal, closed } = open({ resolve: () => new Promise<null>((resolve) => { release = () => resolve(null) }) })
    await settle(m)
    await m.clickNoWait('Keep both')
    const choices = buttons(m).filter((el) => ['Keep this Mac’s', 'Keep the other', 'Keep both'].includes(buttonLabel(el)))
    expect(choices.every((el) => el.props.disabled === true)).toBe(true)
    modalNow(m, modal).onClose()
    expect(closed).toEqual([])
    release()
    await settle(m)
    expect(closed).toEqual(['resolved', 'closed'])
  })

  test('before any choice, Esc and the close button cancel: nothing changed, nothing was sent', async () => {
    const { m, modal, closed } = open()
    await settle(m)
    modalNow(m, modal).onClose()
    expect(closed).toEqual(['closed'])
    expect(m.calls.filter((c) => c.name === 'resolve_conflict')).toHaveLength(0)
    expect(m.toasts).toEqual([])
  })
})

describe('the pieces', () => {
  test('a diff line is a prefix and the text, in the mono face, marked with its kind', () => {
    const el = ConflictLine({ line: 'item 4', type: 'remove' }) as any
    expect(el.props['data-line']).toBe('remove')
    expect(String(el.props.className)).toContain('mw-diff-line')
    expect(textOfNode(el)).toBe('- item 4')
    expect(textOfNode(ConflictLine({ line: 'item 2', type: 'add' }))).toBe('+ item 2')
    expect(textOfNode(ConflictLine({ line: 'same', type: 'same' }))).toBe('  same')
  })

  test('diff ops for both sides come from the shared diff, so the sheet and the Windows window agree', () => {
    const ops = diffLines('a\nb\n', 'a\nc\n').ops
    expect(ops.map((o) => o.type)).toContain('remove')
    expect(ops.map((o) => o.type)).toContain('add')
  })
})

/** Text of a created element tree (no component expansion needed: these are DOM elements). */
function textOfNode(node: any): string {
  if (Array.isArray(node)) return node.map(textOfNode).join('')
  if (node?.props) return textOfNode(node.props.children)
  return node == null || typeof node === 'boolean' ? '' : String(node)
}
function expandText(node: any): string {
  return textOfNode(node)
}

// `button` is re-exported for the other sheet tests; referenced so the import is checked here too.
void button
