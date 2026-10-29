/**
 * Task 2191 (spec D11, Decision (lead, 2026-10-26)): the Windows sign-out confirmation when
 * files from before the last update hold bytes on this PC. Beebeeb can't tell yet whether they
 * hold changes the server doesn't have, so the person chooses: keep copies (the default) or
 * delete them. The copies are ordinary files, and the dialog says so plainly.
 */
export interface UnknownFilesSignOutCopy {
  title: string
  body: string
  keepLabel: string
  keepNote: string
  deleteLabel: string
  deleteNote: string
  cancelLabel: string
}

export function unknownFilesSignOutCopy(count: number, folder: string): UnknownFilesSignOutCopy {
  const files = count === 1 ? '1 file' : `${count} files`
  const them = count === 1 ? 'it' : 'them'
  return {
    title: 'Files from before the last update',
    body: `${files} in your Beebeeb folder ${count === 1 ? 'was' : 'were'} on this PC before the last update. Beebeeb can’t tell yet whether ${count === 1 ? 'it holds' : 'they hold'} changes the server doesn’t have. Signing out removes ${them} from this PC.`,
    keepLabel: 'Keep copies and sign out',
    keepNote: `Copies go to ${folder}. They stay on this PC and aren’t encrypted, like any other file there.`,
    deleteLabel: 'Delete them and sign out',
    deleteNote: `Any change in ${them} that the server doesn’t have is lost. We can’t recover it.`,
    cancelLabel: 'Cancel',
  }
}
