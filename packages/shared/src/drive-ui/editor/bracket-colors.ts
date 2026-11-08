/**
 * Bracket-pair colourisation CodeMirror extension (task 1563 design screen
 * 02). Wraps the pure `computeBracketDepths` (lib/bracket-depth.ts) in a
 * `ViewPlugin` that decorates each bracket character with a class cycling
 * through 3 colours (`editor.css`'s `.cm-bracket-1/2/3`, themed from
 * `--ed-bracket-1/2/3`) by nesting depth, so a matching pair always shares
 * one colour.
 *
 * Full-document depths are recomputed only on a doc change (not on every
 * scroll); only the CURRENTLY VISIBLE range gets actual decorations, cheaply
 * refiltered from the cached depth array on viewport-only changes (a plain
 * scroll doesn't rescan the document).
 */
import { EditorView, Decoration, type DecorationSet, ViewPlugin, type ViewUpdate } from '@codemirror/view'
import { RangeSetBuilder } from '@codemirror/state'
import { computeBracketDepths, type BracketMark } from '../lib/bracket-depth'

const BRACKET_CLASSES = ['cm-bracket-1', 'cm-bracket-2', 'cm-bracket-3']

function buildVisibleDecorations(view: EditorView, marks: BracketMark[]): DecorationSet {
  const builder = new RangeSetBuilder<Decoration>()
  for (const { from, to } of view.visibleRanges) {
    for (const m of marks) {
      if (m.index < from) continue
      if (m.index >= to) break // `marks` is index-ascending — nothing further in this range
      builder.add(m.index, m.index + 1, Decoration.mark({ class: BRACKET_CLASSES[m.depth % 3] }))
    }
  }
  return builder.finish()
}

class BracketColorPlugin {
  marks: BracketMark[]
  decorations: DecorationSet

  constructor(view: EditorView) {
    this.marks = computeBracketDepths(view.state.doc.toString())
    this.decorations = buildVisibleDecorations(view, this.marks)
  }

  update(update: ViewUpdate) {
    if (update.docChanged) {
      this.marks = computeBracketDepths(update.state.doc.toString())
    }
    if (update.docChanged || update.viewportChanged) {
      this.decorations = buildVisibleDecorations(update.view, this.marks)
    }
  }
}

export const bracketColorization = ViewPlugin.fromClass(BracketColorPlugin, {
  decorations: (plugin) => plugin.decorations,
})
