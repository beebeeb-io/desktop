/**
 * Guard for task 1546 finding 4: the workspace brand rule ("EU references:
 * Name the city... No flag emojis", CLAUDE.md) requires user-facing residency
 * copy to name Falkenstein, never a vague "EU servers". Every other surface in
 * this repo already does this (WindowsApp.tsx, WindowsFirstRun.tsx); the macOS
 * onboarding sidebar footer was the one holdout.
 */
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

const ONBOARDING_SRC = readFileSync(join(import.meta.dir, '..', 'src', 'Onboarding.tsx'), 'utf8')

describe('macOS onboarding residency copy', () => {
  test('sidebar footer names Falkenstein, not a vague "EU servers"', () => {
    expect(ONBOARDING_SRC).not.toContain('EU servers')
    expect(ONBOARDING_SRC).toContain('Falkenstein')
  })
})
