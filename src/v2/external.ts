// A page or app outside Beebeeb (the website, the email app, Windows Settings): opened by the OS through the opener
// plugin, never inside a v2 window, which would leave the app. A refusal comes back to say so; nothing falls back to
// window.open (a plain browser would open a page the audit can't follow, and Tauri would open a bare web view).
import { command, type CommandResult } from '../desktopApi'

export function openExternal(url: string): Promise<CommandResult<void>> {
  return command<void>('plugin:opener|open_url', { url })
}

/** An address the OS opens rather than the window: the web, mail, and Windows' own settings pages. */
export function isExternal(href: string): boolean {
  return /^(https?:|mailto:|ms-settings:)/i.test(href)
}
