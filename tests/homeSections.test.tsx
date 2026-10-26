/**
 * Home's three additions as the window draws them (task 1683 slice 4, the Home add-on; spec §7.6 Home, hi-fi section 3
 * "Home"): the counts line inside the status card, "Needs your attention" with Review's own rows, and "Recent activity".
 * The real `HomeCounts`, `HomeAttention`, `HomeActivity` and `ReviewEntryRow` declarations, read as the text, roles and
 * handlers a person gets. What this proves: the words, the order, what a press hands back, and that an empty section
 * draws nothing (no placeholder). What it does NOT prove: layout (tests/render-mac-app.mjs), the data (tests/homeModel.test.ts).
 */
import { describe, expect, test } from 'bun:test'
import { createElement, Fragment } from 'react'
import * as parts from '../src/macSettingsParts'
import * as reviewModel from '../src/reviewModel'
import { expand, elementsOf, loadComponent, textOf, type TreeNode } from './fixtures/componentHarness'

const React = { createElement, Fragment }
const ReviewEntryRow = loadComponent('ReviewRow.tsx', 'ReviewEntryRow', { React, Btn: parts.Btn, ...reviewModel })
const HomeCounts = loadComponent('HomeSections.tsx', 'HomeCounts', { React })
const HomeAttention = loadComponent('HomeSections.tsx', 'HomeAttention', { React, SettingsGroup: parts.SettingsGroup, Btn: parts.Btn, ReviewEntryRow })
const HomeActivity = loadComponent('HomeSections.tsx', 'HomeActivity', { React, SettingsGroup: parts.SettingsGroup, SettingsIcon: parts.SettingsIcon })

const draw = (component: unknown, props: Record<string, unknown>) => expand(createElement(component as never, props))
const els = (tree: any) => elementsOf(tree)
const texts = (tree: any): string[] => {
  const out: string[] = []
  const walk = (n: any) => {
    if (Array.isArray(n)) return n.forEach(walk)
    if (n == null || typeof n === 'boolean') return
    if (typeof n !== 'object' || !n.props) { out.push(String(n)); return }
    if (n.type === 'svg') return
    walk(n.props.children)
  }
  walk(tree)
  return out.map((s) => s.trim()).filter(Boolean)
}
const buttonsOf = (tree: any) => els(tree).filter((el) => el.type === 'button')
const label = (el: TreeNode) => el.props['aria-label'] ?? textOf(expand(el.props.children)).trim()
const visible = (el: TreeNode) => textOf(expand(el.props.children)).trim()

const now = new Date(2026, 9, 25, 14, 30)
const at = (h: number, mi: number) => Math.floor(new Date(2026, 9, 25, h, mi).getTime() / 1000)
const conflict = { id: 'conflict:f1', file_id: 'f1', file_name: 'Board notes, 29 Sept.md', folder: 'Team shared', kind: 'conflict', status: 'needs review', updated_at: at(9, 12), detail: 'Changed on two devices.', action: 'open_conflict' }
const upload = { id: 'op:2', file_id: 'f2', file_name: 'Site visit.mov', folder: 'Photos', kind: 'failed_upload', status: 'upload review', updated_at: at(8, 47), detail: 'Couldn’t upload, will retry', action: 'review_upload' }
const quota = { ...upload, id: 'op:3', file_id: 'f3', file_name: 'Archive.zip', folder: null, kind: 'quota_failure' }

describe('ReviewEntryRow: one row of Review, drawn the same on Review and on Home', () => {
  test('tag, name and folder · when · detail, and the one action', () => {
    const tree = draw(ReviewEntryRow, { entry: conflict, now, onAction: () => {} })
    expect(texts(tree)).toEqual(['Conflict', 'Board notes, 29 Sept.md', 'Team shared · today 09:12 · Changed on two devices.', 'Decide…'])
    expect(els(tree)[0].props['data-row']).toBe('review')
  })

  test('a kind with no action has no button', () => {
    expect(buttonsOf(draw(ReviewEntryRow, { entry: upload, now, onAction: () => {} }))).toHaveLength(0)
  })

  test('pressing the action hands back the entry and the action, and nothing else', () => {
    const got: unknown[] = []
    const tree = draw(ReviewEntryRow, { entry: quota, now, onAction: (...args: unknown[]) => got.push(args) })
    buttonsOf(tree)[0].props.onClick()
    expect(got).toEqual([[quota, { kind: 'upgrade', label: 'Add storage' }]])
  })

  test('the action is never the amber primary: neither Review nor Home has one primary action among its rows', () => {
    const tree = draw(ReviewEntryRow, { entry: conflict, now, onAction: () => {} })
    expect(String(buttonsOf(tree)[0].props.className)).toBe('ms-btn')
  })
})

describe('HomeCounts: inside the status card, a label and a bold value each', () => {
  test('three parts in order', () => {
    const tree = draw(HomeCounts, { parts: [{ label: 'On this Mac', value: '1,204 files' }, { label: 'Online-only', value: '8,311' }, { label: 'Kept on this Mac', value: '3 folders' }] })
    expect(texts(tree)).toEqual(['On this Mac', '1,204 files', 'Online-only', '8,311', 'Kept on this Mac', '3 folders'])
    expect(els(tree).filter((el) => el.type === 'b').map((el) => textOf(el.props.children))).toEqual(['1,204 files', '8,311', '3 folders'])
    expect(els(tree)[0].props['data-row']).toBe('counts')
  })

  test('nothing to say draws nothing: no placeholder row', () => {
    expect(draw(HomeCounts, { parts: [] })).toBeNull()
    expect(draw(HomeCounts, { parts: null })).toBeNull()
  })
})

describe('HomeAttention: "Needs your attention"', () => {
  const base = { entries: [conflict, upload], more: 0, now, onAction: () => {}, onOpenReview: () => {} }

  test('a titled group with Review\'s rows, then whatever the caller adds (the kept rows)', () => {
    const tree = draw(HomeAttention, { ...base, children: createElement('div', { 'data-kept': '' }, 'Files kept on this Mac') })
    expect(els(tree)[0].props['aria-label']).toBe('Needs your attention')
    expect(texts(tree)).toEqual([
      'Needs your attention',
      'Conflict', 'Board notes, 29 Sept.md', 'Team shared · today 09:12 · Changed on two devices.', 'Decide…',
      'Upload', 'Site visit.mov', 'Photos · today 08:47 · Couldn’t upload, will retry',
      'Files kept on this Mac',
    ])
  })

  test('more than three exist: one quiet button says how many more are in Review, and opens Review', () => {
    let opened = 0
    const tree = draw(HomeAttention, { ...base, more: 2, onOpenReview: () => { opened += 1 } })
    const more = buttonsOf(tree).filter((b) => label(b) === '2 more in Review')
    expect(more).toHaveLength(1)
    more[0].props.onClick()
    expect(opened).toBe(1)
    expect(draw(HomeAttention, { ...base, more: 1 }) && texts(draw(HomeAttention, { ...base, more: 1 }))).toContain('1 more in Review')
    expect(texts(draw(HomeAttention, base))).not.toContain('0 more in Review')
  })

  test('an action on a row is handed up with its entry', () => {
    const got: unknown[] = []
    const tree = draw(HomeAttention, { ...base, onAction: (...args: unknown[]) => got.push(args) })
    buttonsOf(tree).filter((b) => visible(b) === 'Decide…')[0].props.onClick()
    expect(got).toEqual([[conflict, { kind: 'open_conflict', label: 'Decide…' }]])
  })

  test('"Decide…" is named for its file here too (the row is Review\'s own), the visible word first', () => {
    const tree = draw(HomeAttention, base)
    const decide = buttonsOf(tree).filter((b) => visible(b) === 'Decide…')[0]
    expect(label(decide)).toBe('Decide…, Board notes, 29 Sept.md')
  })

  test('no entries and nothing added: no group at all', () => {
    expect(draw(HomeAttention, { ...base, entries: [], more: 0 })).toBeNull()
  })

  test('the kept rows alone still draw the group (they are the other half of "Needs your attention", §8.3)', () => {
    const tree = draw(HomeAttention, { ...base, entries: [], children: createElement('div', null, 'kept') })
    expect(texts(tree)).toEqual(['Needs your attention', 'kept'])
  })
})

describe('HomeActivity: "Recent activity", the popover\'s rows', () => {
  const rows = [
    { id: 'a1', name: 'Q3 ledger.xlsx', icon: 'up', line: 'Uploaded · Investigations · 2 minutes ago', label: 'Q3 ledger.xlsx, uploaded, Investigations, 2 minutes ago', path: 'Investigations/Q3 ledger.xlsx', tone: 'neutral' },
    { id: 'a2', name: 'Plan.pdf', icon: 'down', line: 'Downloading · 62 % · Board · just now', label: 'Plan.pdf, downloading', path: 'Board/Plan.pdf', tone: 'busy' },
    { id: 'a3', name: 'Broken.mov', icon: 'alert', line: 'Couldn’t sync · Photos · 1 hour ago', label: 'Broken.mov, couldn’t sync', path: 'Photos/Broken.mov', tone: 'alert' },
  ]

  test('a titled list: name and the line, one list item per row, each readable as one sentence, the path as its tooltip', () => {
    const tree = draw(HomeActivity, { rows })
    expect(els(tree)[0].props['aria-label']).toBe('Recent activity')
    expect(texts(tree)).toEqual([
      'Recent activity',
      'Q3 ledger.xlsx', 'Uploaded · Investigations · 2 minutes ago',
      'Plan.pdf', 'Downloading · 62 % · Board · just now',
      'Broken.mov', 'Couldn’t sync · Photos · 1 hour ago',
    ])
    const items = els(tree).filter((el) => el.props.role === 'listitem')
    expect(items.map((el) => el.props['aria-label'])).toEqual(rows.map((r) => r.label))
    expect(items.map((el) => el.props.title)).toEqual(rows.map((r) => r.path))
    expect(els(tree).filter((el) => el.props.role === 'list')).toHaveLength(1)
  })

  test('the direction is an icon with no words of its own (the line has them), and a failure is drawn as an alert', () => {
    const tree = draw(HomeActivity, { rows })
    const icons = els(tree).filter((el) => el.type === 'svg')
    expect(icons).toHaveLength(3)
    const discs = els(tree).filter((el) => String(el.props.className ?? '').includes('mw-status-disc') || String(el.props.className ?? '').includes('mw-tile'))
    expect(discs.map((el) => String(el.props.className))).toEqual(['mw-tile', 'mw-tile', 'mw-tile mw-tile--alert'])
  })

  test('no rows draws nothing', () => {
    expect(draw(HomeActivity, { rows: [] })).toBeNull()
  })
})
