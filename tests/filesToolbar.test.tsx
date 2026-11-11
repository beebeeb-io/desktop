// The Files toolbar's search control is a labelled field, not a button (DW-1101, DW-1105).
// The field opens Search on Enter only; the toolbar keeps Sort, as List and as Grid and has no Filter.
import { expect, test } from 'bun:test'
import { createElement, Fragment } from 'react'
import { Icon } from '@beebeeb/shared/design'
import { elementsOf, expand, loadComponent, textOf, type TreeNode } from './fixtures/componentHarness'
import { IconButton, BTN, BTN_AMBER, FOCUS } from '../src/v2/sections/files/ui'
import { fmtCount } from '../src/v2/sections/files/model'

// `Crumbs` lives in the same file but is not part of the extracted declaration; the toolbar's own
// controls are what these tests read, so a stub keeps the tree small.
const Toolbar = loadComponent('v2/sections/files/FilesPane.tsx', 'Toolbar', {
  React: { createElement, Fragment },
  Icon,
  IconButton,
  BTN,
  BTN_AMBER,
  FOCUS,
  fmtCount,
  Crumbs: () => null,
})

const props = {
  listing: null,
  items: [],
  prefs: { view: 'list', sort: { key: 'name', dir: 'asc', foldersFirst: true }, panel: false },
  selection: new Set<string>(),
  title: 'Files',
  sortPressed: false,
  canNew: true,
}

/** The accessible name: `aria-label` when present, else the normalised text. Exact, never a substring. */
function accessibleName(node: TreeNode): string {
  const label = node.props['aria-label']
  if (label != null) return String(label).replace(/\s+/g, ' ').trim()
  return textOf(node).replace(/\s+/g, ' ').trim()
}

function buttons(tree: TreeNode): TreeNode[] {
  return elementsOf(tree).filter((node) => node.type === 'button')
}

test('the toolbar draws one labelled search field and no Search names button', () => {
  const tree = expand(Toolbar({ p: props, h: {} }))
  const fields = elementsOf(tree).filter((node) => node.type === 'input'
    && node.props['aria-label'] === 'Search names'
    && node.props.placeholder === 'Search names')
  expect(fields.length).toBe(1)
  expect(textOf(tree).replace(/\s+/g, ' ').trim()).toContain('Ctrl+F')
  expect(buttons(tree).filter((node) => accessibleName(node) === 'Search names').length).toBe(0)
})

test('Enter in the field calls onFind once and is prevented; other keys do not', () => {
  let finds = 0
  const tree = expand(Toolbar({ p: props, h: { onFind: () => { finds += 1 } } }))
  const input = elementsOf(tree).find((node) => node.type === 'input' && node.props['aria-label'] === 'Search names')
  expect(input).toBeDefined()
  if (!input) return
  const enter = { key: 'Enter', defaultPrevented: false, preventDefault() { this.defaultPrevented = true } }
  input.props.onKeyDown(enter)
  expect(finds).toBe(1)
  expect(enter.defaultPrevented).toBe(true)
  input.props.onKeyDown({ key: 'a', preventDefault() {} })
  expect(finds).toBe(1)
})

test('the toolbar has no Filter control and keeps Sort, as List and as Grid', () => {
  const tree = expand(Toolbar({ p: props, h: {} }))
  const names = buttons(tree).map(accessibleName)
  expect(names.filter((name) => name === 'Filter').length).toBe(0)
  for (const name of ['Sort', 'as List', 'as Grid']) expect(names.filter((n) => n === name).length).toBe(1)
})
