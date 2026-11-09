// The update button's words, per platform (lead ruling M3e item 1, 2026-11-09). Pure, so the flyout's model can take
// them: Windows and Linux restart into the new version; a Mac only downloads it, in macUpdateCopy.ts's words (task 2176).
import { UPDATE_DOWNLOAD_HINT, UPDATE_DOWNLOAD_LABEL } from '../macUpdateCopy'

export type UpdateMode = 'restart' | 'download'
export interface UpdateCopy { mode: UpdateMode; label: string; line: string }

export const RESTART_COPY: UpdateCopy = { mode: 'restart', label: 'Restart to update', line: 'Beebeeb closes, installs it and opens again.' }
export const MAC_DOWNLOAD_COPY: UpdateCopy = { mode: 'download', label: UPDATE_DOWNLOAD_LABEL, line: UPDATE_DOWNLOAD_HINT }
/** A build that can't install in place and isn't a Mac (a package-manager Linux install): the installer from the site. */
export const SITE_DOWNLOAD_COPY: UpdateCopy = { mode: 'download', label: UPDATE_DOWNLOAD_LABEL, line: 'Download the installer from beebeeb.io, then open it.' }
