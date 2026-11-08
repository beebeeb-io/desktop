/**
 * Lazy-loaded minimap (task 1563 design screen 02).
 *
 * `@replit/codemirror-minimap` is a maintained, MIT-licensed package from
 * the Replit/CodeMirror ecosystem (the same publishers ship
 * `@replit/codemirror-indentation-markers`, used alongside it — see
 * codemirror-editor.tsx) and its only runtime dependency is `crelt`, the
 * same tiny DOM-helper library `@codemirror/view` itself already depends on
 * — no new dependency surface beyond one already in the tree. CodeMirror 6
 * core ships no minimap of its own, and a correct one (canvas rendering of
 * syntax-highlighted text at scale, viewport-drag scrolling, diagnostics
 * gutters) is a real undertaking that a maintained, already-tested package
 * does better than a first cut here would.
 *
 * Loaded via a dynamic `import()` (not in the editor's main chunk) since a
 * minimap is a secondary affordance most edits of a short text/config file
 * never benefit from — they shouldn't pay for the canvas-rendering code.
 * Mirrors editor-langs.ts's per-language lazy-loading pattern.
 */
import type { Extension } from '@codemirror/state'

let cached: Promise<Extension> | null = null

export function loadMinimapExtension(): Promise<Extension> {
  if (!cached) {
    cached = import('@replit/codemirror-minimap').then(({ showMinimap }) =>
      showMinimap.of({
        create: () => ({ dom: document.createElement('div') }),
        displayText: 'characters',
        showOverlay: 'always',
      }),
    )
  }
  return cached
}
