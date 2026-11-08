import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import ts from 'typescript'
import { backUpAgainLabel } from '../src/backupStop'

// LD-0139 (spec Amendments 12 and 12c): the Backups card shows why a backup stopped, in core's words sent by
// `get_known_folder_backup`, and offers "Back up again". The production `backupStatus` is compiled and called.
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
const ERASED = 'Your files were erased, so backups are off.'
const pictures = {
  key: 'pictures',
  display_name: 'Pictures',
  source_path: 'C:\\Users\\guus\\Pictures',
  enabled: false,
  item_count: null,
  stop_words: ERASED,
  stop_action: 'Back up again',
  check_note: null,
}

describe('a backup that stopped by itself (LD-0139)', () => {
  test('says why in core’s words and offers Back up again', () => {
    expect(backupStatus(pictures, false)).toEqual({ text: ERASED, tone: 'off' })
    expect(backUpAgainLabel(pictures)).toBe('Back up again')
  })
  test('switched off by the person, it has no words and no button', () => {
    const off = { ...pictures, stop_words: null, stop_action: null }
    expect(backupStatus(off, false)).toEqual({ text: 'Not backed up', tone: 'off' })
    expect(backUpAgainLabel(off)).toBeNull()
  })
  test('on again, the old words are gone', () => {
    const on = { ...pictures, enabled: true, stop_words: null, stop_action: null }
    expect(backupStatus(on, false).tone).toBe('on')
    expect(backUpAgainLabel({ ...pictures, enabled: true })).toBeNull()
  })
  test('a notices check that failed says so on a backup that is on', () => {
    const waiting = { ...pictures, enabled: true, stop_words: null, check_note: "Couldn't check — will retry." }
    expect(backupStatus(waiting, false)).toEqual({ text: "Couldn't check — will retry.", tone: 'busy' })
  })
})
