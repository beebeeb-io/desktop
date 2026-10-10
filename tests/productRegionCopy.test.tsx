import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import React from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import ts from 'typescript'
import { productRegionLabel } from '../src/windows/useRegion'
import type { UserRegionResponse } from '../src/desktopApi'

const regions = [
  { continent: 'europe', display_name: 'Europe', city: 'Falkenstein', is_default: true },
  { continent: 'us', display_name: 'North America', city: 'Ashburn', is_default: false },
]
const cases: [string, UserRegionResponse | null | undefined, string][] = [
  ['EU', { preferred_region: 'europe', regions }, 'Stored in the EU'],
  ['US overriding EU default', { preferred_region: 'us', regions }, 'Stored in North America'],
  ['default EU', { preferred_region: null, regions }, 'Stored in the EU'],
  ['default US', { preferred_region: null, regions: regions.map(r => ({ ...r, is_default: r.continent === 'us' })) }, 'Stored in North America'],
  ['unknown preference', { preferred_region: 'unknown', regions }, 'End-to-end encrypted'],
  ['missing preferred metadata', { preferred_region: 'us', regions: [regions[0]] }, 'End-to-end encrypted'],
  ['no default', { preferred_region: null, regions: regions.map(r => ({ ...r, is_default: false })) }, 'End-to-end encrypted'],
  ['empty metadata', { preferred_region: null, regions: [] }, 'End-to-end encrypted'],
  ['loading', undefined, 'End-to-end encrypted'],
  ['error', null, 'End-to-end encrypted'],
]

// Render the actual copy fragments from each shipping JSX surface. This keeps
// copy assertions independent of unrelated IPC/loading state in the full page;
// brandResidencyCopy separately guards every hook binding and hardcoded claim.
const surfaces: [string, number][] = [
  ['Onboarding.tsx', 1], ['WindowsFirstRun.tsx', 1], ['WindowsApp.tsx', 3],
  ['pages/Account.tsx', 1], ['windows/KnownFolderOnboarding.tsx', 1],
  ['windows/views/AccountView.tsx', 1], ['windows/views/ActivityView.tsx', 1],
  ['windows/views/InsightsView.tsx', 1], ['windows/views/SelectiveSyncView.tsx', 1],
  // Two: the Windows/Linux Explorer panel and the macOS Finder panel (Task 17), both from useRegionLabel.
  ['windows/views/SettingsView.tsx', 2],
]

function copyFragments(file: string): string[] {
  const source = readFileSync(new URL(`../src/${file}`, import.meta.url), 'utf8')
  const ast = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX)
  const fragments: string[] = []
  function visit(node: ts.Node) {
    if (ts.isJsxElement(node) && node.children.some(child =>
      ts.isJsxExpression(child) && child.expression?.getText(ast) === 'regionLabel')) {
      // The literal text and label expression are the product copy. Other
      // children are icons, status badges or controls, tested by their owners.
      const copy = node.children.filter(child => ts.isJsxText(child)
        || (ts.isJsxExpression(child) && child.expression?.getText(ast) === 'regionLabel'))
        .map(child => child.getFullText(ast)).join('')
      fragments.push(`<>${copy}</>`)
    }
    if (ts.isJsxAttribute(node) && node.name.getText(ast) === 'subtitle'
      && node.initializer && ts.isJsxExpression(node.initializer)
      && node.initializer.expression?.getText(ast).includes('${regionLabel}')) {
      fragments.push(node.initializer.expression.getText(ast))
    }
    ts.forEachChild(node, visit)
  }
  visit(ast)
  return fragments
}

for (const [state, response, expected] of cases) {
  describe(state, () => {
    test('effective continent resolves without city/provider or default guessing', () => {
      expect(productRegionLabel(response)).toBe(expected)
    })
    for (const [file, count] of surfaces) {
      test(`${file}: all ${count} copy slots`, () => {
        const fragments = copyFragments(file)
        expect(fragments).toHaveLength(count)
        for (const fragment of fragments) {
          const code = ts.transpileModule(`return (${fragment})`, {
            compilerOptions: { jsx: ts.JsxEmit.React, target: ts.ScriptTarget.ES2022 },
          }).outputText
          const render = new Function('React', 'regionLabel', 'fileSurfaceName', 'deviceNoun', code)
          const html = renderToStaticMarkup(render(React, productRegionLabel(response), 'File Explorer', 'this PC'))
          expect(html).toContain(expected)
          expect(html).not.toMatch(/Falkenstein|Germany|Ashburn|provider/i)
          if (expected === 'End-to-end encrypted') expect(html).not.toMatch(/Stored in|the EU|North America/)
          if (expected === 'Stored in North America') expect(html).not.toContain('the EU')
          if (expected === 'Stored in the EU') expect(html).not.toContain('North America')
        }
      })
    }
    for (const file of ['DevicesView', 'SecurityView']) {
      test(`${file}: no location claim for this account`, () => {
        const source = readFileSync(new URL(`../src/windows/views/${file}.tsx`, import.meta.url), 'utf8')
        expect(source).not.toMatch(/Stored in|the EU|North America|Falkenstein|Ashburn/)
      })
    }
  })
}

// Execute the shipping hook with controlled state/effects to check its initial
// neutral value and the fetch-to-label connection (not React scheduling).
for (const [state, response, expected] of cases) {
  test(`hook: loading then ${state}`, async () => {
    const source = readFileSync(new URL('../src/windows/useRegion.ts', import.meta.url), 'utf8')
    const ast = ts.createSourceFile('useRegion.ts', source, ts.ScriptTarget.Latest, true)
    const hook = ast.statements.find(n => ts.isFunctionDeclaration(n) && n.name?.text === 'useRegionLabel')!
    const compiled = ts.transpileModule(hook.getText(ast).replace(/^export /, ''), {
      compilerOptions: { target: ts.ScriptTarget.ES2022 },
    }).outputText
    let value: unknown = null
    let effect: (() => unknown) | undefined
    let requests = 0
    const useLabel = new Function('useState', 'useEffect', 'fetchRegion', 'productRegionLabel', `${compiled}; return useRegionLabel`)(
      () => [value, (next: unknown) => { value = next }],
      (fn: () => unknown) => { effect = fn },
      async () => { requests++; return response },
      productRegionLabel,
    )
    expect(useLabel('signin')).toBe('End-to-end encrypted')
    effect!()
    await Promise.resolve()
    expect(requests).toBe(1)
    expect(useLabel('signin')).toBe(expected)
    // A new onboarding step must hide the previous response until refreshed.
    expect(useLabel('ready')).toBe('End-to-end encrypted')
  })
}
