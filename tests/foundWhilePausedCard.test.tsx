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

const KEPT = 'C:\\Users\\Fixture\\Beebeeb - kept on this PC\\2026-10-28'
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

/** Mount the card. `props.summary` is what the panel's last poll read; a test changes it to stand in for a refresh. */
function open(count: number, backend: Record<string, Handler> = {}) {
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

  test('found while paused: the title, the count and the two choices, and nothing sent on its own', async () => {
    const card = open(3)
    await card.m.flush()
    expect(card.text()).toContain(COPY.title)
    expect(card.text()).toContain('3 changes waiting for your decision.')
    expect(card.labels()).toEqual([COPY.upload, COPY.keep])
    expect(card.m.calls).toEqual([])
  })

  test('one change: the count line is singular', async () => {
    const card = open(1)
    await card.m.flush()
    expect(card.text()).toContain('1 change waiting for your decision.')
  })
})

describe('Upload to my Beebeeb', () => {
  test('sends the upload command with no arguments, then reads the summary again', async () => {
    const card = open(3, { found_while_paused_upload: () => ({ released: 3 }) })
    await card.m.flush()
    await card.m.click(COPY.upload)
    expect(commandsSent(card.m)).toEqual(['found_while_paused_upload'])
    expect(card.m.calls[0].args).toEqual({})
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
    expect(card.m.calls).toEqual([])
    await card.m.click(COPY.cancel)
    expect(card.text()).not.toContain(COPY.confirm)
    expect(card.labels()).toEqual([COPY.upload, COPY.keep])
    expect(card.m.calls).toEqual([])
  })

  test('"Move them" keeps them, shows where, offers Explorer, and stays until dismissed', async () => {
    const card = open(3, {
      found_while_paused_keep: () => ({ moved_files: 5, moved_items: 3, kept_path: KEPT, not_moved: 0, stays_held: 0 }),
      show_kept_folder: () => null,
    })
    await card.m.flush()
    await card.m.click(COPY.keep)
    await card.m.click(COPY.move)
    expect(commandsSent(card.m)).toEqual(['found_while_paused_keep'])
    expect(card.m.calls[0].args).toEqual({})
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
    expect(commandsSent(card.m)).toEqual(['found_while_paused_keep', 'show_kept_folder'])
    expect(card.m.calls[1].args).toEqual({})
    expect(card.text()).toContain(`Moved 5 files to ${KEPT}.`)

    await card.m.click(COPY.dismiss)
    expect(card.m.tree()).toBeNull()
  })

  test('what could not be moved or kept is said, with no Explorer button when nothing moved, and the choice stays for what is left', async () => {
    const card = open(4, { found_while_paused_keep: () => ({ moved_files: 0, moved_items: 0, kept_path: null, not_moved: 1, stays_held: 3 }) })
    await card.m.flush()
    await card.m.click(COPY.keep)
    await card.m.click(COPY.move)
    expect(card.text()).toContain('1 item couldn’t be moved and stays where it is. It isn’t uploaded.')
    expect(card.text()).toContain('3 changes to items already in Beebeeb stay on hold. Contact support@beebeeb.io.')
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
    release({ moved_files: 2, moved_items: 2, kept_path: KEPT, not_moved: 0, stays_held: 0 })
    await card.m.flush()
    expect(card.text()).toContain(`Moved 2 files to ${KEPT}. They stay on this PC and aren’t uploaded.`)
  })
})

describe('the brand', () => {
  test('one amber button per step, the primary one, and no emoji', async () => {
    const card = open(3, { found_while_paused_keep: () => ({ moved_files: 1, moved_items: 1, kept_path: KEPT, not_moved: 0, stays_held: 0 }), show_kept_folder: () => null })
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

