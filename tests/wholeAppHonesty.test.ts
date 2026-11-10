// Whole-app honesty (slice 42): no desktop v2 surface paints a blank card, draws the board's example
// people or files, or disagrees with docs/FEATURE-MAP.md.
//
// The surface set is read from source, never from a hand list: the three Settings pane tables
// (BASE_PANES, PANES_TODAY, PANES_ACCOUNT), every src/v2/sections/<id>/index.tsx on disk, and the three
// windows outside a section (the tray flyout, the sign-in gate's signed-out screen, onboarding's setup
// steps). A pane id and a section id may coincide (`backups`): each screen is its own surface.
//
// First paint is renderToStaticMarkup, inside the screen's normal context: the Settings Ctx for a pane,
// a Navigation for a section. Effects do not run, so a screen that only fills itself from a command
// paints what it claims before any read: real rows, an honest empty state, or an error line. The
// desktop command answers unsupported for every name (no host), so nothing can fill a page with a
// success it never read. The board's example names are rejected in the markup and in its text.
//
// The gate and onboarding windows paint their session check first (`aria-busy`, an honest shell, no
// card). What the map names for them is the screen that follows: the gate's signed-out entry
// (AccountEntry) and onboarding's setup steps (SetupFlow). Those are the surfaces painted here, and
// the shell is pinned so it cannot become an unexplained blank card.
//
// Map consistency is checked per group (panes, sections, windows): every id found in source is a row
// in docs/FEATURE-MAP.md, a Live surface is not blank, a Missing surface says the command is not in
// this build, and a Placeholder surface says on screen that it is not live yet.
import { describe, expect, mock, test } from 'bun:test'
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs'
import { join, resolve } from 'node:path'
import type { ComponentType } from 'react'
import type { PaneId, SettingsCtx } from '../src/v2/sections/settings/shell/context'
import type { Navigation } from '../src/v2/routes'

// General and Bandwidth subscribe with useSyncExternalStore and no server snapshot. React's server
// renderer throws without one. The client's first paint is getSnapshot(), so the test supplies that as
// the server snapshot and does not change the panes.
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
const apiActual = await import(`${apiPath}?actual=1`)

mock.module(apiPath, () => ({
  ...apiActual,
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
const { default: TrayApp } = await import('../src/v2/tray/index')
const { AuthGate } = await import('../src/v2/auth/Gate')
const { AccountEntry } = await import('../src/v2/auth/create/entry')
const { default: OnboardingApp, SetupFlow } = await import('../src/v2/onboarding/index')

const ROOT = resolve(import.meta.dir, '..')
const DOC_PATH = join(ROOT, 'docs/FEATURE-MAP.md')

type Group = 'pane' | 'section' | 'window'
interface Surface { key: string; id: string; group: Group }

const panes: Partial<Record<PaneId, ComponentType>> = { ...BASE_PANES, ...PANES_TODAY, ...PANES_ACCOUNT }

/** The section folders that ship an index, the same set the window mounts from src/v2/sections/. */
function sectionFolders(): string[] {
  const dir = join(ROOT, 'src/v2/sections')
  return readdirSync(dir)
    .filter((name) => {
      const at = join(dir, name)
      return statSync(at).isDirectory() && existsSync(join(at, 'index.tsx'))
    })
    .sort()
}

const sectionModules: Record<string, ComponentType<{ nav: Navigation }>> = {}
for (const id of sectionFolders()) {
  sectionModules[id] = (await import(`../src/v2/sections/${id}/index.tsx`)).default
}

const PANE_IDS = Object.keys(panes) as PaneId[]
const SECTION_IDS = Object.keys(sectionModules)
const WINDOW_IDS = ['auth', 'onboarding', 'tray'] as const

const surfaces: Surface[] = [
  ...PANE_IDS.map((id) => ({ key: `pane:${id}`, id, group: 'pane' as const })),
  ...SECTION_IDS.map((id) => ({ key: `section:${id}`, id, group: 'section' as const })),
  ...WINDOW_IDS.map((id) => ({ key: `window:${id}`, id, group: 'window' as const })),
]

/** The board's example people and files, from tests/v2NoFixturesInProduction.test.ts. */
const EXAMPLES: ReadonlyArray<readonly [string, RegExp]> = [
  ['Anna', /\bAnna\b/],
  ['Lotte de Wit', /Lotte de Wit/],
  ['Priya Shah', /Priya Shah/],
  ['Harbourlight', /Harbourlight/],
  ['Render final', /Render final/],
  ['Wedding 2025', /Wedding 2025/],
]

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

/** Visible text with the screen's h1 removed once. Empty means it painted nothing else. */
function beyondTitle(html: string): string {
  const text = visible(html)
  const h1 = html.match(/<h1\b[^>]*>([\s\S]*?)<\/h1>/)
  const title = h1 ? visible(h1[1]) : ''
  if (!title) return text
  const at = text.indexOf(title)
  if (at < 0) return text
  return `${text.slice(0, at)} ${text.slice(at + title.length)}`.replace(/\s+/g, ' ').trim()
}

// useDriveSource throws without a provider. The stub carries the host's capability flags so a screen can
// ask about them on first paint; its reads are effects and never run here.
const source = {
  kind: 'desktop',
  subscribe: () => () => {},
  capabilities: { localCopies: true, revealInFileManager: true, office: true, nativePickers: true },
} as never

const nav = (id: string): Navigation => ({
  route: { section: id as Navigation['route']['section'], id: null, sub: null },
  go() {},
  back() {},
  forward() {},
})

/** One surface's first paint in its normal context. */
function paint(surface: Surface): string {
  if (surface.group === 'pane') {
    const Pane = panes[surface.id as PaneId]
    if (!Pane) throw new Error(`no pane component for ${surface.id}`)
    const ctx: SettingsCtx = { pane: surface.id as PaneId, go() {}, dialog: null, open() {}, close() {} }
    return renderToStaticMarkup(
      createElement(DriveSourceProvider, { source }, createElement(Ctx.Provider, { value: ctx }, createElement(Pane))),
    )
  }
  if (surface.group === 'section') {
    const Section = sectionModules[surface.id]
    if (!Section) throw new Error(`no section component for ${surface.id}`)
    return renderToStaticMarkup(createElement(DriveSourceProvider, { source }, createElement(Section, { nav: nav(surface.id) })))
  }
  switch (surface.id) {
    case 'tray': return renderToStaticMarkup(createElement(TrayApp))
    case 'onboarding': return renderToStaticMarkup(createElement(SetupFlow))
    case 'auth': return renderToStaticMarkup(createElement(AccountEntry, { startAt: 'sign-in', onSignedIn() {}, onCreated() {} }))
    default: throw new Error(`no window component for ${surface.id}`)
  }
}

/** First paint of the session check the gate and onboarding draw before their content: an honest busy shell. */
function sessionCheck(window: 'auth' | 'onboarding'): string {
  return window === 'auth'
    ? renderToStaticMarkup(createElement(AuthGate, { children: 'files' }))
    : renderToStaticMarkup(createElement(OnboardingApp))
}

const STATUS = /\b(Live|Placeholder|Missing)\b/
const doc = readFileSync(DOC_PATH, 'utf8')

/** A markdown table row that names this id in its own cell and carries a status word. */
function rowFor(id: string): string | undefined {
  return doc.split('\n').find((line) => {
    if (!line.startsWith('|')) return false
    const cells = line.split('|').map((cell) => cell.trim())
    return cells.includes(id) && STATUS.test(line)
  })
}

const statusOf = (row: string) => row.match(STATUS)![1]

/** Not-live wording a Missing surface must say: the command is not in this build. */
const MISSING_WORDS = /in this build|not available|can.t be chosen|isn.t available|not in this version/i
/** Not-live-yet wording a Placeholder surface must say. */
const PLACEHOLDER_WORDS = /not live|coming soon|not available|doesn.t work|only close|nothing to change/i

describe('every desktop v2 surface tells the truth in one pass', () => {
  for (const surface of surfaces) {
    test(surface.key, async () => {
      // No host: every command name is unsupported. The check only proves the stub is wired.
      expect(await command(surface.id)).toEqual({ ok: false, reason: 'unsupported' })

      const html = paint(surface)
      const rest = beyondTitle(html)
      expect(rest, `${surface.key} painted no visible text beyond its title`).not.toBe('')

      const text = visible(html)
      for (const [name, pattern] of EXAMPLES) {
        expect(html, `${surface.key} drew the board's example name ${name}`).not.toMatch(pattern)
        expect(text, `${surface.key} drew the board's example name ${name}`).not.toMatch(pattern)
      }

      if (surface.id === 'auth' || surface.id === 'onboarding') {
        // The session check is a busy shell, never an unexplained blank card.
        expect(sessionCheck(surface.id)).toContain('aria-busy')
      }
    })
  }
})

describe('the feature map agrees with what each surface painted', () => {
  for (const group of ['pane', 'section', 'window'] as Group[]) {
    test(`${group} surfaces`, () => {
      // The enumeration: 18 panes (8 + 7 + 3), 10 section folders, 3 windows outside a section. `backups`
      // is a Settings pane and a Drive section: two screens, one id, both painted.
      expect(PANE_IDS).toHaveLength(18)
      expect(SECTION_IDS).toEqual([
        'available-offline',
        'backups',
        'files',
        'open',
        'photos',
        'search',
        'settings',
        'shared',
        'sync-history',
        'trash',
      ])
      expect(WINDOW_IDS).toEqual(['auth', 'onboarding', 'tray'])
      expect(surfaces).toHaveLength(31)

      const groupSurfaces = surfaces.filter((surface) => surface.group === group)
      expect(groupSurfaces.length).toBeGreaterThan(0)
      for (const surface of groupSurfaces) {
        const row = rowFor(surface.id)
        expect(row, `${surface.id} is not a row in docs/FEATURE-MAP.md`).toBeDefined()
        const status = statusOf(row!)
        const html = paint(surface)
        const text = visible(html)
        if (status === 'Live') {
          expect(beyondTitle(html), `${surface.id} is Live but painted no text beyond its title`).not.toBe('')
        } else if (status === 'Missing') {
          expect(html, `${surface.id} is Missing but its first paint does not say so`).toMatch(MISSING_WORDS)
        } else {
          expect(text, `${surface.id} is Placeholder but does not say it is not live yet`).toMatch(PLACEHOLDER_WORDS)
        }
      }
    })
  }
})
