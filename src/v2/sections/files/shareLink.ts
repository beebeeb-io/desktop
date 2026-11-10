// DW-0407's Copy link: the dialog's expiry and password go to Rust (one way), which makes the link, puts the full
// link (with its key) on the clipboard, and answers the display form only. Several items make one bundle link.
// The command's ok is the clipboard taking the link: only then does the toast say it was copied. The web's
// 60-second clear rule stays where it is (the full link never reaches this side, so only Rust can clear it).
import type { ToastMessage } from '../../chrome/Toast'
import { expiryHours, failureText, shareLinkCopy } from '../shared/api'

/** What the copy did, so a dialog can keep itself open and show the same words the toast shows. */
export interface CopyLinkOutcome { ok: boolean; text: string }

/** The command's own words for a clipboard that refused the link: say what to do, never "Copied". */
const CLIPBOARD_FAILED = 'Couldn’t copy the link. Check that this app can use the clipboard, then try again.'

export async function copyShareLink(
  ids: string[],
  choice: { expires: string; password: string | null },
  toast: (m: ToastMessage) => void,
): Promise<CopyLinkOutcome> {
  if (ids.length === 0) return { ok: false, text: 'Nothing to copy.' }
  const password = choice.password && choice.password.trim() ? choice.password : null
  const r = await shareLinkCopy(ids[0], { itemIds: ids.length > 1 ? ids : undefined, expiresHours: expiryHours(choice.expires), password })
  if (r.ok) {
    const text = `Link copied: ${r.value.display}`
    toast({ kind: 'done', text })
    return { ok: true, text }
  }
  const text = /^clipboard/.test(r.reason) ? CLIPBOARD_FAILED : failureText(r)
  toast({ kind: 'failed', text })
  return { ok: false, text }
}
