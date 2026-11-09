// P1a: honest status lines. DW-1556 (lead's M3d ruling 4): a failed editor's bar drops "Preparing the editor on this PC…"
// and keeps "Encrypted · read-only for now". #273 finding 12: Office's bar says a save is on this PC until the engine has
// uploaded it. Rendered through the real component with React's server renderer (the text the window shows).
import { describe, expect, test } from 'bun:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { OfficeStatus } from '../src/v2/sections/open/office/Parts'
import { SavedMessage } from '../src/v2/sections/open/Editing'

const text = (html: string) => html.replace(/<[^>]+>/g, '').replace(/\s+/g, ' ').trim()

describe('the editor’s status bar (DW-1507, DW-1556)', () => {
  test('a failed editor says nothing is being prepared: no "Preparing…", still encrypted and read-only', () => {
    const t = text(renderToStaticMarkup(createElement(OfficeStatus, { left: [], failed: true, loading: false, version: 7, time: '' })))
    expect(t).not.toContain('Preparing')
    expect(t).toContain('Encrypted')
    expect(t).toContain('read-only for now')
  })
  test('while the editor is prepared it says so (unchanged)', () => {
    const t = text(renderToStaticMarkup(createElement(OfficeStatus, { left: [], loading: true, version: 7, time: '' })))
    expect(t).toContain('Preparing the editor on this PC…')
  })
  test('a save the engine has not uploaded yet is on this PC; once uploaded it is a version', () => {
    expect(text(renderToStaticMarkup(createElement(OfficeStatus, { left: ['12 words'], uploading: true, version: 7, time: '09:41' })))).toContain('saved on this PC · encrypting… · 09:41')
    expect(text(renderToStaticMarkup(createElement(OfficeStatus, { left: ['12 words'], version: 8, time: '09:42' })))).toContain('saved as version 8 · 09:42')
  })
  test('the note under the toolbar says the same (DW-1524)', () => {
    expect(text(renderToStaticMarkup(createElement(SavedMessage, { note: { kind: 'local' } })))).toContain('Saved on this PC · encrypting…')
    expect(text(renderToStaticMarkup(createElement(SavedMessage, { note: { kind: 'encrypted', version: 4 } })))).toContain('Saved · encrypted · version 4')
    // The board's drawing (version alone) is the encrypted state.
    expect(text(renderToStaticMarkup(createElement(SavedMessage, { version: 4 })))).toContain('Saved · encrypted · version 4')
  })
  test('the Office editor passes the failed and uploading states to its bar', () => {
    const office = require('node:fs').readFileSync(new URL('../src/v2/sections/open/office/index.tsx', import.meta.url), 'utf8') as string
    expect(office).toContain("loading={engine.name !== 'ready' && engine.name !== 'failed'} failed={engine.name === 'failed'} uploading={note?.kind === 'local'}")
  })
})
