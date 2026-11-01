/**
 * A button's accessible name when the same word repeats down a list (task 1683 slice 4, review of #142): "Decide…",
 * "Restore" and "Delete…" five times in a row tell a screen reader's user nothing. The visible word comes first, so the
 * name contains it (WCAG 2.5.3, label in name), then what the button acts on: "Restore, Draft 3.docx".
 */
export function actionNamedFor(label: string, subject: string): string {
  return `${label}, ${subject}`
}
