/**
 * The one reading of "how long ago" (PR #140 review, P3): the Mac Security rows and the Windows Devices view say it in their
 * own words over the same buckets. The buckets are pinned here; the words are pinned where they are said
 * (tests/macSecurityModel.test.ts for the Mac's, the Devices view keeps its terse ones).
 */
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { ageOf } from '../src/relativeTime'

const NOW = Date.UTC(2026, 9, 25, 12, 0, 0)
const ago = (seconds: number) => new Date(NOW - seconds * 1000)
const DAY = 86_400

describe('ageOf', () => {
  test('under 45 seconds is now, and so is a clock that is ahead of ours', () => {
    expect(ageOf(ago(0), NOW, 30)).toEqual({ unit: 'now' })
    expect(ageOf(ago(44), NOW, 30)).toEqual({ unit: 'now' })
    expect(ageOf(ago(-3600), NOW, 30)).toEqual({ unit: 'now' })
  })

  test('then whole minutes, rounded, up to an hour', () => {
    expect(ageOf(ago(45), NOW, 30)).toEqual({ unit: 'minute', count: 1 })
    expect(ageOf(ago(5 * 60), NOW, 30)).toEqual({ unit: 'minute', count: 5 })
    expect(ageOf(ago(59 * 60 + 20), NOW, 30)).toEqual({ unit: 'minute', count: 59 })
  })

  test('whole hours up to a day, whole days up to the date threshold, then the date itself', () => {
    expect(ageOf(ago(3600), NOW, 30)).toEqual({ unit: 'hour', count: 1 })
    expect(ageOf(ago(23 * 3600), NOW, 30)).toEqual({ unit: 'hour', count: 23 })
    expect(ageOf(ago(DAY), NOW, 30)).toEqual({ unit: 'day', count: 1 })
    expect(ageOf(ago(13 * DAY), NOW, 14)).toEqual({ unit: 'day', count: 13 })
    expect(ageOf(ago(14 * DAY), NOW, 14)).toEqual({ unit: 'date', date: ago(14 * DAY) })
    expect(ageOf(ago(29 * DAY), NOW, 30)).toEqual({ unit: 'day', count: 29 })
    expect(ageOf(ago(30 * DAY), NOW, 30)).toEqual({ unit: 'date', date: ago(30 * DAY) })
  })
})

describe('nobody keeps a second copy of the buckets', () => {
  const src = (file: string) => readFileSync(new URL(`../src/${file}`, import.meta.url), 'utf8')

  test('the Mac rows and the Devices view both ask ageOf, and neither rounds minutes or hours itself', () => {
    for (const file of ['macSecurityModel.ts', 'windows/views/DevicesView.tsx']) {
      const text = src(file)
      expect(text, file).toContain("from '../../relativeTime'".replace('../../', file.includes('/') ? '../../' : './'))
      expect(text, file).toContain('ageOf(')
      expect(text, file).not.toMatch(/Math\.round\((sec|seconds|min|minutes|hr|hours) \/ (60|24)\)/)
    }
  })
})
