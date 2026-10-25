/**
 * Moves keyboard focus to the element with this id, if the page has one. A no-op without a document (the tests) and when
 * the element is gone. Shared by the window (its sidebar and sheets) and the Settings sections (after a row they were
 * focused on is removed).
 */
export function focusById(id: string): void {
  if (typeof document === 'undefined') return
  document.getElementById(id)?.focus()
}
