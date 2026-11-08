/**
 * CodeMirror 6 theme extension for the Beebeeb editor (task 1563).
 *
 * Parts:
 *  1. `EditorView.theme` — editor chrome (background, gutters, cursor,
 *     selection, search panel). Every color value is a CSS `var(--...)`
 *     reference into `editor.css` / the app's `index.css` tokens, so the
 *     SAME extension instance serves both light and dark: the browser
 *     re-resolves the custom property when `.dark` toggles on `<html>`,
 *     with no JS-side dark-mode branch needed.
 *  2. `syntaxHighlighting(classHighlighter)` — assigns the standard
 *     `@lezer/highlight` class names (`tok-keyword`, `tok-string`, …),
 *     which `editor.css` maps to the design's token hues. Covers
 *     programming-language tags (keyword/string/number/function/typeName/…)
 *     AND the markdown tags it DOES map (link/heading/emphasis/strong).
 *  3. `bbMarkdownProseHighlighting` — a SEPARATE `HighlightStyle` that fills
 *     the gap `classHighlighter` leaves for markdown: `tags.list`,
 *     `tags.quote`, `tags.monospace` (inline code / code fences),
 *     `tags.processingInstruction` (the `#`/`-`/`>`/`*`/`` ` `` mark
 *     characters themselves), `tags.contentSeparator` (`---`), and
 *     `tags.strikethrough` (GFM `~~text~~`) are all subtags of `content` (or
 *     `meta`), and NONE of them appear in `classHighlighter`'s fixed table —
 *     so with only part 2, a markdown file rendered with almost no colour
 *     beyond the (explicitly-mapped) heading/emphasis/strong/link, task
 *     1563 PR #103 review). CodeMirror supports several active
 *     `syntaxHighlighting()` extensions at once (each contributes its own
 *     classes; the renderer unions them per token), so this is purely
 *     ADDITIVE — it never touches a tag `classHighlighter` already owns, so
 *     existing code-file highlighting (TS/JSON/etc.) is unchanged.
 *  4. Bracket-pair colourisation (`bracketColorization`, in
 *     `bracket-colors.ts`) and the two `@replit/codemirror-*` maintained
 *     packages (indentation guides, minimap) are wired in from
 *     `editor-langs.ts`/`codemirror-editor.tsx`; this file only supplies
 *     their CSS-variable theming (`.bb-editor` in editor.css) so they follow
 *     the same light/dark tokens as everything else.
 */

import { EditorView } from '@codemirror/view'
import { HighlightStyle, syntaxHighlighting } from '@codemirror/language'
import { classHighlighter, tags } from '@lezer/highlight'
import type { Extension } from '@codemirror/state'

export const bbEditorTheme: Extension = EditorView.theme({
  '&': {
    backgroundColor: 'var(--ed-bg)',
    color: 'var(--color-ink)',
  },
  '.cm-content': {
    caretColor: 'var(--ed-cursor)',
  },
})

export const bbSyntaxHighlighting: Extension = syntaxHighlighting(classHighlighter)

/** Markdown prose tags `classHighlighter` has no entry for at all — see the
 *  file header. Values reference the SAME `--ed-*` custom properties as
 *  editor.css's `.tok-*` rules, so light/dark theming is automatic. */
const markdownProseHighlightStyle = HighlightStyle.define([
  { tag: tags.list, color: 'var(--ed-property)' },
  { tag: tags.quote, color: 'var(--ed-comment)', fontStyle: 'italic' },
  {
    tag: tags.monospace,
    color: 'var(--ed-string)',
    backgroundColor: 'var(--ed-inline-code-bg)',
    borderRadius: '3px',
  },
  { tag: tags.processingInstruction, color: 'var(--ed-punctuation)' },
  { tag: tags.contentSeparator, color: 'var(--ed-punctuation)' },
  { tag: tags.character, color: 'var(--ed-string)' },
  { tag: tags.escape, color: 'var(--ed-punctuation)' },
  { tag: tags.strikethrough, color: 'var(--ed-comment)', textDecoration: 'line-through' },
])

export const bbMarkdownProseHighlighting: Extension = syntaxHighlighting(
  markdownProseHighlightStyle,
)

export const bbEditorExtensions: Extension[] = [
  bbEditorTheme,
  bbSyntaxHighlighting,
  bbMarkdownProseHighlighting,
]
