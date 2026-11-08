/**
 * Shared markdown-link safety helper (task 1563 PR #103 review thread).
 *
 * Both the read-mode markdown renderer (markdown-preview.tsx) and the
 * editor's split-view live preview (file-editor.tsx) render user-controlled
 * markdown through `react-markdown`, whose DEFAULT `a` element is a plain
 * same-tab anchor. That's fine for read mode (nothing to lose), but inside
 * the editor a same-tab click on a link discards the in-memory draft without
 * ever going through the unsaved-changes guard — clicking navigates the
 * whole page away, unmounting the editor mid-edit.
 *
 * Pure function only — no DOM, no React — so it's unit-testable in
 * isolation. Only `http://`/`https://` targets are allowed through (matches
 * the existing read-mode behavior): `javascript:`, `data:`, `vault://`, etc.
 * are stripped, since they're not safe (or not resolvable) to open in a new
 * tab.
 */
export function resolveSafeMarkdownHref(href: string | undefined | null): string | undefined {
  if (href && /^https?:\/\//i.test(href)) return href
  return undefined
}
