/**
 * Desktop v2 B05e (2440 item 2): a person's comparison this PC cannot read says so and offers
 * Compare again; the row is replaced only by a fresh, completed comparison.
 * - `failureText`: Rust's `mark_unreadable` reason has its own words, never "Something went wrong".
 * - `PersonDialog` (DW-0512): the row reads "Couldn't be read · Compare again" and says what to do.
 * - `afterMarkCompared`: only a successful comparison replaces the row; a failed answer changes nothing.
 */
import { describe, expect, test } from 'bun:test'
import { createElement, Fragment } from 'react'
import { failureText } from '../src/v2/sections/shared/api'
import * as model from '../src/v2/sections/shared/model'
import { elementsOf, expand, loadComponent, textOf } from './fixtures/componentHarness'

const fail = (reason: string) => ({ ok: false as const, reason, unsupported: false })
const GENERIC = 'Something went wrong. Try again.'
const MARK_UNREADABLE = 'Your earlier comparison with this person can’t be read. Compare again to share.'

describe('a comparison this PC cannot read', () => {
  test('mark_unreadable_says_what_to_do', () => {
    expect(failureText(fail('mark_unreadable') as never)).toBe(MARK_UNREADABLE)
    expect(failureText(fail('mark_unreadable') as never)).not.toBe(GENERIC)
    expect(failureText(fail('failed') as never)).toBe(GENERIC)
  })

  const Modal = (p: any) => createElement('div', { role: 'dialog' }, p.title, p.children, p.footer)
  const Icon = () => null
  const Segmented = () => null
  const PersonDialog = loadComponent('v2/sections/shared/Dialogs.tsx', 'PersonDialog', {
    React: { createElement, Fragment }, Modal, Icon, Segmented, BTN: '', RED_TEXT: '', FOCUS: '',
    safetyState: (model as any).safetyState, MARK_UNREADABLE,
  })
  const tree = (member: Record<string, unknown>) =>
    expand(PersonDialog({ member: { id: 'm-jonas', name: 'Jonas Meier', email: 'jonas@northwind.studio', ...member } }))
  const shown = (member: Record<string, unknown>) => textOf(tree(member))

  test('the_unreadable_row_offers_compare_again', () => {
    const unreadable = tree({ markUnreadable: true })
    expect(shown({ markUnreadable: true })).toContain('Couldn’t be read')
    expect(shown({ markUnreadable: true })).toContain('Compare again')
    expect(elementsOf(unreadable).filter((el) => el.props['data-compare-again'] != null)).toHaveLength(1)
    expect(shown({ markUnreadable: true })).toContain(MARK_UNREADABLE)

    const plain = tree({})
    expect(shown({})).toContain('Not compared')
    expect(shown({})).not.toContain('Compare again')
    expect(elementsOf(plain).filter((el) => el.props['data-compare-again'] != null)).toHaveLength(0)
    expect(shown({})).toContain('Optional. Comparing proves your files go to the real Jonas.')
  })

  const member = { id: 'm-jonas', name: 'Jonas Meier', email: 'jonas@northwind.studio', access: 'Can edit' as const, markUnreadable: true }

  test('a_fresh_comparison_replaces_the_row', () => {
    const matched = model.afterMarkCompared(member, { ok: true }, true)
    expect(model.safetyState(matched)).toBe('Compared')
    expect(matched.markUnreadable).toBe(false)

    const mismatched = model.afterMarkCompared(member, { ok: true }, false)
    expect(model.safetyState(mismatched)).toBe('Didn’t match')
    expect(mismatched.markUnreadable).toBe(false)
  })

  test('nothing_else_clears_the_row', () => {
    const failed = model.afterMarkCompared(member, { ok: false, reason: 'failed' }, true)
    expect(failed.markUnreadable).toBe(true)
    expect(model.safetyState(failed)).toBe('Couldn’t be read')

    const unreadable = model.afterMarkCompared(member, { ok: false, reason: 'mark_unreadable' }, false)
    expect(unreadable.markUnreadable).toBe(true)
    expect(model.safetyState(unreadable)).toBe('Couldn’t be read')
  })
})
