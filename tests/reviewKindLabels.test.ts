/**
 * Task 2190 (review-169 #6): every kind the engine's review feed sends has a TypeScript kind and a
 * label of its own in the Review list, so a new kind is never shown under the "Conflict" default.
 * The kinds are read from the engine's source: each `kind: "…".to_string()` in its production code,
 * and the first string of each `classify_review_operation` tuple.
 */
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

const ROOT = join(import.meta.dir, '..')
const ENGINE = readFileSync(join(ROOT, 'src-tauri', 'src', 'engine_bridge.rs'), 'utf8').replace(/\r\n/g, '\n')
const PRODUCTION = ENGINE.slice(0, ENGINE.indexOf('\n#[cfg(test)]\nmod tests {'))
const API = readFileSync(join(ROOT, 'src', 'desktopApi.ts'), 'utf8')
const LABELS = readFileSync(join(ROOT, 'src', 'reviewKindLabel.ts'), 'utf8')

function rustKinds(): string[] {
  const kinds = new Set<string>()
  for (const match of PRODUCTION.matchAll(/kind: "([a-z_]+)"\.to_string\(\)/g)) kinds.add(match[1])
  const classify = PRODUCTION.slice(PRODUCTION.indexOf('fn classify_review_operation('))
  const body = classify.slice(0, classify.indexOf('\n}\n'))
  for (const match of body.matchAll(/\(\s*\n\s*"([a-z_]+)",/g)) kinds.add(match[1])
  return [...kinds].sort()
}

describe('review kinds', () => {
  test('the census sees the engine', () => {
    expect(rustKinds().length).toBeGreaterThanOrEqual(12)
    expect(rustKinds()).toContain('held_unknown')
  })

  test('every Rust kind is a TypeScript kind and has its own label', () => {
    const start = LABELS.indexOf('function reviewKindLabel(')
    expect(start).toBeGreaterThanOrEqual(0)
    const label = LABELS.slice(start, LABELS.indexOf('default:', start))
    for (const kind of rustKinds()) {
      expect(API.includes(`| '${kind}'`), `desktopApi kind ${kind}`).toBe(true)
      // `conflict` is the label's default ("Conflict"); every other kind names its own case.
      if (kind !== 'conflict') expect(label.includes(`case '${kind}':`), `reviewKindLabel ${kind}`).toBe(true)
    }
  })
})
