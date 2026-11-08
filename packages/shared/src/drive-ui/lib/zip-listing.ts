/**
 * zip-listing — task 1574: a honest file LISTING for `.zip` archives on web
 * (names, sizes, folders), read from the archive's own Central Directory
 * only. No entry is ever decompressed or extracted — this never needs to
 * read the archive's actual (potentially huge, arbitrarily compressed)
 * file data, only its own small directory structure, which every valid ZIP
 * keeps at the END of the file specifically so a reader can do exactly
 * this without scanning the whole archive (this is how Finder/Explorer/
 * `unzip -l` all list a zip's contents cheaply).
 *
 * Bounded reads WITHIN THIS FILE, by design, matching this task's RAW/TIFF
 * work:
 *  - only the last `EOCD_SEARCH_WINDOW_BYTES` bytes are read to locate the
 *    End Of Central Directory (EOCD) record (max possible: a 22-byte EOCD
 *    plus a 65535-byte comment — the format's own hard ceiling, not a
 *    number picked here);
 *  - only `MAX_CENTRAL_DIRECTORY_BYTES` of the Central Directory itself are
 *    ever read, however large the archive's own declared CD is — see
 *    `readZipListing`'s doc comment for what happens when a CD is bigger
 *    than that (an honest partial listing, never an unbounded read);
 *  - the archive's actual entry DATA (the potentially gigabyte-scale
 *    compressed payload the CD points at) is never read at all.
 *
 * NOT bounded — a real gap, stated rather than silently left implied
 * (Codex review, task 1574 gate, 2026-09-27): `readZipListing` receives an
 * already-fully-downloaded-and-decrypted `Blob` — its caller
 * (`file-preview.tsx`) has no thumbnail-first-style fast path for archives
 * (there's nothing image-like to thumbnail), so opening ANY `.zip` preview,
 * however large, downloads and decrypts the ENTIRE ciphertext into memory
 * BEFORE this file's own bounded `blob.slice()` calls ever run. The
 * "bounded reads" above are real and true of THIS reader once it has a
 * blob in hand; they do not bound the overall preview flow's network or
 * memory use for a multi-gigabyte archive. This is not new to this task —
 * every OTHER unsupported/download-card preview type in this codebase
 * (PPTX, legacy .doc/.xls, RAW with no embedded preview) has the identical
 * characteristic: none of them have a lazy/range-based download path
 * either. Fixing it for real would mean chunk/range-based partial
 * decryption wired into the preview flow — new cross-cutting
 * infrastructure, not a small addition to this file, and out of this
 * task's scope; flagged for its own backlog task rather than fixed here
 * or silently left for the doc comment above to overclaim past.
 *
 * Split pure/impure the same way as `raw-embedded-jpeg.ts` /
 * `raw-preview.worker.ts`: the byte-level EOCD/Central-Directory parsers
 * below are pure and unit-tested directly against synthetic ZIP bytes;
 * `readZipListing` is the one function that actually touches a `Blob`.
 *
 * ZIP64 (needed for an archive whose CD, entry count, or CD offset doesn't
 * fit a 32-bit field — very large or very many-entry archives) is
 * DELIBERATELY not implemented: detected and reported as "not supported"
 * rather than half-parsed with wrong offsets. A real ZIP64 reader is a
 * meaningfully sized second format spec, not a small addition to this one,
 * and this task's own fixture set (and the overwhelming majority of
 * real-world user uploads) never needs it — see this task's Notes for the
 * explicit call to leave it there rather than build a throwaway partial
 * implementation of it.
 */

export interface ZipEntryInfo {
  /** Full path within the archive, as stored (e.g. "photos/2024/img.jpg"). */
  name: string
  isDirectory: boolean
  compressedSize: number
  uncompressedSize: number
}

export interface ZipListing {
  entries: ZipEntryInfo[]
  /** Total entry count the archive's own EOCD record declares — may be
   *  larger than `entries.length` when the listing was truncated. */
  totalEntries: number
  /** True when `entries` is not the complete listing — either the Central
   *  Directory itself was larger than this reader will read
   *  (`MAX_CENTRAL_DIRECTORY_BYTES`), or the parsed entries were capped for
   *  display (`MAX_LISTED_ENTRIES`). */
  truncated: boolean
}

const EOCD_SIGNATURE = 0x06054b50
const EOCD_FIXED_SIZE = 22
/** The ZIP format's own hard ceiling on an EOCD comment (a 16-bit length
 * field) — this is what bounds the tail-search window, not an arbitrary
 * choice. */
const MAX_EOCD_COMMENT_BYTES = 65535
export const EOCD_SEARCH_WINDOW_BYTES = EOCD_FIXED_SIZE + MAX_EOCD_COMMENT_BYTES

const CENTRAL_DIR_SIGNATURE = 0x02014b50
const CENTRAL_DIR_FIXED_SIZE = 46

/** Central Directory bytes read cap — generous for any realistic upload
 * (a CD entry is ~50-100 bytes; 8MB covers on the order of 100k+ entries)
 * while still being a hard bound, never "read however much the archive
 * claims to have". An archive whose CD is bigger than this gets an honest
 * partial listing (`truncated: true`), never an unbounded read. */
export const MAX_CENTRAL_DIRECTORY_BYTES = 8 * 1024 * 1024

/** Display cap — independent of the byte cap above (a pathological archive
 * could have millions of tiny-named entries well within the byte cap). */
export const MAX_LISTED_ENTRIES = 500

interface EocdRecord {
  totalEntries: number
  centralDirectorySize: number
  centralDirectoryOffset: number
}

/**
 * Locates the End Of Central Directory record within `tail` (bytes read
 * from the END of the archive — see `readZipListing`) and returns its
 * entry-count/size/offset fields, or `null` when no valid EOCD is found
 * (not a ZIP file at all, or one too corrupt to trust) OR when a found
 * EOCD's fields signal ZIP64 (0xFFFF entries / 0xFFFFFFFF size or offset —
 * this reader doesn't implement the ZIP64 extension, see this file's top
 * comment).
 *
 * Scans BACKWARD from the end of `tail`, because the EOCD's own comment
 * field is attacker/tool-controlled bytes that could coincidentally
 * contain the 4-byte signature earlier in the buffer — the validity check
 * (`recordStart + EOCD_FIXED_SIZE + commentLength === tail.length`) also
 * guards this: a false-positive signature match essentially never also
 * has a comment-length field that exactly accounts for every remaining
 * byte to the true end of the buffer.
 */
export function findEocdInTail(tail: Uint8Array): EocdRecord | null {
  if (tail.length < EOCD_FIXED_SIZE) return null
  const view = new DataView(tail.buffer, tail.byteOffset, tail.byteLength)

  for (let i = tail.length - EOCD_FIXED_SIZE; i >= 0; i--) {
    if (view.getUint32(i, true) !== EOCD_SIGNATURE) continue
    const commentLength = view.getUint16(i + 20, true)
    if (i + EOCD_FIXED_SIZE + commentLength !== tail.length) continue // not the real EOCD — keep scanning backward

    const totalEntries = view.getUint16(i + 10, true)
    const centralDirectorySize = view.getUint32(i + 12, true)
    const centralDirectoryOffset = view.getUint32(i + 16, true)

    // ZIP64 marker values — this reader doesn't implement the ZIP64 EOCD
    // locator/record, so an archive that needs it is honestly unsupported
    // rather than mis-parsed with truncated 32-bit fields.
    if (totalEntries === 0xffff || centralDirectorySize === 0xffffffff || centralDirectoryOffset === 0xffffffff) {
      return null
    }

    return { totalEntries, centralDirectorySize, centralDirectoryOffset }
  }
  return null
}

/** MS-DOS directory attribute bit (external attributes, low byte, bit 4) —
 * used as a fallback for archives that never bothered to write the
 * conventional trailing "/" (rare, but real: some Windows zippers omit
 * it). */
const MSDOS_DIRECTORY_BIT = 0x10

/**
 * Parses sequential Central Directory File Header entries starting at the
 * very beginning of `cdBytes` (the caller has already sliced the archive
 * at exactly the EOCD's declared `centralDirectoryOffset`). Stops early —
 * returning whatever was parsed so far, never throwing — the moment a
 * header doesn't match the expected signature (either `cdBytes` was
 * truncated by `MAX_CENTRAL_DIRECTORY_BYTES`, or a genuinely corrupt
 * archive), or once `maxEntries` have been read (this reader's own display
 * cap — no point parsing entry 50,001 of an archive we'll only ever show
 * 500 rows of).
 */
export function parseCentralDirectoryEntries(cdBytes: Uint8Array, maxEntries: number): ZipEntryInfo[] {
  const entries: ZipEntryInfo[] = []
  const view = new DataView(cdBytes.buffer, cdBytes.byteOffset, cdBytes.byteLength)
  let offset = 0

  while (entries.length < maxEntries && offset + CENTRAL_DIR_FIXED_SIZE <= cdBytes.length) {
    if (view.getUint32(offset, true) !== CENTRAL_DIR_SIGNATURE) break

    const generalPurposeFlag = view.getUint16(offset + 8, true)
    const compressedSize = view.getUint32(offset + 20, true)
    const uncompressedSize = view.getUint32(offset + 24, true)
    const nameLength = view.getUint16(offset + 28, true)
    const extraLength = view.getUint16(offset + 30, true)
    const commentLength = view.getUint16(offset + 32, true)
    const externalAttributes = view.getUint32(offset + 38, true)

    const nameStart = offset + CENTRAL_DIR_FIXED_SIZE
    const nameEnd = nameStart + nameLength
    if (nameEnd > cdBytes.length) break // truncated read — stop, keep what we have

    // Bit 11 of the general-purpose flag (0x0800) means the name/comment
    // are UTF-8; otherwise assume the historical IBM Code Page 437
    // default, approximated here with latin1 (exact for the printable
    // ASCII range every test fixture and the overwhelming majority of
    // real filenames use — a documented approximation, not a full CP437
    // table, for the rare non-ASCII legacy-encoded name).
    const isUtf8 = (generalPurposeFlag & 0x0800) !== 0
    const nameBytes = cdBytes.subarray(nameStart, nameEnd)
    const name = isUtf8 ? new TextDecoder('utf-8').decode(nameBytes) : latin1Decode(nameBytes)

    const isDirectory = name.endsWith('/') || (externalAttributes & MSDOS_DIRECTORY_BIT) !== 0

    entries.push({ name, isDirectory, compressedSize, uncompressedSize })

    offset = nameEnd + extraLength + commentLength
  }

  return entries
}

function latin1Decode(bytes: Uint8Array): string {
  let s = ''
  for (let i = 0; i < bytes.length; i++) s += String.fromCharCode(bytes[i]!)
  return s
}

/**
 * Reads a `.zip` blob's file listing from its Central Directory alone —
 * see this file's top comment for the bounded-read strategy. Returns
 * `null` when the blob isn't a valid ZIP this reader can parse at all (no
 * EOCD found, or a ZIP64 archive) — the caller shows the same honest
 * "Preview not available" card any other unsupported type gets, never a
 * blank or a spinner.
 */
export async function readZipListing(blob: Blob): Promise<ZipListing | null> {
  const tailSize = Math.min(blob.size, EOCD_SEARCH_WINDOW_BYTES)
  const tailBuf = await blob.slice(blob.size - tailSize, blob.size).arrayBuffer()
  const eocd = findEocdInTail(new Uint8Array(tailBuf))
  if (!eocd) return null

  const cdReadSize = Math.min(eocd.centralDirectorySize, MAX_CENTRAL_DIRECTORY_BYTES)
  const cdBuf = await blob
    .slice(eocd.centralDirectoryOffset, eocd.centralDirectoryOffset + cdReadSize)
    .arrayBuffer()
  const parsed = parseCentralDirectoryEntries(new Uint8Array(cdBuf), MAX_LISTED_ENTRIES)

  // Per-ENTRY ZIP64 (Codex review, task 1574 gate, 2026-09-27) — distinct
  // from the whole-archive ZIP64 signal `findEocdInTail` already rejects
  // above. An archive can need ZIP64 for only ONE oversized member (≥4GiB)
  // while its EOCD-level entry count/CD size/CD offset all still fit 32
  // bits and pass that check cleanly. For such a member, THIS entry's own
  // Central Directory header stores the 0xFFFFFFFF sentinel in
  // compressedSize/uncompressedSize and puts the real 64-bit size in a
  // ZIP64 extended-information extra field this reader doesn't parse (see
  // this file's top comment: ZIP64 is deliberately not implemented at all,
  // not implemented for the common case and skipped for this one).
  // Without this check, `parseCentralDirectoryEntries` returns the sentinel
  // itself as a size — which decodes to exactly 4294967295 bytes (~4.0GB)
  // and would be shown as that entry's real size, a wrong number presented
  // as a fact. Rejecting the whole listing here (same "honest unsupported
  // card, never half-parsed" fallback as the EOCD-level check) is
  // consistent with this file's own design choice not to build a partial
  // ZIP64 reader.
  const ZIP64_SIZE_SENTINEL = 0xffffffff
  const hasZip64EntrySentinel = parsed.some(
    (e) => e.compressedSize === ZIP64_SIZE_SENTINEL || e.uncompressedSize === ZIP64_SIZE_SENTINEL,
  )
  if (hasZip64EntrySentinel) return null

  const byteCapHit = eocd.centralDirectorySize > MAX_CENTRAL_DIRECTORY_BYTES
  const displayCapHit = eocd.totalEntries > parsed.length
  return {
    entries: parsed,
    totalEntries: eocd.totalEntries,
    truncated: byteCapHit || displayCapHit,
  }
}
