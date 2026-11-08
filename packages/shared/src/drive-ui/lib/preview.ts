// Whether a file can be previewed in the app. The rules live in core (one extension table, one set of MIME
// classes); a host registers core's predicate (web: WASM `is_previewable` + the `web` column; desktop: the
// engine). Until one is registered the answer is false rather than a local guess.

export type PreviewPredicate = (mimeType: string | null | undefined, fileName?: string | null) => boolean

let predicate: PreviewPredicate | null = null

export function registerPreviewPredicate(fn: PreviewPredicate | null): void {
  predicate = fn
}

export function isPreviewable(mimeType: string | null | undefined, fileName?: string | null): boolean {
  return predicate ? predicate(mimeType, fileName) : false
}
