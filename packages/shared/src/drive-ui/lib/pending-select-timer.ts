/**
 * Task 1565 (web #107) added a 300ms debounce to the non-previewable
 * single-click → `onSelectFile` side effect in file-list.tsx's row click
 * handler, to stop a synchronous select from mounting FileDetailsPanel's
 * full-viewport click-to-close backdrop in the gap between a double-click's
 * two native 'click' events (see file-list.tsx's own comment on
 * `pendingSelectTimerRef` for the full story of that bug).
 *
 * That fix only cleared the timer from the SAME row's own `onDoubleClick`
 * handler. Every OTHER control-flow path that can follow a non-previewable
 * single click within the 300ms window left the stale timer armed:
 *
 *   - a plain single click on a folder (navigates immediately, never
 *     touched the timer)
 *   - a plain single click on a previewable file (opens the preview
 *     immediately, never touched the timer)
 *   - a shift-click range-select or a Cmd/Ctrl-click toggle-select
 *   - keyboard activation (Enter) of a different row
 *
 * In every one of those cases the stale timer still fired ~300ms after the
 * ORIGINAL click and called `onSelectFile` for that first, now-irrelevant
 * file — popping its details panel open over whatever the user had since
 * navigated to or opened. A double-click's own `onDoubleClick` already
 * clears unconditionally (so double-clicking the SAME row, or double-
 * clicking a DIFFERENT row fast enough for the native 'dblclick' to land
 * before 300ms elapses, was already safe) — the gap was specifically the
 * single-click and keyboard branches above.
 *
 * Fix: every entry point that represents "the user is now doing something
 * else with this list" clears the pending timer FIRST, unconditionally,
 * before deciding what that something else is. Only the one case that
 * legitimately wants a timer — a single click on a non-previewable file —
 * arms a new one.
 *
 * This module is deliberately framework-free (no React, no DOM) so it can be
 * unit-tested directly: this repo's `bun test` harness has no
 * @testing-library/react / jsdom (see
 * test/1518-billing-reset-shared-handler.test.ts's header comment).
 * file-list.tsx wires a `useRef<PendingSelectTimerRef['current']>(null)`
 * into a `PendingSelectTimerRef` object and calls these functions from its
 * row click / dblclick / keydown handlers and its unmount effect.
 */

export interface PendingSelectTimerRef {
  current: ReturnType<typeof setTimeout> | null
}

export function createPendingSelectTimerRef(): PendingSelectTimerRef {
  return { current: null }
}

export const SELECT_DEBOUNCE_MS = 300

/**
 * Clears any pending debounced select timer. Idempotent — safe to call
 * unconditionally from every row-interaction entry point (click, dblclick,
 * keydown) and from the component's unmount cleanup.
 */
export function clearPendingSelectTimer(ref: PendingSelectTimerRef): void {
  if (ref.current !== null) {
    clearTimeout(ref.current)
    ref.current = null
  }
}

/**
 * Capture-phase entry point for EVERY click / context-menu gesture that
 * lands anywhere inside a row, wired as the row's `onClickCapture` (and
 * called first thing in its `onContextMenu`).
 *
 * Why capture: the row's nested controls — the selection checkbox, the
 * inline-rename input, the star toggle, the share badge(s), the
 * encryption-details lock button, the row-actions "more" button and any
 * caller-supplied `renderActions` control — all call `stopPropagation()` in
 * their bubble-phase onClick, so a click on one of them never reaches the
 * row's own bubble-phase onClick (`handleRowClick`). Before this hook, a
 * non-previewable click that armed the timer followed within 300ms by a
 * click on any of those controls left the timer armed, and it then opened
 * the stale file's details panel over the user's newer action (Codex P2 on
 * web #114). Capture runs on the row BEFORE any descendant's handler, so it
 * covers every current and future nested control without each one having to
 * remember to clear the timer.
 *
 * The legitimate case is unaffected: for a plain click on the row body the
 * capture clears, then the bubble-phase `handleRowClick` re-arms.
 */
export function handleRowInteractionCapture(ref: PendingSelectTimerRef): void {
  clearPendingSelectTimer(ref)
}

export interface RowClickInput {
  /** Shift+click extending the last selection range. */
  isShiftRange: boolean
  /** Cmd/Ctrl-click toggling this row into the multi-selection. */
  isModKey: boolean
  isFolder: boolean
  isPreviewable: boolean
}

export interface RowClickActions {
  rangeSelect: () => void
  toggleSelect: () => void
  navigateFolder: () => void
  preview: () => void
  selectFile: () => void
}

/**
 * The single decision point for a row's plain click (no drag). ALWAYS
 * clears any timer left over from a PREVIOUS click first — only the final
 * non-previewable-file branch re-arms one.
 */
export function handleRowClick(
  ref: PendingSelectTimerRef,
  input: RowClickInput,
  actions: RowClickActions,
  delayMs: number = SELECT_DEBOUNCE_MS,
): void {
  clearPendingSelectTimer(ref)

  if (input.isShiftRange) {
    actions.rangeSelect()
    return
  }
  if (input.isModKey) {
    actions.toggleSelect()
    return
  }
  if (input.isFolder) {
    actions.navigateFolder()
    return
  }
  if (input.isPreviewable) {
    actions.preview()
    return
  }
  ref.current = setTimeout(() => {
    ref.current = null
    actions.selectFile()
  }, delayMs)
}

export interface RowDoubleClickInput {
  isFolder: boolean
  isPreviewable: boolean
}

export interface RowDoubleClickActions {
  navigateFolder: () => void
  preview: () => void
  cannotPreviewToast: () => void
}

/**
 * A real 'dblclick' landed on this row — always clears the pending timer
 * (whichever row's click scheduled it, not just this row's) before deciding
 * what the double-click itself does.
 */
export function handleRowDoubleClick(
  ref: PendingSelectTimerRef,
  input: RowDoubleClickInput,
  actions: RowDoubleClickActions,
): void {
  clearPendingSelectTimer(ref)

  if (input.isFolder) {
    actions.navigateFolder()
    return
  }
  if (input.isPreviewable) {
    actions.preview()
    return
  }
  actions.cannotPreviewToast()
}
