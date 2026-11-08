import type { ReactElement } from 'react'

/**
 * One entry per board frame a batch builds: `?frame=DW-0401` renders it (headless screenshots, one per frame).
 * The render function returns the whole window state for that frame: the shell with a route and a
 * fixtures scenario, or a bare surface (the flyout, a dialog, a first-run step).
 */
export type FrameRegistry = Record<string, () => ReactElement>
