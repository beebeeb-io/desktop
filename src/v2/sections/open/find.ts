// Who takes Ctrl+F inside Open (DW-1505, DW-1523). A view with a find of its own takes it: the PDF's find bar, the
// text editor's find panel. With none, Ctrl+F is Search, as everywhere else in the window (V2App leaves Find to this
// section). An editor's route never leaves for Search: that would unmount the editor and drop its unsaved edit with
// no alert (desktop #273, finding 14).
import { useCallback, useEffect } from 'react'
import { useCommand } from '../../shortcuts'

/** How many mounted views own Ctrl+F. */
let owners = 0

/** A view that owns Ctrl+F while `own` is true; `handler` opens its find. */
export function useOwnFind(own: boolean, handler: () => void) {
  useEffect(() => {
    if (!own) return
    owners += 1
    return () => { owners -= 1 }
  }, [own])
  useCommand('find', useCallback(() => { if (own) handler() }, [own, handler]))
}

/** The Open routes that hold an edit: the text editor, Office (a file or a new document). */
const EDITING = new Set(['edit', 'office', 'new-document'])

/** Where Ctrl+F goes from Open: a mounted view's own find, nowhere (an editor without one: Office's engine takes the
 *  key itself while it has focus), or Search. */
export function findGoesTo(sub: string | null | undefined, ownerCount: number = owners): 'own' | 'stay' | 'search' {
  if (ownerCount > 0) return 'own'
  if (sub && EDITING.has(sub)) return 'stay'
  return 'search'
}
