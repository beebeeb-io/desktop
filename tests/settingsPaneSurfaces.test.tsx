// No Settings pane renders blank. The panes are the three tables SettingsView merges,
// not a hand list. First paint is renderToStaticMarkup: effects do not run, so a pane
// that returns nothing, or only its title, fails here. command() answers unsupported
// for every name, so a render-time read cannot fill the page with a success or with
// the board's example people and files.
import { describe, expect, mock, test } from 'bun:test'
import type { ComponentType } from 'react'
import type { DriveSource } from '@beebeeb/shared/drive-ui'
import type { PaneId, SettingsCtx } from '../src/v2/sections/settings/shell/context'

// General and Bandwidth subscribe with useSyncExternalStore and no server snapshot.
// React's server renderer throws without one. The client's first paint is getSnapshot(),
// so the test supplies that as the server snapshot and does not change the panes.
const reactActual = await import('react')
// Copy before mock.module. Spreading the live module inside the factory re-enters the mock.
const reactPlain = { ...reactActual }
const realUseSyncExternalStore = reactPlain.useSyncExternalStore
mock.module('react', () => ({
  ...reactPlain,
  useSyncExternalStore<T>(
    subscribe: (onStoreChange: () => void) => () => void,
    getSnapshot: () => T,
    getServerSnapshot?: () => T,
  ): T {
    return realUseSyncExternalStore(subscribe, getSnapshot, getServerSnapshot ?? getSnapshot)
  },
}))

const apiPath = new URL('../src/desktopApi.ts', import.meta.url).pathname
const actual = await import(`${apiPath}?actual=1`)

mock.module(apiPath, () => ({
  ...actual,
  command: async (_name: string) => ({ ok: false, reason: 'unsupported' }),
}))

const { createElement } = await import('react')
const { renderToStaticMarkup } = await import('react-dom/server')
const { DriveSourceProvider } = await import('@beebeeb/shared/drive-ui')
const { Ctx } = await import('../src/v2/sections/settings/shell/context')
const { BASE_PANES } = await import('../src/v2/sections/settings/shell/panes')
const { PANES_TODAY } = await import('../src/v2/sections/settings/panes-today')
const { PANES_ACCOUNT } = await import('../src/v2/sections/settings/panes-account')
const { command } = await import(apiPath)

const panes: Partial<Record<PaneId, ComponentType>> = { ...BASE_PANES, ...PANES_TODAY, ...PANES_ACCOUNT }
const ids = Object.keys(panes) as PaneId[]
// Count found by merging the three tables. SettingsView uses the same spread.
const PANE_COUNT = ids.length

const EXAMPLES = [/\bAnna\b/, /Lotte de Wit/, /Priya Shah/, /Harbourlight/, /Render final/, /Wedding 2025/]

function visible(html: string): string {
  return html
    .replace(/<[^>]+>/g, ' ')
    .replace(/&#x27;|&#39;/g, "'")
    .replace(/&amp;/g, '&')
    .replace(/&quot;/g, '"')
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&nbsp;/g, ' ')
    .replace(/\s+/g, ' ')
    .trim()
}

/** Visible text with the page's h1 removed once. Empty means the pane painted nothing else. */
function beyondTitle(html: string): string {
  const text = visible(html)
  const h1 = html.match(/<h1\b[^>]*>([\s\S]*?)<\/h1>/)
  const title = h1 ? visible(h1[1]) : ''
  if (!title) return text
  const at = text.indexOf(title)
  if (at < 0) return text
  return `${text.slice(0, at)} ${text.slice(at + title.length)}`.replace(/\s+/g, ' ').trim()
}

// useDriveSource throws without a provider. The stub is never read on first paint (reads are effects).
const source = { kind: 'desktop', subscribe: () => () => {} } as unknown as DriveSource

function markup(id: PaneId): string {
  const Pane = panes[id]
  if (!Pane) throw new Error(`no pane component for ${id}`)
  const ctx: SettingsCtx = { pane: id, go() {}, dialog: null, open() {}, close() {} }
  return renderToStaticMarkup(
    createElement(
      DriveSourceProvider,
      { source },
      createElement(Ctx.Provider, { value: ctx }, createElement(Pane)),
    ),
  )
}

describe('no Settings pane renders blank', () => {
  for (const id of ids) {
    test(id, async () => {
      expect(PANE_COUNT).toBe(Object.keys(BASE_PANES).length + Object.keys(PANES_TODAY).length + Object.keys(PANES_ACCOUNT).length)
      expect(PANE_COUNT).toBe(18)
      expect(ids).toHaveLength(PANE_COUNT)
      expect(await command(id)).toEqual({ ok: false, reason: 'unsupported' })
      const html = markup(id)
      const rest = beyondTitle(html)
      expect(rest, `${id} painted no visible text beyond its title`).not.toBe('')
      const text = visible(html)
      for (const name of EXAMPLES) {
        expect(html).not.toMatch(name)
        expect(text).not.toMatch(name)
      }
    })
  }
})
