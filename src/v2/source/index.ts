import type { DriveSource } from '@beebeeb/shared/drive-ui'
import { board } from '../board'
import { createDesktopSource } from './desktop'

/** Dev server only: the v2 fixture host (../dev/host.ts) marks the window with this key; it stands in for Rust behind `invoke`. */
export const DEV_HOST_KEY = '__bbV2DevHost'

/** True inside the Tauri webview. A plain browser (headless screenshots, `bun run dev`) has no IPC bridge. */
export function hasTauri(): boolean {
  if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) return false
  // The fixture host in a plain browser is not Tauri.
  return !(import.meta.env.DEV && DEV_HOST_KEY in window)
}

/** The board's sources, for a window with no host: registered by the dev server (../dev/live.ts, through ../board.ts). */
export interface BoardSources {
  source: (search: string) => DriveSource
}

/**
 * On the dev server, `?source=fixtures` and a plain browser get the board's fixtures (./fixtures/board.ts, the one set
 * the frames render too, registered by ../dev/live.ts), so every frame renders headless. Inside Tauri, and in a
 * production build, the desktop source: a plain browser with no host then gets its honest "unavailable" answers, never
 * the board's example data (P1c).
 */
export function pickSource(search: string = window.location.search): DriveSource {
  const params = new URLSearchParams(search)
  const wantsBoard = !hasTauri() || (import.meta.env.DEV && params.get('source') === 'fixtures')
  const fromBoard = wantsBoard ? board<BoardSources['source']>('source') : undefined
  return fromBoard ? fromBoard(search) : createDesktopSource()
}
