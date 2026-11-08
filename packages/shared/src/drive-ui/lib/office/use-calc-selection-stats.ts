/**
 * Sum/Average/Count of the current Calc selection, live in the status bar
 * (task 1567, Calc lane) -- design/office-editor-shots/calc-*.png.
 *
 * KNOWN ENGINE GAP, verified against the REAL rebuilt engine artifact
 * (repos/office/evidence/artifacts/emscripten, phase4-2026-09-27), not
 * assumed -- see the dated note this lane appended to
 * .claude/tasks/in-development/1567-*.md for the full probe transcript:
 *   1. `bbOffice.onSelectionChange()` never fires for a Calc cell-range
 *      selection in this build (bb-office-worker.js's own
 *      doAddSelectionListener only extracts XTextRange text -- a Calc range
 *      isn't one; probed with a real keyboard-driven multi-cell selection,
 *      zero events arrived, not even an empty-text one).
 *   2. There is no bridge op to read a cell/range's value or address at all
 *      (the worker's full op table is open/save/insertImage/insertHyperlink/
 *      dispatch/onState/onModifiedChange/onSelectionChange/getOutline/
 *      goToHeading/setZoom/newDocument/setTheme -- nothing else).
 * Both gaps require a new bb-office-worker.js op to close properly (e.g. a
 * `getSelectionValues()` reading `XCellRangeData` directly) -- out of this
 * lane's scope (a different repo's runtime bridge, shared by every office
 * app, not this task's "new files only" grant).
 *
 * What DOES work, verified against the real engine: the ordinary product
 * command `.uno:Copy` really does put the selection's cell values on the
 * system clipboard, tab/newline-separated (confirmed with a real 3-cell
 * numeric range: clipboard read back as "10\n20\n30\n", and a
 * `.uno:GoToCell` + `.uno:Copy` round trip on a 3-row range). This hook uses
 * that as its read path, on a debounced pointerup/keyup inside the canvas:
 *   1. read whatever is on the clipboard right now (so it can be restored),
 *   2. dispatch `.uno:Copy`,
 *   3. read the copied values back and parse sum/average/count from them,
 *   4. restore the clipboard read in step 1, best-effort, in a finally --
 *      so a user's own last real copy is never left overwritten.
 * If ANY step fails (Clipboard permission denied, API missing in this
 * browser, engine error), it fails silently to "no stats" rather than
 * showing a wrong number -- an honest gap, matching this task's own
 * established convention for the other bridge gaps above.
 *
 * Tradeoff, stated plainly: this cannot be a passive, always-current readout
 * the way Excel/Sheets' native status bar is (no selection-change signal
 * exists to key off for Calc), so it only updates within `debounceMs` of the
 * user's last click/keypress inside the canvas, not on every intermediate
 * change (e.g. mid-drag).
 */
import { useEffect, useRef, useState } from 'react'
import type { OfficeBridge } from './bb-office-bridge'
import { parseClipboardStats, type CalcSelectionStats } from './calc-selection-stats'

export interface UseCalcSelectionStatsOptions {
  bridge: OfficeBridge | null
  /** The engine iframe's own `contentWindow` -- clipboard reads/writes are
   *  tried there FIRST (the realm the engine's own copy actually executes
   *  in), falling back to the host page's `window` if that's unavailable. */
  iframeWindow: Window | null | undefined
  enabled: boolean
  debounceMs?: number
}

type ClipboardCapableWindow = Window & {
  navigator: Navigator & {
    clipboard?: {
      readText(): Promise<string>
      writeText(text: string): Promise<void>
    }
  }
}

async function readClipboardText(...windows: Array<Window | null | undefined>): Promise<string | null> {
  for (const w of windows) {
    const clipboard = (w as ClipboardCapableWindow | null | undefined)?.navigator?.clipboard
    if (!clipboard?.readText) continue
    try {
      return await clipboard.readText()
    } catch {
      // Try the next candidate window before giving up entirely.
    }
  }
  return null
}

async function writeClipboardText(text: string, ...windows: Array<Window | null | undefined>): Promise<void> {
  for (const w of windows) {
    const clipboard = (w as ClipboardCapableWindow | null | undefined)?.navigator?.clipboard
    if (!clipboard?.writeText) continue
    try {
      await clipboard.writeText(text)
      return
    } catch {
      // Try the next candidate window.
    }
  }
}

const DEFAULT_DEBOUNCE_MS = 550

export function useCalcSelectionStats({ bridge, iframeWindow, enabled, debounceMs = DEFAULT_DEBOUNCE_MS }: UseCalcSelectionStatsOptions): CalcSelectionStats | null {
  const [stats, setStats] = useState<CalcSelectionStats | null>(null)
  const bridgeRef = useRef(bridge)
  bridgeRef.current = bridge
  const iframeWindowRef = useRef(iframeWindow)
  iframeWindowRef.current = iframeWindow
  const inFlightRef = useRef(false)

  useEffect(() => {
    if (!enabled) {
      setStats(null)
      return
    }
    const doc = iframeWindow?.document
    if (!doc) return

    let timer: ReturnType<typeof setTimeout> | null = null
    let cancelled = false

    async function computeNow() {
      const b = bridgeRef.current
      if (!b || inFlightRef.current) return
      inFlightRef.current = true
      const hostWindow = typeof window !== 'undefined' ? window : undefined
      const engineWindow = iframeWindowRef.current ?? undefined
      let original: string | null = null
      try {
        original = await readClipboardText(engineWindow, hostWindow)
        await b.dispatch('.uno:Copy')
        const copied = await readClipboardText(engineWindow, hostWindow)
        if (cancelled) return
        setStats(copied == null ? null : parseClipboardStats(copied))
      } catch {
        if (!cancelled) setStats(null)
      } finally {
        if (original != null) {
          try {
            await writeClipboardText(original, engineWindow, hostWindow)
          } catch {
            // Best-effort restore only -- never fail the stats computation
            // over a clipboard write we can't confirm.
          }
        }
        inFlightRef.current = false
      }
    }

    function schedule() {
      if (timer) clearTimeout(timer)
      timer = setTimeout(computeNow, debounceMs)
    }

    doc.addEventListener('pointerup', schedule)
    doc.addEventListener('keyup', schedule)
    // Fallback poll, found necessary by actually running this against the
    // real engine: the primary trigger above only fires for a selection made
    // by clicking/dragging DIRECTLY in the canvas (which is where real DOM
    // focus lands, so pointerup/keyup bubble to `doc`). Navigating via OUR
    // OWN formula-bar ref box (calc-formula-bar.tsx's `.uno:GoToCell`) moves
    // the ENGINE's selection without ever moving DOM focus into the iframe
    // (focus stays on that plain HTML `<input>`), so neither listener above
    // fires for it. Rather than wire a bespoke event bus between two
    // sibling components for this one case, a slow poll converges on both
    // paths honestly -- bounded by `inFlightRef`, so a slow clipboard round
    // trip is never piled on top of itself.
    const pollId = setInterval(computeNow, Math.max(debounceMs * 2, 1500))
    return () => {
      cancelled = true
      if (timer) clearTimeout(timer)
      clearInterval(pollId)
      doc.removeEventListener('pointerup', schedule)
      doc.removeEventListener('keyup', schedule)
    }
  }, [enabled, iframeWindow, debounceMs])

  return stats
}
