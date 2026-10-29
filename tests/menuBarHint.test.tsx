/**
 * Home's one-time hint, the real `MenuBarHint` declaration executed with the component harness against a scripted backend
 * (task 1683 slice 6; spec §7.6). What this proves: it reads once, shows only for a Mac that has not dismissed it, "Got it"
 * hides it at once and saves that exactly once, and a failed read or save never puts an error on Home.
 * What it does NOT prove: the card's layout (tests/render-mac-app.mjs), or anything on a Mac.
 */
import { afterEach, describe, expect, test } from 'bun:test'
import { createElement, Fragment } from 'react'
import * as desktopApi from '../src/desktopApi'
import * as hintModel from '../src/menuBarHintModel'
import * as parts from '../src/macSettingsParts'
import { mount, textOf, type Mounted, type TreeNode } from './fixtures/componentHarness'

const mounted: Mounted[] = []
afterEach(() => {
  while (mounted.length) mounted.pop()!.close()
  hintModel.forgetHintDismissedThisRun()
})

function openHint(backend: Record<string, (args: any) => unknown>) {
  const m = mount('MenuBarHint.tsx', 'MenuBarHint', {
    backend: backend as any,
    expand: true,
    bindings: { React: { createElement, Fragment }, ...desktopApi, ...hintModel, ...parts },
  })
  mounted.push(m)
  return m
}
const settle = async (m: Mounted) => { await m.flush(); await m.flush(); await m.flush() }
const text = (m: Mounted) => textOf(m.tree() ? m.elements().map((el) => (typeof el.props.children === 'string' ? el.props.children : '')) : '')
const calls = (m: Mounted, name: string) => m.calls.filter((c) => c.name === name)
const find = (m: Mounted, match: (el: TreeNode) => boolean) => m.elements().filter(match)

describe('Home\'s one-time hint', () => {
  test('a Mac that has not dismissed it shows the card: the title, the sentence and Got it', async () => {
    const m = openHint({ get_menu_bar_hint_seen: () => false })
    await settle(m)
    expect(calls(m, 'get_menu_bar_hint_seen')).toHaveLength(1)
    const t = text(m)
    expect(t).toContain('Beebeeb lives in the menu bar')
    expect(t).toContain('Click the b at the top of your screen to see what’s syncing. You can close this window; Beebeeb keeps running.')
    expect(find(m, (el) => el.type === 'button').map((el) => textOf(el.props.children).trim())).toEqual(['Got it'])
    expect(find(m, (el) => el.props['data-home'] === 'hint')).toHaveLength(1)
  })

  test('a Mac that dismissed it shows nothing, and is never asked to save anything', async () => {
    const m = openHint({ get_menu_bar_hint_seen: () => true })
    await settle(m)
    expect(find(m, (el) => el.props['data-home'] === 'hint')).toHaveLength(0)
    expect(calls(m, 'mark_menu_bar_hint_seen')).toHaveLength(0)
  })

  test('before the Mac has answered there is nothing to see', async () => {
    const m = openHint({ get_menu_bar_hint_seen: () => new Promise(() => {}) })
    await settle(m)
    expect(find(m, (el) => el.props['data-home'] === 'hint')).toHaveLength(0)
  })

  test('a read that failed shows nothing and no error: the next launch asks again', async () => {
    const m = openHint({ get_menu_bar_hint_seen: () => { throw new Error('config unreadable') } })
    await settle(m)
    expect(find(m, (el) => el.props['data-home'] === 'hint')).toHaveLength(0)
    expect(m.toasts).toEqual([])
    expect(text(m)).not.toContain('unreadable')
  })

  test('Got it hides the card at once and saves that exactly once', async () => {
    const m = openHint({ get_menu_bar_hint_seen: () => false, mark_menu_bar_hint_seen: () => undefined })
    await settle(m)
    await m.click('Got it')
    expect(find(m, (el) => el.props['data-home'] === 'hint')).toHaveLength(0)
    expect(calls(m, 'mark_menu_bar_hint_seen')).toHaveLength(1)
    expect(calls(m, 'get_menu_bar_hint_seen')).toHaveLength(1)
  })

  test('a save that failed changes nothing on screen: gone for this run, no toast, no inline error', async () => {
    const m = openHint({ get_menu_bar_hint_seen: () => false, mark_menu_bar_hint_seen: () => { throw new Error('disk full') } })
    await settle(m)
    await m.click('Got it')
    await settle(m)
    expect(find(m, (el) => el.props['data-home'] === 'hint')).toHaveLength(0)
    expect(m.toasts).toEqual([])
    expect(text(m)).not.toContain('disk full')
  })

  // PR #146 review, P3: Home is mounted again on every visit (Review, Trash, Settings and back); a hint whose save failed must
  // not come back within the same run, as the doc above says.
  test('a save that failed keeps the hint gone for the rest of the run: Home mounted again does not show it', async () => {
    const first = openHint({ get_menu_bar_hint_seen: () => false, mark_menu_bar_hint_seen: () => { throw new Error('disk full') } })
    await settle(first)
    await first.click('Got it')
    await settle(first)
    first.unmount()
    const second = openHint({ get_menu_bar_hint_seen: () => false })
    await settle(second)
    expect(find(second, (el) => el.props['data-home'] === 'hint')).toHaveLength(0)
  })

  test('a save that worked is the same: the Mac says seen, and the run says so before it is asked', async () => {
    const first = openHint({ get_menu_bar_hint_seen: () => false, mark_menu_bar_hint_seen: () => undefined })
    await settle(first)
    await first.click('Got it')
    first.unmount()
    const second = openHint({ get_menu_bar_hint_seen: () => false })
    expect(find(second, (el) => el.props['data-home'] === 'hint')).toHaveLength(0)
    await settle(second)
    expect(find(second, (el) => el.props['data-home'] === 'hint')).toHaveLength(0)
  })

  test('the card is a note with its own name, so VoiceOver reads what it says', async () => {
    const m = openHint({ get_menu_bar_hint_seen: () => false })
    await settle(m)
    const card = find(m, (el) => el.props['data-home'] === 'hint')[0]
    expect(card.props.role).toBe('note')
    expect(card.props['aria-label']).toBe('Beebeeb lives in the menu bar')
  })

  test('answers that arrive after the page was left change nothing', async () => {
    let answer!: (value: boolean) => void
    const pending = new Promise<boolean>((resolve) => { answer = resolve })
    const m = openHint({ get_menu_bar_hint_seen: () => pending })
    await m.flush()
    m.unmount()
    answer(false)
    await settle(m)
    expect(find(m, (el) => el.props['data-home'] === 'hint')).toHaveLength(0)
  })
})
