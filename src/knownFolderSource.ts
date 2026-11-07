// LD-0113 Amendment 9 (Guus, decision 2338, "Ask once"): a known folder OneDrive's Known Folder Move has moved is
// backed up from the folder the person chooses, asked once. The question's words come from the app (`question` on
// `KnownFolderStatus`); this is the answer.
import { command, type CommandResult } from './desktopApi'

/** Which folder a moved known folder is backed up from. */
export type KnownFolderSource = 'onedrive' | 'default'

/** Answer the one question about a folder OneDrive moved: back up OneDrive's folder or the default one. */
export function setKnownFolderSource(key: string, choice: KnownFolderSource): Promise<CommandResult<void>> {
  return command<void>('set_known_folder_source', { key, choice })
}
