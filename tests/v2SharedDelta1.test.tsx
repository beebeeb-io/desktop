/**
 * Desktop #265 delta 1 (v2 Shared, B05e): the words the window shows for the answers Rust gives since the
 * review, and the person's safety-number state when this PC holds a comparison it cannot read.
 * - `failureText`: an ended session, a rate limit (the permanent delete's 401/403/429) and an approval that did
 *   not go through (`not-shared`) each get their own words, never "Something went wrong".
 * - `PersonDialog` (DW-0512): `markUnreadable` reads "Couldn’t be read", never "Not compared".
 */
import { describe, expect, test } from 'bun:test'
import { createElement, Fragment } from 'react'
import { failureText } from '../src/v2/sections/shared/api'
import * as model from '../src/v2/sections/shared/model'
import { expand, loadComponent, textOf } from './fixtures/componentHarness'

const fail = (reason: string) => ({ ok: false as const, reason, unsupported: false })
const GENERIC = 'Something went wrong. Try again.'

describe('Shared failure words (delta 1)', () => {
  test('each new reason has its own words', () => {
    const cases: [string, string][] = [
      ['signed-out', 'You’re signed out. Sign in again, then try again.'],
      ['rate_limited', 'Too many attempts. Wait a minute, then try again.'],
      ['not-shared: the approval did not go through', 'Couldn’t finish sharing. Nothing was shared. Try again.'],
    ]
    let checked = 0
    for (const [reason, words] of cases) {
      expect(failureText(fail(reason) as never)).toBe(words)
      checked += 1
    }
    expect(checked).toBe(3)
    expect(failureText(fail('failed') as never)).toBe(GENERIC)
  })
})

describe('PersonDialog safety-number state (delta 1)', () => {
  const Modal = (p: any) => createElement('div', { role: 'dialog' }, p.title, p.children, p.footer)
  const Icon = () => null
  const Segmented = () => null
  const PersonDialog = loadComponent('v2/sections/shared/Dialogs.tsx', 'PersonDialog', {
    React: { createElement, Fragment }, Modal, Icon, Segmented, BTN: '', RED_TEXT: '', FOCUS: '', safetyState: (model as any).safetyState,
  })
  const shown = (member: Record<string, unknown>) =>
    textOf(expand(PersonDialog({ member: { id: 'm', name: 'Jonas Meier', email: 'jonas@northwind.studio', ...member } })))

  test('a comparison this PC cannot read is said', () => {
    const text = shown({ markUnreadable: true })
    expect(text).toContain('Couldn’t be read')
    expect(text).not.toContain('Not compared')
  })

  test('the other states keep their words', () => {
    expect(shown({})).toContain('Not compared')
    expect(shown({ compared: true })).toContain('Compared')
    expect(shown({ mismatch: true })).toContain('Didn’t match')
  })
})
