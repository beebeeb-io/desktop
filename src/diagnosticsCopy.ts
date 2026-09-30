/**
 * Copy for the support bundle (task 1685).
 *
 * Every claim here is one the Rust export keeps, and is pinned to it by tests:
 * `src-tauri/src/diagnostic_redaction.rs` (allow-list filter, `[path]`/`[name]`
 * placeholders) and `diagnostics_export_tests` in `src-tauri/src/lib.rs`.
 * Change this text only together with those guarantees; `tests/diagnosticsCopy.test.ts`
 * fails if the old, broader claim comes back anywhere under `src/`.
 */
export const SUPPORT_BUNDLE_TITLE = 'Support bundle'

export const SUPPORT_BUNDLE_DETAIL =
  'Saves a support bundle you can read before you send it: queue counts, an error code and the last error message. ' +
  'Paths, file and folder names and sign-in tokens are removed; only standard error words and numbers are kept.'

export const SUPPORT_BUNDLE_SAVED_TITLE = 'Support bundle saved'

/** Toast body after the bundle is written; `path` is where the app saved it. */
export function supportBundleSavedMessage(path: string): string {
  return `Saved to ${path}. A draft to support is opening in your email app; attach the file yourself.`
}
