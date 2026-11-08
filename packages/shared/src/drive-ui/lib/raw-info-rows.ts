// ─── RAW camera rows for the Info rail ─────────────
// Task 1574: the camera/lens/exposure rows RawPreview extracted, shown the
// way mobile's Info sheet (task 1569) shows them. Absent for every non-RAW
// file, and for a RAW file with no EXIF this reader could resolve (e.g. CR3's
// embedded preview, which carries none — an honest gap, not a bug).

import type { RawExifInfo } from './raw-embedded-jpeg'

/** The label/value rows appended to the Info rail for a RAW file. */
export function rawInfoRowsOf(info: RawExifInfo | null): [string, string][] {
  if (!info) return []
  return [
    ...(info.cameraModel ? ([['Camera', info.cameraModel]] as [string, string][]) : []),
    ...(info.lensModel ? ([['Lens', info.lensModel]] as [string, string][]) : []),
    // `mapExifToRawInfo`'s own self-labeled string ("ISO 200") is its tested
    // contract (test/1574-raw-embedded-jpeg.test.ts), left as-is there. This
    // row's own "ISO" label column would repeat it verbatim ("ISO  ISO 200",
    // Codex review round 2, task 1574 gate, 2026-09-27) — every sibling row's
    // value is bare (Shutter: "1/125s", not "Shutter 1/125s"), so strip the
    // redundant prefix here, at the one place that adds the label.
    ...(info.iso ? ([['ISO', info.iso.replace(/^ISO\s*/i, '')]] as [string, string][]) : []),
    ...(info.shutterSpeed ? ([['Shutter', info.shutterSpeed]] as [string, string][]) : []),
    ...(info.aperture ? ([['Aperture', info.aperture]] as [string, string][]) : []),
    ...(info.focalLength ? ([['Focal length', info.focalLength]] as [string, string][]) : []),
  ]
}
