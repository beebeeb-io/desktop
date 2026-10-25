/**
 * The Beebeeb window's title (task 1683 slice 2; spec C C-W8 and spec `docs/specs/2026-10-24-macos-two-surface-app.md`
 * §10.2 row 36): signed out it follows the heading of the card shown ("Sign in to Beebeeb", "Sign in again", …), signed
 * in it is "Beebeeb". The title bar is hidden (`hiddenTitle`), so this is what Mission Control, the Window menu and
 * VoiceOver read. Setting it needs `core:window:allow-set-title`, granted to `main` alone (`macos-main.json`).
 */
import { getCurrentWindow } from '@tauri-apps/api/window'

/** Best effort: a title that cannot be set is cosmetic, and never an error for the person. */
export async function setMainWindowTitle(title: string): Promise<void> {
  try {
    await getCurrentWindow().setTitle(title)
  } catch {
    // The window may be closing, or this is a browser render with no Tauri window.
  }
}
