import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import ts from 'typescript'

// LD-0113 Amendment 9: the Backups card's status line for a folder OneDrive moved. The production declaration is
// compiled and called; no copy of its logic lives here.
function declaration(file: string, name: string) {
  const source = readFileSync(new URL(`../src/${file}`, import.meta.url), 'utf8')
  const ast = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX)
  const found = ast.statements.find(n => ts.isFunctionDeclaration(n) && n.name?.text === name)
  if (!found) throw new Error(`Missing ${name}`)
  const compiled = ts.transpileModule(found.getText(ast).replace(/^export (default )?/, ''), {
    compilerOptions: { jsx: ts.JsxEmit.React, target: ts.ScriptTarget.ES2022 },
  }).outputText
  return new Function(`${compiled}; return ${name}`)()
}

const backupStatus = declaration('WindowsApp/filesView.tsx', 'backupStatus')
const pictures = {
  key: 'pictures',
  display_name: 'Pictures',
  source_path: 'C:\\Users\\guus\\Pictures',
  enabled: true,
  item_count: null,
  moved_path: 'C:\\Users\\guus\\OneDrive\\Pictures',
  source_choice: null,
  question: 'OneDrive has moved your Pictures folder. Back up OneDrive’s folder …?',
  note: null,
}

describe('a folder OneDrive moved (LD-0113 Amendment 9)', () => {
  test('waits for the answer while it is on and unanswered', () => {
    expect(backupStatus(pictures, false)).toEqual({ text: 'Waiting for your answer: not backed up yet', tone: 'busy' })
  })
  test('answered, it is backed up as any folder', () => {
    expect(backupStatus({ ...pictures, question: null, source_choice: 'onedrive' }, false).tone).toBe('on')
  })
  test('off, it never waits', () => {
    expect(backupStatus({ ...pictures, enabled: false }, false).tone).toBe('off')
  })
})
