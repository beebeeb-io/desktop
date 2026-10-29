/**
 * The harness's opt-in honest re-render (PR #147 review, P3-a): a `flush` re-renders only when a state setter changed a value,
 * as React does. By default it always re-renders, which hid a handler that forgot to set state. Pinned here on a component with
 * one state that settles and then stays still (`MenuBarHint`).
 */
import { afterEach, describe, expect, test } from 'bun:test'
import { createElement, Fragment } from 'react'
import * as desktopApi from '../src/desktopApi'
import * as hintModel from '../src/menuBarHintModel'
import * as parts from '../src/macSettingsParts'
import { mount, type Mounted } from './fixtures/componentHarness'

const mounted: Mounted[] = []
afterEach(() => {
  while (mounted.length) mounted.pop()!.close()
  hintModel.forgetHintDismissedThisRun()
})

function open(honestRender: boolean) {
  const m = mount('MenuBarHint.tsx', 'MenuBarHint', {
    backend: { get_menu_bar_hint_seen: () => false } as any,
    expand: true,
    honestRender,
    bindings: { React: { createElement, Fragment }, ...desktopApi, ...hintModel, ...parts },
  })
  mounted.push(m)
  return m
}

describe('the harness renders as React does when asked to', () => {
  test('honest: a flush that changed no state does not render again', async () => {
    const m = open(true)
    await m.flush()
    const settled = m.tree()
    await m.flush()
    expect(m.tree()).toBe(settled)
  })

  test('honest: a flush in which a state setter changed a value renders', async () => {
    const m = open(true)
    const first = m.tree()
    await m.flush()
    expect(m.tree()).not.toBe(first)
  })

  test('by default every flush renders, as before', async () => {
    const m = open(false)
    await m.flush()
    const settled = m.tree()
    await m.flush()
    expect(m.tree()).not.toBe(settled)
  })
})
