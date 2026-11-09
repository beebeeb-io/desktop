// The update button, per platform (lead ruling M3e item 1, 2026-11-09): Windows and Linux restart into the new version
// (`install_update` installs in place, as v1's UpdateBanner does on Windows); a Mac only downloads it (task 2176, the
// App Sandbox), with macUpdateCopy.ts as the source of its words and address. The flyout (DW-1210, DW-1214), General's
// banner (DW-0781) and the release notes (DW-0789b) take their label, line and action from here.
import { canInstallUpdate, type DesktopCapabilities } from '../capabilities'
import { command, type CommandResult } from '../desktopApi'
import { MACOS_DOWNLOAD_URL } from '../macUpdateCopy'
import { openExternal } from './external'
import { MAC_DOWNLOAD_COPY, RESTART_COPY, SITE_DOWNLOAD_COPY, type UpdateCopy } from './updateCopy'

/** How this build updates, in words. Unknown (a plain browser, the host not read yet) is the Windows app's: v2 is
 *  Windows first, and a Mac never mounts these windows. */
export function updateCopyFor(caps: DesktopCapabilities | null): UpdateCopy {
  if (!caps || canInstallUpdate(caps)) return RESTART_COPY
  return caps.host_os === 'macos' ? MAC_DOWNLOAD_COPY : SITE_DOWNLOAD_COPY
}

/** The install running in this window, if any: a second click joins it instead of starting another download. */
let installing: Promise<CommandResult<void>> | null = null

/**
 * Runs the update: restart into it, or open its download. Resolves with what happened, never with a fake success. An
 * install has no time limit: `install_update` answers with an error, or restarts the app into the new version, and a
 * button that said "couldn't" while the download went on would offer a second install.
 */
export async function runUpdate(copy: UpdateCopy, caps: DesktopCapabilities | null = null): Promise<CommandResult<void>> {
  if (copy.mode === 'download') return openExternal(caps?.host_os === 'macos' ? MACOS_DOWNLOAD_URL : 'https://beebeeb.io/download')
  installing ??= command<void>('install_update').finally(() => { installing = null })
  return installing
}
