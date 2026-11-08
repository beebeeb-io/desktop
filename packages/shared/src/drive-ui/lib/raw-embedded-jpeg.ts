/**
 * raw-embedded-jpeg — PURE core of the web camera-RAW preview feature (task
 * 1574): finding the largest embedded JPEG in a RAW file's bytes, and
 * reading its own minimal TIFF/EXIF tags (Make/Model/LensModel/ISO/
 * ExposureTime/FNumber/FocalLength) for the Info rail.
 *
 * This is a direct port of mobile's `src/lib/raw-preview.ts` (task 1569) —
 * that module's own doc comment has the full story of why a hand-rolled
 * byte scanner beats `exifr` for 4 of 6 RAW formats (CR3/RAF aren't
 * top-level formats `exifr` recognizes at all; NEF/DNG's real preview lives
 * in a SubIFDs entry `exifr.thumbnail()` doesn't read) — the exact gap this
 * task closes on web (`raw-preview.tsx` previously used `exifr.thumbnail()`
 * directly and only found a usable preview for 2 of 6: CR2, ARW). Verified
 * independently on web's OWN fixtures (`e2e/fixtures/preview-matrix/raw/`)
 * before this task's e2e expectations were written: this scanner finds a
 * real, correctly-oriented, correctly-dimensioned preview in all 6
 * (confirmed via macOS `sips` against the extracted bytes) — dimensions
 * matched mobile's own fixture dimensions exactly for DNG/CR3/ARW/NEF/RAF
 * (same raw.pixls.us source files), which is strong independent
 * confirmation this port has no transcription bugs.
 *
 * Kept dependency-free (no DOM, no Worker globals) so it stays directly
 * unit-testable with `bun test` and reusable from the Web Worker
 * (`src/workers/raw-preview.worker.ts`) that actually reads file bytes —
 * same pure/impure split as mobile's `raw-preview.ts` / `raw-extract.ts`.
 *
 * Unlike the mobile version, this file has NO base64 helpers — the browser
 * worker reads bytes via `Blob.arrayBuffer()` directly, never a base64
 * round-trip (that was mobile's `expo-file-system` API constraint, not a
 * universal one).
 */

export interface RawExifInfo {
  cameraModel: string | null;
  lensModel: string | null;
  iso: string | null;
  shutterSpeed: string | null;
  aperture: string | null;
  focalLength: string | null;
}

export interface JpegSpan {
  /** Byte offset of the span's leading 0xFFD8 (SOI). */
  start: number;
  /** Byte offset one past the span's trailing 0xFFD9 (EOI) — i.e. exclusive,
   * so `bytes.subarray(start, end)` is the complete JPEG. */
  end: number;
}

/** Below this size, a found SOI…EOI span is treated as an incidental byte
 * coincidence in raw sensor data rather than a genuine embedded preview —
 * every real fixture's largest span is well over 100KB, so this has wide
 * margin without risking a false negative on a real (if small) preview. */
export const MIN_EMBEDDED_JPEG_BYTES = 4096;

/**
 * Scans arbitrary bytes for every complete JFIF-style JPEG (SOI 0xFFD8 …
 * EOI 0xFFD9) span, walking each marker segment's own declared length so
 * that entropy-coded scan data — which routinely contains incidental
 * 0xFF-led byte pairs — never truncates a span early. A naive
 * `indexOf(0xFFD9)` does exactly that: it would stop at the FIRST 0xFF 0xD9
 * byte pair anywhere after the SOI, which for a multi-megabyte JPEG is
 * almost always well before the real end. This walks the actual marker
 * structure instead: ordinary segments are skipped by their own two-byte
 * big-endian length; the SOS (Start Of Scan) segment's entropy-coded data
 * has no declared length, so scanning resumes byte-by-byte until the next
 * genuine marker (a literal 0xFF in scan data is always "stuffed" as
 * 0xFF 0x00 by any correct encoder, or is an RSTn restart marker — neither
 * of those, nor a fill byte, terminates the scan; only a marker with a
 * real payload code does).
 */
export function findJpegSpans(bytes: Uint8Array): JpegSpan[] {
  const spans: JpegSpan[] = [];
  const len = bytes.length;
  let i = 0;
  while (i < len - 2) {
    if (bytes[i] === 0xff && bytes[i + 1] === 0xd8 && bytes[i + 2] === 0xff) {
      const end = scanForEoi(bytes, i + 2);
      if (end !== null) {
        spans.push({ start: i, end });
        i = end;
        continue;
      }
    }
    i++;
  }
  return spans;
}

/**
 * Walks marker segments starting at a byte index known to hold 0xFF (the
 * third byte of an already-matched SOI), returning the index just past the
 * terminating EOI, or `null` if the buffer ends first (a truncated/corrupt
 * candidate — the caller's outer scan just advances one byte and keeps
 * looking, which is safe: JPEG markers are byte-aligned two-byte codes, so
 * a genuine JPEG will not be found starting one byte later either).
 */
function scanForEoi(bytes: Uint8Array, from: number): number | null {
  const len = bytes.length;
  let k = from;
  while (k < len - 1) {
    if (bytes[k] !== 0xff) {
      k++;
      continue;
    }
    const marker = bytes[k + 1];
    if (marker === 0xd9) return k + 2; // EOI
    if (marker === 0x01 || (marker! >= 0xd0 && marker! <= 0xd7)) {
      // TEM / RSTn restart markers carry no payload.
      k += 2;
      continue;
    }
    if (marker === 0xff) {
      // Fill byte before the real marker code — re-check at k+1.
      k++;
      continue;
    }
    if (marker === 0xda) {
      // SOS: a normal length-prefixed header, then raw entropy-coded scan
      // data with NO declared length — resume byte-by-byte until the next
      // marker that isn't a stuffed 0xFF00 or an RSTn restart marker.
      if (k + 4 > len) return null;
      const segLen = (bytes[k + 2]! << 8) | bytes[k + 3]!;
      k = k + 2 + segLen;
      while (k < len - 1) {
        if (bytes[k] === 0xff && bytes[k + 1] !== 0x00 && !(bytes[k + 1]! >= 0xd0 && bytes[k + 1]! <= 0xd7)) {
          break;
        }
        k++;
      }
      continue;
    }
    // Generic marker segment: 2-byte big-endian length, INCLUDING itself.
    if (k + 4 > len) return null;
    const segLen = (bytes[k + 2]! << 8) | bytes[k + 3]!;
    if (segLen < 2) return null; // malformed — bail, caller advances past this candidate
    k += 2 + segLen;
  }
  return null;
}

/**
 * Above this ratio, a candidate span is treated as an implausible JPEG and
 * skipped rather than trusted. Real embedded camera previews across all six
 * of this task's fixtures measured 0.05–0.83 bytes/pixel; 1.0 leaves wide
 * margin above the highest real one while still catching the false-positive
 * class this constant exists for (see `jpegPixelCount`'s doc comment).
 */
const MAX_PLAUSIBLE_JPEG_BYTES_PER_PIXEL = 1.0;

function readUint16BE(bytes: Uint8Array, offset: number): number {
  return (bytes[offset]! << 8) | bytes[offset + 1]!;
}

/**
 * Reads the pixel count (width × height) a JPEG span's own Start-Of-Frame
 * marker (SOF0-SOF15, excluding DHT/JPG/DAC which reuse marker codes in
 * that range) declares, or `null` if none is found/parseable before the
 * span's own end. Used by `findLargestJpegSpan` to reject spans whose
 * claimed dimensions don't match their byte size plausibly — see that
 * function's doc comment for why this check exists at all.
 */
function jpegPixelCount(bytes: Uint8Array, span: JpegSpan): number | null {
  let i = span.start + 2; // skip the SOI this span starts with
  const end = span.end;
  while (i < end - 1) {
    if (bytes[i] !== 0xff) {
      i++;
      continue;
    }
    const marker = bytes[i + 1]!;
    if (marker === 0xd8 || marker === 0x01 || (marker >= 0xd0 && marker <= 0xd9)) {
      i += 2;
      continue;
    }
    if (marker === 0xff) {
      i++;
      continue;
    }
    if (i + 4 > end) return null;
    const segLen = readUint16BE(bytes, i + 2);
    const isStartOfFrame = marker >= 0xc0 && marker <= 0xcf && marker !== 0xc4 && marker !== 0xc8 && marker !== 0xcc;
    if (isStartOfFrame) {
      if (i + 9 > end) return null;
      const height = readUint16BE(bytes, i + 5);
      const width = readUint16BE(bytes, i + 7);
      return width * height > 0 ? width * height : null;
    }
    if (marker === 0xda) return null; // hit scan data without ever finding a SOF
    if (segLen < 2) return null;
    i += 2 + segLen;
  }
  return null;
}

/**
 * The largest complete JPEG span in `bytes` at or above
 * `MIN_EMBEDDED_JPEG_BYTES` whose own declared dimensions are plausible for
 * its byte size, or `null` if none qualifies (a genuinely corrupt/
 * unsupported file, or one whose preview lies past the caller's own bounded
 * read window — see `raw-preview.worker.ts` — the honest fallback card is
 * always correct here, never a blank/spinner).
 *
 * "Largest wins" alone is NOT enough: verified on mobile's real CR2 fixture
 * (task 1569) — the RAW sensor data that follows the real embedded preview
 * in the file can, purely by byte-pattern coincidence, contain a span that
 * LOOKS like a complete, well-formed JPEG (valid SOI/markers/EOI) and is
 * LARGER than the genuine preview, but decodes to a flat, wrong,
 * noise-banded image — not the real photo. Rejecting implausibly-dense
 * candidates and falling through to the next-largest one fixes this without
 * a real JPEG decoder.
 */
export function findLargestJpegSpan(bytes: Uint8Array): JpegSpan | null {
  const candidates = findJpegSpans(bytes)
    .map((span) => ({ span, size: span.end - span.start }))
    .filter(({ size }) => size >= MIN_EMBEDDED_JPEG_BYTES)
    .sort((a, b) => b.size - a.size);

  for (const { span, size } of candidates) {
    const pixels = jpegPixelCount(bytes, span);
    if (pixels != null && size / pixels > MAX_PLAUSIBLE_JPEG_BYTES_PER_PIXEL) {
      continue; // implausibly dense for its own declared dimensions — skip
    }
    return span;
  }
  return null;
}

// ---------------------------------------------------------------------------
// Minimal TIFF/EXIF tag reader — same rationale as mobile's raw-preview.ts:
// a hand-rolled reader for exactly the tags this task needs (Make, Model,
// LensModel, ISOSpeedRatings, ExposureTime, FNumber, FocalLength) from IFD0
// and, when present, the Exif sub-IFD it points to (tag 0x8769). No
// thumbnail/SubIFDs/GPS/maker-note handling — this is deliberately narrow,
// not a general TIFF library (the actual PREVIEW image comes from the JPEG
// span scanner above, not from this reader).
// ---------------------------------------------------------------------------

const TIFF_TYPE_SIZE_BYTES: Record<number, number> = {
  1: 1, // BYTE
  2: 1, // ASCII
  3: 2, // SHORT
  4: 4, // LONG
  5: 8, // RATIONAL (2x LONG)
  6: 1, // SBYTE
  7: 1, // UNDEFINED
  8: 2, // SSHORT
  9: 4, // SLONG
  10: 8, // SRATIONAL (2x SLONG)
  11: 4, // FLOAT
  12: 8, // DOUBLE
};

/** Tag id -> output key, for the tags this reader understands. 0x8769
 * (ExifIFD) is handled specially in `readIfd` (it recurses into the
 * pointed-to sub-IFD rather than being read as an ordinary value). */
const TIFF_TAG_NAMES: Record<number, string> = {
  0x010f: 'Make',
  0x0110: 'Model',
  0x8769: 'ExifIFD',
  0x829a: 'ExposureTime',
  0x829d: 'FNumber',
  0x8827: 'ISO',
  0x920a: 'FocalLength',
  0xa434: 'LensModel',
};

function readUint16(bytes: Uint8Array, offset: number, little: boolean): number {
  const b0 = bytes[offset]!;
  const b1 = bytes[offset + 1]!;
  return little ? b0 | (b1 << 8) : (b0 << 8) | b1;
}

function readUint32(bytes: Uint8Array, offset: number, little: boolean): number {
  const b0 = bytes[offset]!;
  const b1 = bytes[offset + 1]!;
  const b2 = bytes[offset + 2]!;
  const b3 = bytes[offset + 3]!;
  return (little ? b0 | (b1 << 8) | (b2 << 16) | (b3 << 24) : (b0 << 24) | (b1 << 16) | (b2 << 8) | b3) >>> 0;
}

function readAsciiValue(bytes: Uint8Array, offset: number, count: number): string {
  let s = '';
  for (let i = 0; i < count; i++) {
    const c = bytes[offset + i];
    if (!c) break; // NUL-terminated, per the TIFF spec
    s += String.fromCharCode(c);
  }
  return s.trim();
}

/** Reads one IFD entry's value given its type/count and the offset of its
 * own 4-byte value/offset field (which either holds the value inline, when
 * it fits in 4 bytes, or an offset to it elsewhere in the buffer). Returns
 * `null` for anything out of bounds (a corrupt or adversarially-crafted
 * offset) rather than throwing — this reader must degrade to "no EXIF
 * shown", never crash the preview. Only ASCII/SHORT/LONG/RATIONAL are
 * implemented — the only types this task's six tags actually use. */
function readIfdEntryValue(
  bytes: Uint8Array,
  tiffBase: number,
  little: boolean,
  type: number,
  count: number,
  valueFieldOffset: number,
): string | number | null {
  const typeSize = TIFF_TYPE_SIZE_BYTES[type];
  if (!typeSize || count <= 0) return null;
  const totalBytes = typeSize * count;
  const dataOffset = totalBytes <= 4 ? valueFieldOffset : tiffBase + readUint32(bytes, valueFieldOffset, little);
  if (dataOffset < 0 || dataOffset + totalBytes > bytes.length) return null;

  if (type === 2) return readAsciiValue(bytes, dataOffset, count) || null; // ASCII
  if (type === 3) return readUint16(bytes, dataOffset, little); // SHORT (first value)
  if (type === 4) return readUint32(bytes, dataOffset, little); // LONG (first value)
  if (type === 5) {
    // RATIONAL: two LONGs, numerator then denominator.
    const numerator = readUint32(bytes, dataOffset, little);
    const denominator = readUint32(bytes, dataOffset + 4, little);
    return denominator !== 0 ? numerator / denominator : null;
  }
  return null;
}

/** Walks one IFD's entries, writing recognized tags into `out`. Recurses
 * (at most `depthRemaining` times — real EXIF only ever nests IFD0 -> Exif
 * sub-IFD, i.e. depth 1) when it finds the Exif sub-IFD pointer, so a
 * maliciously crafted cyclic offset can't recurse unboundedly. */
function readIfd(
  bytes: Uint8Array,
  tiffBase: number,
  little: boolean,
  ifdOffset: number,
  out: Record<string, string | number>,
  depthRemaining: number,
): void {
  if (ifdOffset < 0 || ifdOffset + 2 > bytes.length) return;
  const entryCount = readUint16(bytes, ifdOffset, little);
  let entryOffset = ifdOffset + 2;
  for (let i = 0; i < entryCount; i++) {
    if (entryOffset + 12 > bytes.length) break;
    const tag = readUint16(bytes, entryOffset, little);
    const type = readUint16(bytes, entryOffset + 2, little);
    const count = readUint32(bytes, entryOffset + 4, little);
    const valueFieldOffset = entryOffset + 8;
    const name = TIFF_TAG_NAMES[tag];
    if (name === 'ExifIFD') {
      if (depthRemaining > 0) {
        const subIfdOffset = readUint32(bytes, valueFieldOffset, little);
        readIfd(bytes, tiffBase, little, tiffBase + subIfdOffset, out, depthRemaining - 1);
      }
    } else if (name) {
      const value = readIfdEntryValue(bytes, tiffBase, little, type, count, valueFieldOffset);
      if (value !== null) out[name] = value;
    }
    entryOffset += 12;
  }
}

/**
 * Reads Make/Model/LensModel/ISO/ExposureTime/FNumber/FocalLength from a
 * TIFF-structured byte range (a raw TIFF file — CR2/ARW/NEF/DNG all start
 * with one at byte 0 — or an embedded Exif block inside a JPEG, via
 * `findJpegExifTiffOffset`'s returned offset). Returns `null` when
 * `startOffset` isn't a valid TIFF header (wrong signature/magic, or a
 * corrupt/truncated buffer) or when the IFD walk found none of the
 * recognized tags.
 */
export function parseTiffExifTags(bytes: Uint8Array, startOffset = 0): Record<string, string | number> | null {
  if (startOffset < 0 || startOffset + 8 > bytes.length) return null;
  const b0 = bytes[startOffset]!;
  const b1 = bytes[startOffset + 1]!;
  let little: boolean;
  if (b0 === 0x49 && b1 === 0x49) little = true; // "II" — little-endian
  else if (b0 === 0x4d && b1 === 0x4d) little = false; // "MM" — big-endian
  else return null;

  const magic = readUint16(bytes, startOffset + 2, little);
  if (magic !== 42) return null;

  const ifd0Offset = readUint32(bytes, startOffset + 4, little);
  const out: Record<string, string | number> = {};
  readIfd(bytes, startOffset, little, startOffset + ifd0Offset, out, 1);
  return Object.keys(out).length > 0 ? out : null;
}

/**
 * Finds a JPEG's APP1 "Exif\0\0" segment and returns the byte offset where
 * the TIFF structure inside it begins (ready to hand to `parseTiffExifTags`
 * as `startOffset`), or `null` if the JPEG has no such segment. Only scans
 * the marker segments that precede SOS (Start Of Scan) — APP1/Exif always
 * appears there, never inside entropy-coded scan data.
 */
export function findJpegExifTiffOffset(bytes: Uint8Array): number | null {
  let i = 0;
  const len = bytes.length;
  while (i < len - 4) {
    if (bytes[i] !== 0xff) {
      i++;
      continue;
    }
    const marker = bytes[i + 1]!;
    if (marker === 0xd8 || marker === 0x01 || (marker >= 0xd0 && marker <= 0xd9)) {
      i += 2;
      continue;
    }
    if (marker === 0xda) return null; // Start Of Scan — no more markers before it
    if (i + 4 > len) return null;
    const segLen = (bytes[i + 2]! << 8) | bytes[i + 3]!;
    if (marker === 0xe1 && segLen >= 8) {
      const payloadOffset = i + 4;
      if (
        bytes[payloadOffset] === 0x45 && // 'E'
        bytes[payloadOffset + 1] === 0x78 && // 'x'
        bytes[payloadOffset + 2] === 0x69 && // 'i'
        bytes[payloadOffset + 3] === 0x66 && // 'f'
        bytes[payloadOffset + 4] === 0x00 &&
        bytes[payloadOffset + 5] === 0x00
      ) {
        return payloadOffset + 6;
      }
    }
    if (segLen < 2) return null;
    i += 2 + segLen;
  }
  return null;
}

function trimTrailingZero(n: number): string {
  return (Math.round(n * 10) / 10).toString();
}

function formatShutterSpeed(seconds: number | null | undefined): string | null {
  if (seconds == null || !Number.isFinite(seconds) || seconds <= 0) return null;
  if (seconds >= 1) return `${trimTrailingZero(seconds)}s`;
  const denominator = Math.round(1 / seconds);
  return denominator > 1 ? `1/${denominator}s` : `${trimTrailingZero(seconds)}s`;
}

/**
 * Maps the raw tag object to the six fields the Info rail shows (camera
 * model, lens, ISO, shutter speed, aperture, focal length). Returns `null`
 * when NONE of the six could be resolved, so the caller can skip adding an
 * empty EXIF section entirely rather than showing six blank rows.
 */
export function mapExifToRawInfo(tags: Record<string, unknown> | null | undefined): RawExifInfo | null {
  if (!tags) return null;

  const make = typeof tags.Make === 'string' ? tags.Make.trim() : '';
  const model = typeof tags.Model === 'string' ? tags.Model.trim() : '';
  // Some cameras' Model already repeats the Make (e.g. "Canon EOS 40D") —
  // avoid "Canon Canon EOS 40D" by dropping Make when Model already starts
  // with it (case-insensitive).
  const cameraModel =
    make && model && model.toLowerCase().startsWith(make.toLowerCase())
      ? model
      : [make, model].filter(Boolean).join(' ').trim() || null;

  const lensRaw = typeof tags.LensModel === 'string' ? tags.LensModel.trim() : '';
  // Sony (and others) report an all-dashes placeholder ("----") when no
  // lens is recorded — treat that the same as absent, not as a real value.
  const lensModel = lensRaw && !/^-+$/.test(lensRaw) ? lensRaw : null;

  const iso = typeof tags.ISO === 'number' && tags.ISO > 0 ? `ISO ${Math.round(tags.ISO)}` : null;
  const shutterSpeed = formatShutterSpeed(typeof tags.ExposureTime === 'number' ? tags.ExposureTime : null);
  const aperture = typeof tags.FNumber === 'number' && tags.FNumber > 0 ? `f/${trimTrailingZero(tags.FNumber)}` : null;
  const focalLength =
    typeof tags.FocalLength === 'number' && tags.FocalLength > 0 ? `${trimTrailingZero(tags.FocalLength)}mm` : null;

  if (!cameraModel && !lensModel && !iso && !shutterSpeed && !aperture && !focalLength) return null;
  return { cameraModel: cameraModel || null, lensModel, iso, shutterSpeed, aperture, focalLength };
}
