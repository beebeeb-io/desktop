// DW-0407's Copy link: the dialog's expiry and password go to Rust (one way), which makes the link, puts the full
// link (with its key) on the clipboard, and answers the display form only. Several items make one bundle link.
import type { ToastMessage } from '../../chrome/Toast'
import { expiryHours, failureText, shareLinkCopy } from '../shared/api'

export async function copyShareLink(
  ids: string[],
  choice: { expires: string; password: string | null },
  toast: (m: ToastMessage) => void,
): Promise<void> {
  if (ids.length === 0) return
  const password = choice.password && choice.password.trim() ? choice.password : null
  const r = await shareLinkCopy(ids[0], { itemIds: ids.length > 1 ? ids : undefined, expiresHours: expiryHours(choice.expires), password })
  toast(r.ok ? { kind: 'done', text: `Link copied: ${r.value.display}` } : { kind: 'failed', text: failureText(r) })
}
