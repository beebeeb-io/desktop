// What one Office save does (DW-1507, DW-1558), apart from the window: a new document's first save makes the file, a
// later one writes a new version over the tag it was read at. Pure of React so each path is tested (#273 finding 11).
import { DriveSourceError, type DriveItem } from '@beebeeb/shared/drive-ui'

export type OfficeSaveResult =
  /** Rust wrote the new file. `versionId` is its tag, or null when the engine has not indexed it yet. */
  | { kind: 'made'; item: DriveItem; versionId: string | null }
  /** Rust wrote a new version on this PC; `versionId` is its tag. */
  | { kind: 'saved'; versionId: string }
  /** The new file is written but not indexed yet, so there is no tag to save over: nothing was written. */
  | { kind: 'not-indexed' }

export async function officeSave(
  at: { creating: boolean; fileId: string | null; doc: { versionId: string | null } | null; base?: string },
  bytes: Uint8Array,
  io: {
    make: (bytes: Uint8Array) => Promise<DriveItem>
    tagOf: (id: string) => Promise<string | null>
    saveBytes: (id: string, bytes: Uint8Array, base: string) => Promise<{ versionId: string }>
  },
): Promise<OfficeSaveResult> {
  if (at.creating) {
    // Once Rust has made the file, this save is done; a read the engine cannot answer yet never undoes that.
    const item = await io.make(bytes)
    return { kind: 'made', item, versionId: await io.tagOf(item.id).catch(() => null) }
  }
  // A save with nothing to save into is an error, never a success.
  if (!at.fileId || !at.doc) throw new DriveSourceError('failed', 'no document')
  const base = at.base ?? at.doc.versionId ?? await io.tagOf(at.fileId)
  if (!base) return { kind: 'not-indexed' }
  return { kind: 'saved', versionId: (await io.saveBytes(at.fileId, bytes, base)).versionId }
}
