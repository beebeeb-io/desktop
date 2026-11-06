/**
 * Person-facing strings for the downloading / streaming progress row (LD-0105 U1).
 * No logic lives here — only the final sentences. The model in `downloadProgressModel.ts`
 * assembles them from the DTO.
 *
 * Brand voice: honest over reassuring. No ellipsis on "failed" copy.
 */

/** The tray's one-line sub-label for each transfer state. */
export const TRAY_SUBLINE = {
  /** `speed` null: no byte arrived in the speed window, so no speed is shown (never "0 B/s"). */
  downloading: (pct: string, speed: string | null) => (speed ? `Downloading · ${pct} · ${speed}` : `Downloading · ${pct}`),
  streaming: (pct: string, speed: string) => `Streaming · ${pct} buffered · ${speed}`,
  /** Buffering: speed known, no meaningful percent yet (total unknown or zero). */
  buffering: (speed: string) => `Buffering · ${speed}`,
  /** The transfer ended in an error. Full sentence goes in tooltip / Activity. */
  failed: "Couldn't finish downloading",
} as const

/**
 * The detail sentence shown in Activity › Timeline and in the tray row's tooltip.
 * Each sentence ends with a period.
 */
export const ACTIVITY_DETAIL = {
  downloading: (pct: string, speed: string | null) =>
    `Downloading · ${pct}${speed ? ` · ${speed}` : ''} · the program that opened it waits for the whole file`,
  streaming: (pct: string, speed: string) =>
    `Streaming · ${pct} buffered · ${speed} · playing while it arrives`,
  buffering: (speed: string) =>
    `Buffering · ${speed} · building a buffer before playback`,
  failed: "Couldn't finish downloading the file, so the download may be partial.",
} as const

/** Header for the "In progress" card on Activity › Timeline. */
export const IN_PROGRESS_CARD_HEADER = 'Files arriving on this PC.'

/**
 * "Needs attention" strings (Amendment 4, approved 2026-11-06).
 *
 * Story: a download failed; then something on this device changed the file
 * afterwards. Beebeeb kept the local file exactly as it is, won't use it as
 * the cloud file, and won't delete it without asking. The user must explicitly
 * discard the local copy before Beebeeb will try downloading again.
 */

/** Tray subline for a "needs attention" row: same short line as the failed row. */
export const NEEDS_ATTENTION_TRAY_SUBLINE = "Couldn't finish downloading"

/**
 * Activity detail / tooltip for a "needs attention" row.
 * Uses the word "Beebeeb" deliberately — the product is acting, not the cloud.
 */
export const NEEDS_ATTENTION_DETAIL =
  "Couldn't finish downloading. The file changed on this device afterwards, so Beebeeb kept it as it is. Discard it to download the file again."

/** Label for the action button and confirm button. */
export const NEEDS_ATTENTION_ACTION = 'Discard and download again'

/** Confirmation question before the discard. */
export const NEEDS_ATTENTION_CONFIRM =
  'Discard the copy on this device and download it again?'

/** Error messages for `discard_and_download_again` command errors. */
export const NEEDS_ATTENTION_ERRORS: Record<string, string> = {
  not_poisoned: "The file is no longer in a needs-attention state.",
  in_use: "Close the file in other apps, then try again.",
  missing: "The file is no longer downloading.",
}
/** Fallback for unexpected error codes from `discard_and_download_again`. */
export const NEEDS_ATTENTION_ERROR_FALLBACK = "Something went wrong. Try again."
