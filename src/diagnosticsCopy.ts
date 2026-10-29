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
  'Paths, file and folder names and sign-in tokens are removed. Standard error words, numbers, opaque ids and the server address are kept, ' +
  'so a name the app does not recognise can stay only if it is itself a standard word or a number.'

export const SUPPORT_BUNDLE_SAVED_TITLE = 'Support bundle saved'

/** What `report_problem` returns: the bundle is written before the email draft is tried. */
export interface ProblemReportResult {
  path: string
  email_opened: boolean
}

/** Toast body after the bundle is written; `path` is where the app saved it. */
export function supportBundleSavedMessage(path: string, emailOpened: boolean = true): string {
  if (!emailOpened) {
    return `Saved to ${path}. Your email app didn’t open; send the file to support@beebeeb.io yourself.`
  }
  return `Saved to ${path}. A draft to support is opening in your email app; attach the file yourself.`
}
