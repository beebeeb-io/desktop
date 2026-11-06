/**
 * Person-facing strings for the downloading / streaming progress row (LD-0105 U1).
 * No logic lives here — only the final sentences. The model in `downloadProgressModel.ts`
 * assembles them from the DTO.
 *
 * Brand voice: honest over reassuring. No ellipsis on "failed" copy.
 */

/** The tray's one-line sub-label for each transfer state. */
export const TRAY_SUBLINE = {
  downloading: (pct: string, speed: string) => `Downloading · ${pct} · ${speed}`,
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
  downloading: (pct: string, speed: string) =>
    `Downloading · ${pct} · ${speed} · the program that opened it waits for the whole file`,
  streaming: (pct: string, speed: string) =>
    `Streaming · ${pct} buffered · ${speed} · playing while it arrives`,
  buffering: (speed: string) =>
    `Buffering · ${speed} · building a buffer before playback`,
  failed: "Couldn't finish downloading the file, so the download may be partial.",
} as const

/** Header for the "In progress" card on Activity › Timeline. */
export const IN_PROGRESS_CARD_HEADER = 'Files arriving on this PC.'

/** The "needs attention" action label (Amendment 4). */
export const NEEDS_ATTENTION_ACTION = 'Discard and download again'

/** Detail sentence for a "needs attention" row (Amendment 4). */
export const NEEDS_ATTENTION_DETAIL =
  'The file changed on disk while it was downloading. The original version was kept.'
