// M4 overlay rule (rulings v2-m3c item 2, v2-m3d item 2): when the target frame's section differs from the source's,
// the audit scores only the target's dialog or message layer, never the screen behind it. The pure seam is
// scripts/v2-map-audit/compare.mjs; these six checks pin the rule, its limits and the untouched family rule.
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { dialogLayer, isOverlaySection } from '../scripts/v2-map-audit/compare.mjs'

const ROOT = resolve(import.meta.dir, '..')
const AUDIT = readFileSync(join(ROOT, 'scripts/v2-map-audit/audit.mjs'), 'utf8')

type Sig = string[]

const SECTION_OF: Record<string, string> = {
  'DW-0401': 'Files',
  'DW-0407': 'Files',
  'DW-0408': 'Files',
  'DW-0408b': 'Files',
  'DW-0411': 'Files',
  'DW-0505': 'Shared',
}

const cover = (sig: Sig, state: Set<string>) => (sig.length ? sig.filter((t) => state.has(t)).length / sig.length : 0)
const family = (id: string) => id.replace(/[a-z]$/, '')

/**
 * The audit's score(): one line chooses which signature is scored. Everything else is the audit's formula, copied so
 * the test reads as the production comparison (0.5 whole signature + 0.5 what the frame adds to the starting screen).
 */
function decide(targetIds: string[], sigs: Record<string, Sig>, pre: Set<string>, post: Set<string>, changed: boolean, sourceId: string) {
  const sourceSection = SECTION_OF[sourceId]
  const score = (id: string) => {
    const own = sigs[id] ?? []
    const sig = isOverlaySection(sourceSection, SECTION_OF[id]) ? dialogLayer(own) : own
    const distinct = sig.filter((x) => !pre.has(x))
    return 0.5 * cover(sig, post) + 0.5 * (distinct.length ? cover(distinct, post) : changed ? 0.3 : 1)
  }
  const ranked = Object.keys(sigs)
    .map((id) => ({ id, s: score(id) }))
    .sort((x, y) => y.s - x.s)
  const targets = new Set(targetIds.flatMap((t) => Object.keys(sigs).filter((id) => family(id) === family(t))))
  const bestTarget = ranked.find((r) => targets.has(r.id)) ?? null
  const bestOther = ranked.find((r) => !targets.has(r.id)) ?? null
  const ok =
    bestTarget !== null &&
    bestTarget.s >= 0.6 &&
    bestTarget.s >= (bestOther?.s ?? 0) - 0.05 &&
    (changed || bestTarget.s >= 0.95)
  return { bestTarget, bestOther, ok, score }
}

describe('the audit compares a dialog over another screen', () => {
  test('same_section_scores_the_whole_frame', () => {
    const sig: Sig = ['s:files', 'h:Files', 'b:New', 'b:Search names']
    const sigs = { 'DW-0401': sig }
    const pre = new Set(sig)
    const full = decide(['DW-0401'], sigs, pre, new Set(sig), false, 'DW-0401')
    expect(full.bestTarget?.s).toBe(1)
    expect(full.ok).toBe(true)

    const missingToolbar = sig.filter((t) => t !== 'b:Search names')
    const partial = decide(['DW-0401'], sigs, pre, new Set(missingToolbar), true, 'DW-0401')
    expect(partial.bestTarget?.s).toBeLessThan(1)
    expect(partial.bestTarget?.s).toBeLessThan(0.6)
  })

  test('another_section_scores_only_the_dialog_layer', () => {
    const target: Sig = ['s:files', 'h:Files', 'b:New', 'd:Details', 'dlg:d:Details', 'dlg:b:Close', 'dlg:st:1 item selected']
    const post = new Set(['s:shared', 'h:Shared', 'b:New', 'dlg:d:Details', 'dlg:b:Close', 'dlg:st:1 item selected'])
    const pre = new Set(['s:shared', 'h:Shared', 'b:New'])
    const sigs = { 'DW-0505': ['s:shared', 'h:Shared', 'b:New'], 'DW-0411': target }

    expect(isOverlaySection(SECTION_OF.DW-0505, SECTION_OF['DW-0411'])).toBe(true)
    expect(dialogLayer(target)).toHaveLength(3)

    const overlay = decide(['DW-0411'], sigs, pre, post, true, 'DW-0505')
    expect(overlay.bestTarget?.id).toBe('DW-0411')
    expect(overlay.bestTarget?.s).toBe(1)

    // The old whole-screen score would not reach the audit's 0.6 bar on this screen.
    expect(cover(target, post)).toBeLessThan(0.6)
    // The audit's score() uses this exact seam.
    expect(AUDIT).toMatch(
      /isOverlaySection\(SECTION_OF\[a\.source\], SECTION_OF\[id\]\)\s*\?\s*dialogLayer\(sigs\[id\]\)\s*:\s*sigs\[id\]/,
    )
  })

  test('a_dialog_over_the_wrong_screen_still_passes', () => {
    const target: Sig = ['s:files', 'h:Files', 'b:New', 'dlg:d:Move', 'dlg:b:Move here', 'dlg:st:Moving 2 files']
    // The right dialog over Shared: none of the target's background (files, Files, New) is on screen.
    const post = new Set(['s:shared', 'h:Shared', 'b:New', 'dlg:d:Move', 'dlg:b:Move here', 'dlg:st:Moving 2 files'])
    const pre = new Set(['s:shared', 'h:Shared', 'b:New'])
    const sigs = { 'DW-0505': ['s:shared', 'h:Shared', 'b:New'], 'DW-0411': target }

    const result = decide(['DW-0411'], sigs, pre, post, true, 'DW-0505')
    expect(result.bestTarget?.id).toBe('DW-0411')
    expect(result.bestTarget?.s).toBe(1)
    expect(result.ok).toBe(true)
  })

  test('the_right_screen_with_the_wrong_dialog_fails', () => {
    const target: Sig = ['s:files', 'h:Files', 'b:New', 'dlg:d:Move', 'dlg:b:Move here']
    // Every background token is there, one of the two dialog tokens is not: the rule must not pass on the screen alone.
    const post = new Set(['s:files', 'h:Files', 'b:New', 'dlg:d:Move'])
    const pre = new Set(['s:shared', 'h:Shared', 'b:New'])
    const sigs = { 'DW-0505': ['s:shared', 'h:Shared', 'b:New'], 'DW-0411': target }

    const result = decide(['DW-0411'], sigs, pre, post, true, 'DW-0505')
    expect(result.bestTarget?.s).toBe(0.5)
    expect(result.ok).toBe(false)
  })

  test('an_empty_dialog_layer_never_passes', () => {
    const target: Sig = ['s:files', 'h:Files', 'b:New']
    const post = new Set(target)
    const pre = new Set(['s:shared', 'h:Shared', 'b:New'])
    const sigs = { 'DW-0505': ['s:shared', 'h:Shared', 'b:New'], 'DW-0411': target }

    expect(dialogLayer(target)).toHaveLength(0)
    expect(cover(dialogLayer(target), post)).toBe(0)
    const result = decide(['DW-0411'], sigs, pre, post, true, 'DW-0505')
    expect(result.ok).toBe(false)
  })

  test('a_family_variant_still_counts', () => {
    const base: Sig = ['s:files', 'h:Files', 'b:New', 'b:Sort', 'sel:Name']
    const variant: Sig = ['s:files', 'h:Files', 'b:New', 'b:Sort', 'sel:Date']
    const pre = new Set(['s:shared', 'h:Shared', 'b:New'])
    const sigs = { 'DW-0407': [...pre], 'DW-0408': base, 'DW-0408b': variant }

    expect(family('DW-0408b')).toBe(family('DW-0408'))
    const result = decide(['DW-0408'], sigs, pre, new Set(variant), true, 'DW-0407')
    expect(result.bestTarget?.id).toBe('DW-0408b')
    expect(result.ok).toBe(true)
    // The family rule is untouched by the overlay change.
    expect(AUDIT).toContain("const family = (id) => id.replace(/[a-z]$/, '')")
  })
})
