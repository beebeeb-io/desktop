/**
 * Magic-byte guard for the office editor (task 1584).
 *
 * The engine must only ever get decrypted bytes that really are the document
 * type the file claims to be. This is checked BEFORE the bytes go to the
 * engine, and a mismatch gets a clear error. It is never "opened anyway".
 * Two reasons:
 *   - If a decryption or chunk-assembly bug ever passed ciphertext through,
 *     it has to fail loudly here, not be handed to LibreOffice.
 *   - The user gets a sentence that says what is wrong. Handed non-zip bytes
 *     under a .docx name, this engine build returns no document at all, which
 *     the editor could only report as "Failed to open this document".
 *
 * Signatures:
 *   - OOXML (.docx/.xlsx/.pptx) and ODF (.odt/.ods/.odp) are zip packages:
 *     local file header `PK\x03\x04`.
 *   - Legacy .doc/.xls/.ppt are OLE2 compound files: `D0 CF 11 E0 A1 B1 1A E1`.
 *   - .rtf starts with `{\rtf`.
 *   - .csv is plain text and has no signature, so it is not checked.
 *
 * Pure. No DOM and no network, so it can be unit-tested
 * (test/1584-office-magic.test.ts).
 */

export type OfficeContainer = 'zip' | 'ole' | 'rtf'

const ZIP_MAGIC = [0x50, 0x4b, 0x03, 0x04]
const OLE_MAGIC = [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]
const RTF_MAGIC = [0x7b, 0x5c, 0x72, 0x74, 0x66] // "{\rtf"

const CONTAINER_BY_EXT: Record<string, OfficeContainer> = {
  docx: 'zip',
  xlsx: 'zip',
  pptx: 'zip',
  odt: 'zip',
  ods: 'zip',
  odp: 'zip',
  doc: 'ole',
  xls: 'ole',
  ppt: 'ole',
  rtf: 'rtf',
}

const LABEL_BY_EXT: Record<string, string> = {
  docx: 'Word document',
  doc: 'Word document',
  rtf: 'Word document',
  odt: 'OpenDocument text document',
  xlsx: 'Excel spreadsheet',
  xls: 'Excel spreadsheet',
  ods: 'OpenDocument spreadsheet',
  pptx: 'PowerPoint presentation',
  ppt: 'PowerPoint presentation',
  odp: 'OpenDocument presentation',
}

function startsWith(bytes: Uint8Array, magic: number[]): boolean {
  if (bytes.length < magic.length) return false
  for (let i = 0; i < magic.length; i++) {
    if (bytes[i] !== magic[i]) return false
  }
  return true
}

/** The container a file's bytes actually start with, or null if none of ours. */
export function sniffOfficeContainer(bytes: Uint8Array): OfficeContainer | null {
  if (startsWith(bytes, ZIP_MAGIC)) return 'zip'
  if (startsWith(bytes, OLE_MAGIC)) return 'ole'
  if (startsWith(bytes, RTF_MAGIC)) return 'rtf'
  return null
}

/**
 * The file's REAL extension (lowercase, no dot), or '' when it has none.
 * Use this, not resolveOfficeFileKind().ext, as checkOfficeBytes' input:
 * for a file matched only by its MIME type that helper synthesizes the
 * modern save extension (a legacy OLE .doc with no extension comes back as
 * "docx"), which would wrongly demand a zip (Codex review, PR #119).
 */
export function extensionOf(filename: string): string {
  const dot = filename.lastIndexOf('.')
  if (dot <= 0 || dot === filename.length - 1) return ''
  return filename.slice(dot + 1).toLowerCase()
}

export type OfficeBytesCheck =
  | { ok: true; container: OfficeContainer | null }
  | { ok: false; message: string; found: OfficeContainer | null; expected: OfficeContainer | null; firstBytesHex: string }

function hexPrefix(bytes: Uint8Array, n = 8): string {
  return Array.from(bytes.subarray(0, n))
    .map((b) => b.toString(16).padStart(2, '0'))
    .join(' ')
}

/**
 * Checks that `bytes` really are a document of the kind the extension `ext`
 * (lowercase, no dot, as resolveOfficeFileKind returns it) claims to be.
 *
 * - A known extension must match its own container exactly (a .docx must be
 *   a zip; an OLE file named .docx is refused).
 * - `csv` is not checked (plain text has no signature).
 * - Any other extension (the file resolved to an office app through its MIME
 *   type only) must at least be a zip or OLE container.
 */
export function checkOfficeBytes(bytes: Uint8Array, ext: string): OfficeBytesCheck {
  const found = sniffOfficeContainer(bytes)
  if (ext === 'csv') return { ok: true, container: found }
  const expected = CONTAINER_BY_EXT[ext] ?? null
  const matches = expected ? found === expected : found === 'zip' || found === 'ole'
  if (matches) return { ok: true, container: found }
  const label = LABEL_BY_EXT[ext] ?? 'office document'
  return {
    ok: false,
    message: `This file could not be decrypted or is not a valid ${label}.`,
    found,
    expected,
    firstBytesHex: hexPrefix(bytes),
  }
}
