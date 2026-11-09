// Slice 14: every settings pane and dialog id, every section folder, and the windows that are not a section
// (guide, onboarding, auth gate, tray) is a row in docs/FEATURE-MAP.md with a status word. Source scan only:
// the pane tables and the section folders on disk. The doc is the claim; this file does not judge Live or Missing.
import { describe, expect, test } from 'bun:test'
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { BASE_PANES, DIALOGS } from '../src/v2/sections/settings/shell/panes'
import { DIALOGS_TODAY, PANES_TODAY } from '../src/v2/sections/settings/panes-today'
import { PANES_ACCOUNT } from '../src/v2/sections/settings/panes-account'

const ROOT = resolve(import.meta.dir, '..')
const DOC_PATH = join(ROOT, 'docs/FEATURE-MAP.md')

/** Section folders that ship an index, the same set the window mounts from src/v2/sections/. */
export function sectionFolders(): string[] {
  const dir = join(ROOT, 'src/v2/sections')
  return readdirSync(dir)
    .filter((name) => {
      const at = join(dir, name)
      return statSync(at).isDirectory() && existsSync(join(at, 'index.tsx'))
    })
    .sort()
}

/**
 * One id per surface the map has to name. Pane and dialog keys come from the three registration tables
 * (a later table's same id is still one surface). Section folders and the four windows outside sections are added.
 */
export function featureSurfaces(): string[] {
  const ids = [
    ...Object.keys(BASE_PANES),
    ...Object.keys(DIALOGS),
    ...Object.keys(PANES_TODAY),
    ...Object.keys(DIALOGS_TODAY),
    ...Object.keys(PANES_ACCOUNT),
    ...sectionFolders(),
    'guide',
    'onboarding',
    'auth',
    'tray',
  ]
  return [...new Set(ids)].sort()
}

const STATUS = /\b(Live|Placeholder|Missing)\b/

/** A markdown table row that names this id in its own cell and carries a status word. */
export function rowFor(doc: string, id: string): string | undefined {
  return doc.split('\n').find((line) => {
    if (!line.startsWith('|')) return false
    const cells = line.split('|').map((cell) => cell.trim())
    return cells.includes(id) && STATUS.test(line)
  })
}

const surfaces = featureSurfaces()
const doc = existsSync(DOC_PATH) ? readFileSync(DOC_PATH, 'utf8') : ''

describe('docs/FEATURE-MAP.md names every surface with a status', () => {
  test('the scan reads the pane tables and the section folders (a scan of nothing would pass)', () => {
    expect(Object.keys(BASE_PANES).length).toBeGreaterThan(0)
    expect(Object.keys(DIALOGS)).toContain('report')
    expect(Object.keys(DIALOGS)).toContain('limit-previews')
    expect(Object.keys(DIALOGS)).toContain('limit-downloads')
    expect(Object.keys(PANES_TODAY)).toContain('location')
    expect(Object.keys(PANES_TODAY)).toContain('bandwidth')
    expect(Object.keys(DIALOGS_TODAY)).toContain('keep')
    expect(Object.keys(PANES_ACCOUNT)).toEqual(['account', 'security', 'devices'])
    expect(sectionFolders()).toEqual([
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
    expect(surfaces).toContain('guide')
    expect(surfaces).toContain('onboarding')
    expect(surfaces).toContain('auth')
    expect(surfaces).toContain('tray')
    expect(surfaces.length).toBeGreaterThan(30)
  })

  for (const id of surfaces) {
    test(`${id} is a table row with Live, Placeholder, or Missing`, () => {
      expect(rowFor(doc, id), id).toBeDefined()
    })
  }
})
