/**
 * Task 2267: the Settings › Sync card for changes found while sync was paused. The real `FoundWhilePausedCard`
 * declaration runs with the component harness against a scripted backend: what it shows, which command each button
 * sends (none of them carries a path), the inline confirmation, the busy state, the keep result and an error.
 * What this does not prove: browser layout, WebView2, or Explorer opening (the Windows device rung).
 */
import { afterEach, describe, expect, test } from 'bun:test'
import * as desktopApi from '../src/desktopApi'
import * as model from '../src/windows/foundWhilePaused'
import { CardButton } from '../src/windows/FoundWhilePausedCard'
import type { SyncActivitySummary } from '../src/windows/syncActivityModel'
import { Card, NavIcon, T } from '../src/windows/ui'
import { mount, textOf, type Handler, type Mounted } from './fixtures/componentHarness'

const KEPT = 'C:\\Users\\Fixture\\Beebeeb - not uploaded\\2026-10-28'
const COPY = model.FOUND_WHILE_PAUSED_COPY

const mounted: Mounted[] = []
afterEach(() => {
  while (mounted.length) mounted.pop()!.close()
})

function summaryHolding(count: number): SyncActivitySummary {
  return {
    paused: false,
    engine_state: 'idle',
    files_remaining: null,
    transfers_active: 0,
    last_tick_ok_at: 1_793_000_000,
    queue: { waiting: 0, gave_up: 0, held: count > 0 ? [{ reason: 'found_while_paused', count, oldest_at: 0, newest_at: 0 }] : [] },
    startup_check: null,
  }
}

/** What `found_while_paused_review` reads for `count` held new files (lead review round 3, P2-2). */
function reviewOf(count: number, patch: Partial<model.FoundWhilePausedReview> = {}): model.FoundWhilePausedReview {
  const op_ids = Array.from({ length: count }, (_, i) => `op-${i + 1}`)
  return { new_folders: 0, new_files: count, edits: 0, deletions: 0, in_shared_folders: 0, op_ids, synced_by: null, ...patch }
}

/** A `found_while_paused_keep` answer. */
function kept(patch: Partial<model.KeepResult>): model.KeepResult {
  return { moved_files: 0, moved_items: 0, kept_path: null, not_moved: 0, stays_held: 0, synced_by: null, ...patch }
}

/**
 * Mount the card. `props.summary` is what the panel's last poll read; a test changes it to stand in for a refresh. The
 * backend reads `count` held new files unless the test scripts the review itself.
 */
function open(count: number, scripted: Record<string, Handler> = {}) {
  const backend: Record<string, Handler> = { found_while_paused_review: () => reviewOf(count), ...scripted }
  const refreshes: SyncActivitySummary[] = []
  const props = { summary: summaryHolding(count), refresh: async () => { refreshes.push(props.summary) } }
  const m = mount('windows/FoundWhilePausedCard.tsx', 'FoundWhilePausedCard', {
    props,
    backend,
    bindings: { ...desktopApi, ...model, CardButton, Card, NavIcon, T, RED: 'var(--red)' },
    expand: true,
  })
  mounted.push(m)
  const view = () => m.elements()
  const text = () => textOf(m.tree() === null ? null : view()[0] ?? null)
  const buttons = () => view().filter((el) => el.type === 'button')
  const labels = () => buttons().map((el) => textOf(el.props.children).trim())
  return { m, props, refreshes, text, buttons, labels, view }
}

const commandsSent = (m: Mounted) => m.calls.map((call) => call.name)

describe('when it shows', () => {
  test('nothing found while paused: no card', async () => {
    const card = open(0)
    await card.m.flush()
    expect(card.m.tree()).toBeNull()
    expect(card.m.calls).toEqual([])
  })

  test('found while paused: the title, what is held by kind, the sentence and the two choices; only the read is sent', async () => {
    const card = open(3, { found_while_paused_review: () => reviewOf(3, { new_files: 1, new_folders: 1, in_shared_folders: 1 }) })
    await card.m.flush()
    expect(card.text()).toContain(COPY.title)
    expect(card.text()).toContain('Waiting for your decision: 1 new folder, 1 new file and 1 change in a folder shared with you.')
    expect(card.text()).toContain(model.NOT_CONFIRMED_LINE)
    expect(card.labels()).toEqual([COPY.upload, COPY.keep])
    expect(card.buttons().every((button) => button.props.disabled === false)).toBe(true)
    expect(card.m.calls).toEqual([{ name: 'found_while_paused_review', args: {} }])
  })

  test('one change: the line is singular', async () => {
    const card = open(1)
    await card.m.flush()
    expect(card.text()).toContain('Waiting for your decision: 1 new file.')
  })

  test('until the changes are read, both choices show and wait, with the count and the sentence', async () => {
    const card = open(3, { found_while_paused_review: () => new Promise(() => undefined) })
    await card.m.flush()
    expect(card.text()).toContain('3 changes waiting for your decision.')
    expect(card.text()).toContain(model.NOT_CONFIRMED_LINE)
    expect(card.labels()).toEqual([COPY.upload, COPY.keep])
    expect(card.buttons().every((button) => button.props.disabled === true)).toBe(true)
  })

  test('a read that fails says so, and the next refresh reads again', async () => {
    let reads = 0
    const card = open(2, {
      found_while_paused_review: () => {
        reads += 1
        if (reads === 1) throw new Error('fixture: the changes could not be read')
        return reviewOf(2)
      },
    })
    await card.m.flush()
    expect(textOf(card.view().find((el) => el.props.role === 'alert'))).toBe('fixture: the changes could not be read')
    expect(card.buttons().every((button) => button.props.disabled === true)).toBe(true)
    card.props.summary = summaryHolding(2)
    card.m.render()
    await card.m.flush()
    expect(reads).toBe(2)
    expect(card.view().some((el) => el.props.role === 'alert')).toBe(false)
    expect(card.text()).toContain('Waiting for your decision: 2 new files.')
    expect(card.buttons().every((button) => button.props.disabled === false)).toBe(true)
  })

  test('a slow read lands although the panel polls meanwhile, and is not asked again while it runs', async () => {
    // Self-review of round 3, F6: each poll's new summary dropped the read in flight and asked again, so a read slower
    // than the poll (the session-transition lock held) never landed and the choices waited for ever.
    let answer: (value: unknown) => void = () => undefined
    let reads = 0
    const card = open(3, {
      found_while_paused_review: () => {
        reads += 1
        return new Promise((resolve) => { answer = resolve })
      },
    })
    await card.m.flush()
    card.props.summary = summaryHolding(3)
    card.m.render()
    await card.m.flush()
    expect(reads).toBe(1)
    answer(reviewOf(3))
    await card.m.flush()
    expect(card.text()).toContain('Waiting for your decision: 3 new files.')
    expect(card.buttons().every((button) => button.props.disabled === false)).toBe(true)
  })

  test('nothing is focused for the person: no element takes the focus on its own', async () => {
    const card = open(3)
    await card.m.flush()
    expect(card.view().some((el) => el.props.autoFocus)).toBe(false)
    await card.m.click(COPY.keep)
    expect(card.view().some((el) => el.props.autoFocus)).toBe(false)
  })
})

describe('Upload to my Beebeeb', () => {
  test('sends back exactly the changes the card counted, then reads the summary again', async () => {
    const card = open(3, { found_while_paused_upload: () => ({ released: 3 }) })
    await card.m.flush()
    await card.m.click(COPY.upload)
    expect(commandsSent(card.m).slice(0, 2)).toEqual(['found_while_paused_review', 'found_while_paused_upload'])
    expect(card.m.calls[1].args).toEqual({ opIds: ['op-1', 'op-2', 'op-3'] })
    expect(card.refreshes.length).toBe(1)
    card.props.summary = summaryHolding(0)
    card.m.render()
    expect(card.m.tree()).toBeNull()
  })

  test('while it runs, every button is disabled', async () => {
    let release: (value: unknown) => void = () => undefined
    const card = open(3, { found_while_paused_upload: () => new Promise((resolve) => { release = resolve }) })
    await card.m.flush()
    await card.m.clickNoWait(COPY.upload)
    expect(card.buttons().length).toBe(2)
    expect(card.buttons().every((button) => button.props.disabled === true)).toBe(true)
    expect(card.buttons().some((button) => button.props['aria-busy'] === true)).toBe(true)
    release({ released: 3 })
    await card.m.flush()
    // What is held is read again before the choices come back (the harness runs that effect at the next flush).
    await card.m.flush()
    expect(commandsSent(card.m)).toEqual(['found_while_paused_review', 'found_while_paused_upload', 'found_while_paused_review'])
    expect(card.buttons().every((button) => button.props.disabled === false)).toBe(true)
  })

  test('a failure shows the command’s own words in the card, as an alert', async () => {
    const card = open(3, { found_while_paused_upload: () => { throw new Error('fixture: the queue could not be written') } })
    await card.m.flush()
    await card.m.click(COPY.upload)
    const alert = card.view().find((el) => el.props.role === 'alert')
    expect(textOf(alert)).toBe('fixture: the queue could not be written')
    expect(card.m.toasts).toEqual([])
    expect(card.labels()).toEqual([COPY.upload, COPY.keep])
  })
})

describe('Keep on this PC only', () => {
  test('asks inline first, and Cancel goes back without sending anything', async () => {
    const card = open(3, { found_while_paused_keep: () => { throw new Error('must not be sent') } })
    await card.m.flush()
    await card.m.click(COPY.keep)
    expect(card.text()).toContain(COPY.confirm)
    expect(card.labels()).toEqual([COPY.move, COPY.cancel])
    expect(card.view().some((el) => el.props.role === 'dialog')).toBe(false)
    expect(commandsSent(card.m)).toEqual(['found_while_paused_review'])
    await card.m.click(COPY.cancel)
    expect(card.text()).not.toContain(COPY.confirm)
    expect(card.labels()).toEqual([COPY.upload, COPY.keep])
    expect(commandsSent(card.m)).toEqual(['found_while_paused_review'])
  })

  test('a kept folder OneDrive syncs: the button and the question say so, never "on this PC"', async () => {
    const card = open(2, { found_while_paused_review: () => reviewOf(2, { synced_by: 'OneDrive' }) })
    await card.m.flush()
    expect(card.labels()).toEqual([COPY.upload, 'Keep out of Beebeeb'])
    await card.m.click('Keep out of Beebeeb')
    expect(card.text()).toContain(model.keepConfirm('OneDrive'))
    expect(card.text()).toContain('This folder is synced by OneDrive.')
  })

  test('"Move them" keeps them, shows where, offers Explorer, and stays until dismissed', async () => {
    const card = open(3, {
      found_while_paused_keep: () => kept({ moved_files: 5, moved_items: 3, kept_path: KEPT, not_moved: 0, stays_held: 0 }),
      show_kept_folder: () => null,
    })
    await card.m.flush()
    await card.m.click(COPY.keep)
    await card.m.click(COPY.move)
    expect(commandsSent(card.m).slice(0, 2)).toEqual(['found_while_paused_review', 'found_while_paused_keep'])
    // Self-review of round 3, F7: like Upload, Keep acts on exactly the changes the card counted.
    expect(card.m.calls[1].args).toEqual({ opIds: ['op-1', 'op-2', 'op-3'] })
    expect(card.refreshes.length).toBe(1)

    // The poll now reads no held group: the result stays.
    card.props.summary = summaryHolding(0)
    card.m.render()
    expect(card.text()).toContain(`Moved 5 files to ${KEPT}. They stay on this PC and aren’t uploaded.`)
    expect(card.text()).not.toContain(COPY.title)
    const path = card.view().find((el) => textOf(el) === KEPT && el.type === 'span')
    expect(path?.props.style.fontFamily).toBe(T.fontMono)
    expect(path?.props.style.overflowWrap).toBe('anywhere')
    expect(card.labels()).toEqual([COPY.showInExplorer, COPY.dismiss])

    await card.m.click(COPY.showInExplorer)
    expect(commandsSent(card.m).filter((name) => name !== 'found_while_paused_review')).toEqual(['found_while_paused_keep', 'show_kept_folder'])
    expect(card.m.calls.find((call) => call.name === 'show_kept_folder')?.args).toEqual({})
    expect(card.text()).toContain(`Moved 5 files to ${KEPT}.`)

    await card.m.click(COPY.dismiss)
    expect(card.m.tree()).toBeNull()
  })

  test('what could not be moved or kept is said, with no Explorer button when nothing moved, and the choice stays for what is left', async () => {
    const card = open(4, { found_while_paused_keep: () => kept({ moved_files: 0, moved_items: 0, kept_path: null, not_moved: 1, stays_held: 3 }) })
    await card.m.flush()
    await card.m.click(COPY.keep)
    await card.m.click(COPY.move)
    expect(card.text()).toContain('1 item couldn’t be moved and stays where it is. It isn’t uploaded.')
    expect(card.text()).toContain('3 changes can’t be kept on this PC only. They aren’t uploaded and stay on hold until you choose “Upload to my Beebeeb”.')
    expect(card.labels()).toEqual([COPY.upload, COPY.keep, COPY.dismiss])
  })

  test('a failed move shows the command’s own words and moves nothing on screen', async () => {
    const card = open(2, { found_while_paused_keep: () => { throw new Error('fixture: the kept folder could not be created') } })
    await card.m.flush()
    await card.m.click(COPY.keep)
    await card.m.click(COPY.move)
    const alert = card.view().find((el) => el.props.role === 'alert')
    expect(textOf(alert)).toBe('fixture: the kept folder could not be created')
    expect(card.labels()).toEqual([COPY.upload, COPY.keep])
    expect(card.text()).not.toContain('Moved')
  })

  test('an answer outside the contract’s shape is an error, never a made-up count', async () => {
    const card = open(2, { found_while_paused_keep: () => ({ moved: 2 }) })
    await card.m.flush()
    await card.m.click(COPY.keep)
    await card.m.click(COPY.move)
    expect(textOf(card.view().find((el) => el.props.role === 'alert'))).toBe('The answer had an unexpected shape.')
    expect(card.text()).not.toContain('Moved')
  })

  test('while the move runs, every button is disabled and a poll that drops the group does not hide the card', async () => {
    let release: (value: unknown) => void = () => undefined
    const card = open(2, { found_while_paused_keep: () => new Promise((resolve) => { release = resolve }) })
    await card.m.flush()
    await card.m.click(COPY.keep)
    await card.m.clickNoWait(COPY.move)
    expect(card.buttons().every((button) => button.props.disabled === true)).toBe(true)
    card.props.summary = summaryHolding(0)
    card.m.render()
    expect(card.m.tree()).not.toBeNull()
    release(kept({ moved_files: 2, moved_items: 2, kept_path: KEPT, not_moved: 0, stays_held: 0 }))
    await card.m.flush()
    expect(card.text()).toContain(`Moved 2 files to ${KEPT}. They stay on this PC and aren’t uploaded.`)
  })
})

describe('the brand', () => {
  test('one amber button per step, the primary one, and no emoji', async () => {
    const card = open(3, { found_while_paused_keep: () => kept({ moved_files: 1, moved_items: 1, kept_path: KEPT, not_moved: 0, stays_held: 0 }), show_kept_folder: () => null })
    await card.m.flush()
    const amber = () => card.buttons().filter((button) => String(button.props.style.background).includes('amber')).map((button) => textOf(button.props.children).trim())
    expect(amber()).toEqual([COPY.upload])
    await card.m.click(COPY.keep)
    expect(amber()).toEqual([COPY.move])
    await card.m.click(COPY.move)
    card.props.summary = summaryHolding(0)
    card.m.render()
    expect(card.labels()).toEqual([COPY.showInExplorer, COPY.dismiss])
    expect(amber()).toEqual([])
    expect(card.text()).not.toMatch(/\p{Extended_Pictographic}/u)
  })

  test('the amber button’s label uses the on-amber ink, readable in the dark theme too', async () => {
    const card = open(3)
    await card.m.flush()
    const [primary, secondary] = card.buttons()
    expect(primary.props.style.background).toBe(T.amber)
    expect(primary.props.style.color).toBe('var(--amber-ink)')
    expect(secondary.props.style.color).toBe(T.ink)
  })
})

