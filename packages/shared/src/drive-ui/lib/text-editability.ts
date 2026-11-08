/**
 * Editability gate for the in-browser text/markdown/code editor (task 1563).
 *
 * v1 limits (design/editor-1563.html screen 06 "Limits"): a file is editable
 * only when it is UTF-8 text of at most MAX_EDITABLE_BYTES. Anything larger,
 * anything that looks like binary data, or anything that fails strict UTF-8
 * decoding opens read-only with a notice instead of the editor — never a
 * garbled or frozen editing surface.
 *
 * Pure functions only — no DOM, no network — so they can run as bun:test
 * unit tests without a browser.
 */

/** v1 editable-size ceiling (design: "UTF-8, ≤ 2 MB editable"). */
export const MAX_EDITABLE_BYTES = 2 * 1024 * 1024

export type EditabilityReason = 'ok' | 'too-large' | 'binary' | 'invalid-utf8'

export interface EditabilityResult {
  editable: boolean
  reason: EditabilityReason
}

/**
 * Sniffs the first 8 KB for binary content: a NUL byte (the same signal git
 * and most editors use), or a high density of non-printable control bytes
 * outside tab/LF/CR. Deliberately cheap — it only needs to catch obviously
 * non-text content (images, archives, database files, etc.) misrouted here
 * by extension/MIME, not to be a general-purpose file-type sniffer.
 */
export function looksBinary(bytes: Uint8Array): boolean {
  const sampleLen = Math.min(bytes.length, 8192)
  if (sampleLen === 0) return false
  let controlCount = 0
  for (let i = 0; i < sampleLen; i++) {
    const b = bytes[i]
    if (b === 0) return true
    // Tab(9), LF(10), CR(13) are normal in text. Everything else below
    // 0x20, plus DEL(127), counts as a control byte for the density check.
    if (b === 9 || b === 10 || b === 13) continue
    if (b < 0x20 || b === 0x7f) controlCount++
  }
  return controlCount / sampleLen > 0.3
}

/**
 * Strict UTF-8 validity check. Uses the platform's fatal-mode TextDecoder,
 * which rejects overlong encodings, lone/unpaired surrogates and truncated
 * multi-byte sequences — the same class of malformed input that would
 * otherwise silently replace or drop characters (and could corrupt the file
 * on save, since we save the DECODED string, not the original bytes).
 */
export function isValidUtf8(bytes: Uint8Array): boolean {
  try {
    new TextDecoder('utf-8', { fatal: true }).decode(bytes)
    return true
  } catch {
    return false
  }
}

/**
 * Full editability decision for a candidate text/markdown/code file's raw
 * bytes. Order matters: size is the cheapest check and the one the design
 * calls out with its own distinct notice, so it's checked first.
 */
export function checkEditability(bytes: Uint8Array): EditabilityResult {
  if (bytes.byteLength > MAX_EDITABLE_BYTES) {
    return { editable: false, reason: 'too-large' }
  }
  if (looksBinary(bytes)) {
    return { editable: false, reason: 'binary' }
  }
  if (!isValidUtf8(bytes)) {
    return { editable: false, reason: 'invalid-utf8' }
  }
  return { editable: true, reason: 'ok' }
}

/** Human copy for the read-only notice (design screen 06). */
export function editabilityNotice(reason: EditabilityReason): { title: string; body: string } {
  switch (reason) {
    case 'too-large':
      return {
        title: 'Read-only: this file is larger than 2 MB',
        body: 'Editing large files in the browser gets slow, so it opens read-only. Download it to edit the rest.',
      }
    case 'binary':
      return {
        title: 'This is not a text file',
        body: "It contains binary data, so it can't be shown or edited as text. It stays encrypted in your vault exactly as uploaded.",
      }
    case 'invalid-utf8':
      return {
        title: 'This file is not UTF-8 text',
        body: 'Its encoding is not supported by the editor, so it opens read-only. It stays encrypted in your vault exactly as uploaded.',
      }
    case 'ok':
      return { title: '', body: '' }
  }
}
