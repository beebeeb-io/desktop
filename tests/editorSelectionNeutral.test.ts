// Task 2321: desktop's text editor paints a selection and a find match in a neutral ink/paper mix, never Amber.
// The mounted CodeMirror theme lives in packages/shared (a generated mirror, not ours to edit), so this pins the
// desktop's own override stylesheet: it must exist outside packages/shared, set both surfaces without Amber, and
// be imported by the module that mounts the editor.
import { describe, expect, test } from 'bun:test'
import { existsSync, readFileSync } from 'node:fs'
import { join, relative } from 'node:path'

const ROOT = join(import.meta.dir, '..')
const CSS_PATH = join(ROOT, 'src/v2/sections/open/editor-chrome.css')
const EDITING_PATH = join(ROOT, 'src/v2/sections/open/Editing.tsx')

/** The stylesheet as code: a comment may name the shared Amber rules it replaces, and is not a declaration. */
const code = (src: string) => src.replace(/\/\*[\s\S]*?\*\//g, '')

/** The declared value(s) of `selector` — the first rule whose selector text matches, up to its closing brace. */
function block(css: string, selector: string): string | null {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  const m = css.match(new RegExp(`${escaped}\\s*\\{([^}]*)\\}`))
  return m ? m[1] : null
}

const isNeutral = (value: string) => /var\(--color-(ink|paper)(-[0-9])?\)/.test(value)

describe('task 2321: the desktop editor selection and find matches are neutral, not Amber', () => {
  test('a desktop stylesheet outside packages/shared sets the selection and both search-match colours without Amber', () => {
    expect(existsSync(CSS_PATH)).toBe(true)
    // The override belongs to desktop; packages/shared is generated and must stay untouched.
    expect(relative(join(ROOT, 'packages/shared'), CSS_PATH).startsWith('..')).toBe(true)
    const css = code(readFileSync(CSS_PATH, 'utf8'))

    // Selection: the shared theme derives it from --color-amber; the desktop re-declares the token.
    const selection = css.match(/--ed-selection\s*:\s*([^;]+);/)
    expect(selection).not.toBeNull()
    expect(isNeutral(selection![1])).toBe(true)
    expect(selection![1]).not.toMatch(/amber/i)

    // Find matches: .cm-searchMatch and .cm-searchMatch-selected.
    for (const selector of ['.cm-searchMatch', '.cm-searchMatch-selected']) {
      const body = block(css, selector)
      expect(body).not.toBeNull()
      expect(body!).toMatch(/background\s*:/)
      expect(body!).not.toMatch(/amber/i)
      expect(isNeutral(body!)).toBe(true)
    }

    // The find field's focus ring: neutral outline, not an Amber one. CodeMirror's field has no `type`
    // attribute, so the desktop rule must also reach the class it really carries.
    for (const selector of [".cm-panel.cm-search input[type='text']:focus", '.cm-panel.cm-search input.cm-textfield:focus']) {
      const focus = block(css, selector)
      expect(focus).not.toBeNull()
      expect(focus!).toMatch(/outline\s*:/)
      expect(focus!).not.toMatch(/amber/i)
      expect(isNeutral(focus!)).toBe(true)
    }
  })

  test('Editing.tsx imports the desktop override and still mounts the shared CodeMirror editor', () => {
    const src = readFileSync(EDITING_PATH, 'utf8')
    expect(src).toContain("import './editor-chrome.css'")
    expect(src).toContain("from '@beebeeb/shared/drive-ui'")
    expect(src).toContain('<CodeMirrorEditor')
  })
})
