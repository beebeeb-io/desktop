/**
 * Task 2409: Activity › "Not sent" (`src/windows/views/NotSentSection.tsx`), the REAL component declaration executed
 * with the component harness against a scripted backend: one row per change the count counts, each with its reason
 * and its one action; Try again on a failed change; the locked and empty states; "Show more".
 * What this does not prove: layout or WebView2 (`tests/render-not-sent.mjs` renders it in a browser).
 */
import { afterEach, describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { createElement, Fragment } from 'react'
import * as desktopApi from '../src/desktopApi'
import * as unsent from '../src/windows/unsentChanges'
import { elementsOf, loadComponent, mount, textOf, type Mounted } from './fixtures/componentHarness'

const fixture = JSON.parse(readFileSync(new URL('./fixtures/unsent-changes.page.json', import.meta.url), 'utf8'))
const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close() })

const T = new Proxy({}, { get: (_t, key) => `var(--${String(key)})` })
const shared = {
  React: { createElement, Fragment },
  T,
  WARN_COLOR: 'var(--warn-text)',
  actionButton: {},
  FileGlyph: 'glyph',
  NavIcon: 'icon',
  FILE_COLOR: new Proxy({}, { get: () => '' }),
  getFileType: () => 'default',
  ...unsent,
}
const NotSentRow = loadComponent('windows/views/NotSentSection.tsx', 'NotSentRow', shared)
const Line = loadComponent('windows/views/NotSentSection.tsx', 'Line', shared)

function open(backend: Record<string, (args: any) => unknown>, props: Record<string, unknown> = {}) {
  const m = mount('windows/views/NotSentSection.tsx', 'NotSentSection', {
    backend,
    props,
    expand: true,
    bindings: {
      ...shared,
      NotSentRow,
      Line,
      Card: 'card',
      command: desktopApi.command,
      commandUnavailableLabel: desktopApi.commandUnavailableLabel,
      forceReauth: async () => ({ ok: true, value: { warning: null } }),
      openUrl: async (url: string) => { opened.push(url); return { ok: true, value: undefined } },
      pollWhileVisible: (tick: () => void) => { void tick(); return () => undefined },
      NOT_SENT_POLL_MS: 5000,
      NOT_SENT_ANCHOR: 'not-sent',
    },
  })
  mounted.push(m)
  return m
}
const opened: string[] = []
const settle = async (m: Mounted) => { for (let i = 0; i < 4; i++) await m.flush() }

/** Each row as a person reads it: its text, by the change's id. */
function rows(m: Mounted): Record<string, string> {
  const out: Record<string, string> = {}
  for (const el of m.elements()) if (el.props['data-unsent-id']) out[el.props['data-unsent-id']] = textOf(el.props.children)
  return out
}
const buttons = (m: Mounted) => m.elements().filter((el) => el.type === 'button').map((el) => textOf(el.props.children))
const links = (m: Mounted) => m.elements().filter((el) => el.type === 'a').map((el) => [textOf(el.props.children), el.props.href])

describe('NotSentSection', () => {
  test('one row per change the count counts, each with what it is, where, why, and its one action', async () => {
    const m = open({ unsent_changes: () => fixture })
    await settle(m)
    const listed = rows(m)
    expect(Object.keys(listed)).toEqual(['op-delete', 'op-edit', 'op-new'])
    expect(listed['op-delete']).toContain('old plan.docx')
    expect(listed['op-delete']).toContain('Delete')
    expect(listed['op-delete']).toContain('Projects/2026')
    expect(listed['op-delete']).toContain('Stopped after repeated errors')
    expect(listed['op-edit']).toContain('budget.xlsx')
    expect(listed['op-edit']).toContain('On hold: your storage is full')
    expect(listed['op-new']).toContain('IMG_0412.jpg')
    expect(listed['op-new']).toContain('Sending · 34%')
    expect(buttons(m)).toEqual(['Try again'])
    expect(links(m)).toEqual([['Add storage', desktopApi.BILLING_URL]])
    expect(textOf(m.tree())).toContain('Not sent')
    expect(m.calls.filter((c) => c.name === 'unsent_changes').map((c) => c.args)).toEqual([{ offset: 0, limit: 100 }])
  })

  test('Try again sends the id, and the change then reads "Queued"', async () => {
    let retried = false
    const m = open({
      unsent_changes: () =>
        retried
          ? { ...fixture, engine_state: 'idle', items: fixture.items.map((i: any) => (i.id === 'op-delete' ? { ...i, group: 'waiting', retryable: false } : i)) }
          : fixture,
      retry_unsent_change: (args: { id: string }) => { retried = true; return 'queued' },
    })
    await settle(m)
    await m.click('Try again')
    await settle(m)
    expect(m.calls.find((c) => c.name === 'retry_unsent_change')?.args).toEqual({ id: 'op-delete' })
    expect(rows(m)['op-delete']).toContain('Queued')
    expect(buttons(m)).not.toContain('Try again')
    expect(m.toasts).toEqual([])
  })

  test('a refused Try again says why in a toast, and the row stays as the next read says', async () => {
    const m = open({
      unsent_changes: () => fixture,
      retry_unsent_change: () => { throw 'unknown' },
    })
    await settle(m)
    await m.click('Try again')
    await settle(m)
    expect(m.toasts.map((t) => [t.variant, t.title, t.message])).toEqual([
      ['error', 'Couldn’t try it again', 'It isn’t waiting any more: it was sent or removed in the meantime.'],
    ])
    expect(rows(m)['op-delete']).toContain('Stopped after repeated errors')
  })

  test('Add storage opens billing in the browser; Decide in Settings › Sync opens Settings here', async () => {
    let settings = 0
    const found = { ...fixture.items[1], id: 'op-found', group: 'found_while_paused' }
    const m = open({ unsent_changes: () => ({ ...fixture, total: 2, items: [fixture.items[1], found] }) }, { onOpenSyncSettings: () => { settings += 1 } })
    await settle(m)
    const link = (label: string) => m.elements().find((el) => el.type === 'a' && textOf(el.props.children) === label)!
    link('Add storage').props.onClick({ preventDefault() {} })
    link('Decide in Settings › Sync').props.onClick({ preventDefault() {} })
    await settle(m)
    expect(opened).toContain(desktopApi.BILLING_URL)
    expect(settings).toBe(1)
  })

  test('locked: no names, one Unlock; nothing sent: it says so', async () => {
    const locked = open({ unsent_changes: () => { throw 'vault is locked' } })
    await settle(locked)
    expect(rows(locked)).toEqual({})
    expect(buttons(locked)).toEqual(['Unlock'])
    expect(textOf(locked.tree())).toContain('Beebeeb is locked. Unlock it to see which changes these are.')
    // On Windows the session gate answers first, with its own words (`session_commands.rs`): still locked (self-review).
    const gated = open({ unsent_changes: () => { throw 'vault is locked or stopping' } })
    await settle(gated)
    expect(buttons(gated)).toEqual(['Unlock'])
    expect(textOf(gated.tree())).not.toContain('Couldn’t read')
    const empty = open({ unsent_changes: () => ({ ...fixture, total: 0, items: [] }) })
    await settle(empty)
    expect(textOf(empty.tree())).toContain('Every change on this PC has been sent.')
  })

  test('"Show more" shows the next 100, read in one call with the rows already shown', async () => {
    const page = ({ offset, limit }: { offset: number; limit: number }) => ({
      ...fixture,
      total: 150,
      items: Array.from({ length: Math.max(0, Math.min(limit, 150 - offset)) }, (_, i) => ({ ...fixture.items[2], id: `op-${offset + i}`, sending: null })),
    })
    const m = open({ unsent_changes: page })
    await settle(m)
    expect(Object.keys(rows(m))).toHaveLength(100)
    expect(textOf(m.tree())).toContain('100 of 150')
    await m.click('Show more')
    await settle(m)
    expect(Object.keys(rows(m))).toHaveLength(150)
    expect(buttons(m)).not.toContain('Show more')
    expect(m.calls.filter((c) => c.name === 'unsent_changes').map((c) => c.args)).toContainEqual({ offset: 0, limit: 200 })
  })
})

test('every element the harness reads is real output (sanity: the tree is not empty)', async () => {
  const m = open({ unsent_changes: () => fixture })
  await settle(m)
  expect(elementsOf(m.tree()).length).toBeGreaterThan(10)
})
