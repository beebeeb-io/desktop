/**
 * Impress ribbon + filmstrip command tables and pure state-derivation
 * helpers (task 1567, Impress lane). Mirrors the Writer-side split already
 * established in `ribbon-commands.ts`: this module owns no DOM, no
 * `bbOffice` — only data and pure functions, so it is fully unit-testable in
 * isolation (same pattern `office-conflict.ts` / `text-editability.ts` use).
 *
 * Every `.uno:` command referenced below was verified dispatchable against
 * the REAL engine artifact before being wired into any UI (task 1567
 * Impress lane probes, 2026-09-27, against
 * repos/office/evidence/artifacts/emscripten via a throwaway local server —
 * not guessed from upstream LibreOffice documentation alone):
 *   - `.uno:InsertPage` / `.uno:DuplicatePage` / `.uno:DeletePage`: dispatch
 *     cleanly and the slide count visibly changes (confirmed via
 *     `.uno:PageStatus`'s own status text going 1→2→3→2 across
 *     Insert/Duplicate/Delete in one probe run).
 *   - `.uno:MovePageUp` / `.uno:MovePageDown`: dispatch cleanly (reorder
 *     confirmed via the surrounding probe's page-count bookkeeping; this
 *     build has no per-slide title readout to assert exact ordering by
 *     content, only by count/position — see `parseSlideStatus`'s own note).
 *   - `.uno:FirstPage` / `.uno:PreviousPage` / `.uno:NextPage` /
 *     `.uno:LastPage`: dispatch cleanly; `.uno:PreviousPage` was directly
 *     observed changing `.uno:PageStatus` from "Slide 3 of 3" to
 *     "Slide 2 of 3". `.uno:GoToPage` also dispatches without error but 4
 *     plausible argument names (Page/PageNumber/Number/Nr) all failed to
 *     move the cursor in 4 real attempts — no direct "jump to slide N" was
 *     found, so `planNavigation` below reaches an arbitrary target slide by
 *     stepping with Next/PreviousPage instead (see its own doc comment).
 *   - `.uno:Presentation`: dispatches cleanly (`{dispatched:true}`).
 *     `.uno:Escape` does NOT have a dispatch handler when tried against the
 *     document's own frame outside of an actually-focused running
 *     slideshow (`bbOffice.dispatch: no dispatch handler for .uno:Escape`)
 *     — `ImpressPresentOverlay`'s own comment documents how exit is handled
 *     without relying on that call succeeding.
 *   - `.uno:AssignLayout` with a `WhatLayout` argument dispatches cleanly —
 *     used for the Design tab's layout picker.
 *
 * KNOWN ENGINE GAP, not fixed here (out of this lane's scope — the engine
 * itself lives in a separate repo/build lane this task's brief does not
 * authorize touching): `bbOffice.insertImage()` unconditionally throws for
 * a non-Writer document (`bb-office-worker.js`'s `doInsertImage` hard-codes
 * `css.text.XTextDocument.query(state.model)` and throws "current document
 * is not a text document" otherwise — confirmed by probing it directly
 * against a fresh Impress document: threw). The Impress ribbon still wires
 * its Insert-image button to the real call and surfaces the real failure
 * honestly (see office-editor.tsx's `imageInsertError` handling) rather than
 * hiding the affordance or silently swallowing the rejection.
 *
 * ALSO NOT FOUND (flagged, not faked): no `.uno:` command or bridge call
 * exposes per-slide title/body TEXT or a real thumbnail bitmap for the
 * filmstrip — only the aggregate "Slide N of M" status string
 * (`.uno:PageStatus`). `ImpressFilmstrip` therefore renders a generic
 * schematic tile per slide (numbered, not a content preview) rather than
 * pretending to show real per-slide content it cannot read.
 */

import type { RibbonCommandDef } from './ribbon-commands'

/** Impress Home tab: the same text-formatting primitives Writer's ribbon
 *  uses (Bold/Italic/Underline dispatch identically against a shape's own
 *  text run in this engine — the FeatureStateEvent-based toggle mechanism
 *  is app-agnostic), plus Undo/Redo and one alignment command matching the
 *  approved mockup's Home row. */
export const IMPRESS_HOME_COMMANDS: RibbonCommandDef[] = [
  { id: 'undo', label: 'Undo', command: '.uno:Undo', shortcut: '⌘Z', group: 'Edit' },
  { id: 'redo', label: 'Redo', command: '.uno:Redo', shortcut: '⌘⇧Z', group: 'Edit' },
  { id: 'bold', label: 'Bold', command: '.uno:Bold', shortcut: '⌘B', group: 'Format' },
  { id: 'italic', label: 'Italic', command: '.uno:Italic', shortcut: '⌘I', group: 'Format' },
  { id: 'underline', label: 'Underline', command: '.uno:Underline', shortcut: '⌘U', group: 'Format' },
  { id: 'align-center', label: 'Align center', command: '.uno:CenterPara', shortcut: '⌘E', group: 'Format' },
]

export type SlideOp = 'insert' | 'duplicate' | 'delete' | 'moveUp' | 'moveDown'

/** Slide-management commands the ribbon's Home tab AND the filmstrip both
 *  drive — one definition, two call sites, so their `.uno:` command strings
 *  can never drift apart. */
export const IMPRESS_SLIDE_COMMANDS: Record<SlideOp, RibbonCommandDef> = {
  insert: { id: 'slide-insert', label: 'New slide', command: '.uno:InsertPage', shortcut: '⌘M', group: 'Insert' },
  duplicate: { id: 'slide-duplicate', label: 'Duplicate slide', command: '.uno:DuplicatePage', group: 'Insert' },
  delete: { id: 'slide-delete', label: 'Delete slide', command: '.uno:DeletePage', group: 'Document' },
  moveUp: { id: 'slide-move-up', label: 'Move slide up', command: '.uno:MovePageUp', group: 'Document' },
  moveDown: { id: 'slide-move-down', label: 'Move slide down', command: '.uno:MovePageDown', group: 'Document' },
}

/** The Present button — full-screen playback inside the page (task brief:
 *  "no external windows"). Not a toggle command (no FeatureStateEvent), so
 *  the ribbon never shows it as "pressed" — presenting/not-presenting is
 *  purely local React state driven by ImpressPresentOverlay. */
export const PRESENT_COMMAND: RibbonCommandDef = {
  id: 'present',
  label: 'Present',
  command: '.uno:Presentation',
  shortcut: '⌘⏎',
  group: 'Document',
}

/** Design tab: autolayout picker. `whatLayout` ordinals match LibreOffice
 *  Impress's own AutoLayout enum (upstream `sd/inc/pres.hxx`); verified only
 *  as "dispatches without error" against this build (task note above) — no
 *  bridge call exists yet to read back which layout is currently applied,
 *  so (unlike Bold/Italic) these buttons cannot show a real "pressed" state
 *  and deliberately don't claim one. */
export interface LayoutDef {
  id: string
  label: string
  whatLayout: number
}
export const IMPRESS_LAYOUTS: LayoutDef[] = [
  { id: 'layout-title', label: 'Title slide', whatLayout: 0 },
  { id: 'layout-title-content', label: 'Title, content', whatLayout: 1 },
  { id: 'layout-title-two-content', label: 'Title, two content', whatLayout: 3 },
  { id: 'layout-title-only', label: 'Title only', whatLayout: 19 },
  { id: 'layout-blank', label: 'Blank', whatLayout: 20 },
]

export function layoutDispatchArgs(layout: LayoutDef): RibbonCommandDef['args'] {
  return [{ name: 'WhatLayout', value: layout.whatLayout }]
}

/**
 * Parses the engine's `.uno:PageStatus` status text ("Slide 3 of 5") into a
 * structured `{index, count}` — the ONLY slide-count/position signal this
 * bridge exposes (no per-slide title/thumbnail read API — see this module's
 * header). Returns null for any shape it doesn't recognise rather than
 * throwing, so a future engine wording change degrades to "no status
 * shown" instead of crashing the ribbon/filmstrip.
 */
export function parseSlideStatus(state: unknown): { index: number; count: number } | null {
  if (typeof state !== 'string') return null
  const m = /^Slide (\d+) of (\d+)$/.exec(state)
  if (!m) return null
  const index = Number(m[1])
  const count = Number(m[2])
  if (!Number.isFinite(index) || !Number.isFinite(count)) return null
  if (index < 1 || count < 1 || index > count) return null
  return { index, count }
}

/** 1-indexed slide numbers for a given count, for the filmstrip to map over
 *  (no per-slide identity beyond position exists via this bridge). */
export function slideIndices(count: number): number[] {
  if (count < 1) return []
  return Array.from({ length: count }, (_, i) => i + 1)
}

/** Clamps a possibly-stale index into the current [1, count] range — used
 *  while a slide-mutating dispatch's OWN `.uno:PageStatus` event hasn't
 *  arrived yet (this engine's status events were observed, empirically, to
 *  lag one dispatch cycle behind the promise that triggered them — the same
 *  category of gap already documented for onSelectionChange/theming
 *  elsewhere in this task). Never throws on an empty document. */
export function clampSlideIndex(index: number, count: number): number {
  if (count < 1) return 0
  return Math.min(Math.max(index, 1), count)
}

export function canMoveUp(index: number): boolean {
  return index > 1
}
export function canMoveDown(index: number, count: number): boolean {
  return index > 0 && index < count
}
/** Product-level safety guard (never confirmed against the engine itself —
 *  no probe attempted deleting the last remaining slide, since doing so
 *  against a shared probe session risked leaving it in an unrecoverable
 *  state for no verifiable benefit): disable Delete once only one slide is
 *  left, rather than find out live what an empty deck does. */
export function canDelete(count: number): boolean {
  return count > 1
}

/**
 * No direct "jump to slide N" command was found (see this module's header —
 * 4 real `.uno:GoToPage` argument-name attempts all no-op). Reaches an
 * arbitrary target slide by stepping with the PROVEN Next/PreviousPage
 * commands instead. Returns the list of `.uno:` commands to dispatch, in
 * order (the caller awaits each one before firing the next — the engine's
 * own status events lag a dispatch, so firing all of them without awaiting
 * risks the LAST step landing on a stale current-position read).
 */
export function planNavigation(from: number, to: number): string[] {
  if (from < 1 || to < 1 || from === to) return []
  const step = to > from ? '.uno:NextPage' : '.uno:PreviousPage'
  return Array(Math.abs(to - from)).fill(step)
}
