// Dev only (batch M2), never bundled: one live state recipe per board frame (329; Mail Backup excluded). The recipes
// live per area in ./states-*.ts; this file only joins them and holds the shape.
import type { Step } from './driver'
import type { Reads } from './reads'
import { FILES_STATES } from './states-files'
import { DRIVE_STATES } from './states-drive'
import { WINDOW_STATES } from './states-windows'
import { SETTINGS_STATES } from './states-settings'

export type WindowKind = 'main-app' | 'tray' | 'onboarding'

export interface Recipe {
  /** The window the frame is drawn in; the main window when absent. */
  window?: WindowKind
  /** The route and the data, as the URL query: section, id, sub, scenario. */
  q?: string
  /** localStorage entries set before the window mounts (a folder's view, recent searches). */
  storage?: Record<string, unknown>
  /** Reads answered for this state over the board's (./reads.ts). */
  reads?: Reads
  /** One-shot host answers a step needs (./reads.ts `answerFrom`), consumed in order. */
  answers?: Record<string, unknown[]>
  /** A component's first UI state (../seed.ts), for a state no event reaches from a route. */
  seeds?: Record<string, unknown>
  /** Replayed through real DOM events once the window has drawn. */
  steps?: Step[]
  /** Windows draws this frame (a picker, the tray menu, File Explorer, the installer): no web view to open, so every
   *  action on it is not automatable. The route opens the window the frame sits over, as it is. */
  native?: string
  /** The live app cannot draw this frame's state yet: why (wiring for the next batch). The route still opens the nearest live state. */
  gap?: string
}

export const RECIPES: Record<string, Recipe> = {
  ...FILES_STATES,
  ...DRIVE_STATES,
  ...WINDOW_STATES,
  ...SETTINGS_STATES,
}
