/**
 * Office open errors (task 1585, item 2).
 *
 * `bbOffice.open()` runs inside the engine iframe, so its rejections are
 * `Error`s from THAT realm: `err instanceof Error` is false in the host page
 * (each realm has its own `Error` constructor). office-editor.tsx used to test
 * exactly that, so every engine failure — a damaged file, an engine that never
 * booted — reached the user as the same generic "Failed to open this
 * document".
 *
 * This module is the boundary: it reads the rejection by shape (a `message`
 * string, and an optional `kind` the bridge may attach in a later engine
 * build), turns it into a host-realm `OfficeOpenError` with a real `kind`, and
 * pairs each kind with an honest, specific message. The raw engine text rides
 * along as `detail` (shown in mono under the message) — it is what a support
 * request needs, never what a user should have to parse.
 *
 * The kinds are grounded in what repos/office/bridge actually rejects with
 * (bb-office-api.js / bb-office-worker.js at office 743e7cc):
 *   - `bb-office-api.js: Module.uno_main never appeared within …ms`
 *       → the engine did not finish loading          → 'engine-not-loaded'
 *   - `loadComponentFromURL returned null for <name> (filter …)`, or a UNO
 *     IO/IllegalArgument exception thrown by the load itself
 *       → LibreOffice could not read the bytes        → 'invalid-document'
 *   - anything else from the engine                  → 'engine-failed'
 * `decrypt-failed` is raised by the office route itself (office-editor-page),
 * before the engine ever sees the bytes.
 */

export type OfficeOpenErrorKind = 'engine-not-loaded' | 'invalid-document' | 'engine-failed' | 'decrypt-failed'

const KINDS: readonly OfficeOpenErrorKind[] = ['engine-not-loaded', 'invalid-document', 'engine-failed', 'decrypt-failed']

/** What the user reads. Honest over reassuring: say what happened and what, if anything, helps. */
export const OFFICE_OPEN_ERROR_MESSAGES: Record<OfficeOpenErrorKind, string> = {
  'engine-not-loaded': "The editor didn't finish loading on this device. Reload the page to try again.",
  'invalid-document': "This file isn't a document the editor can read. It may be damaged, or saved in a format it doesn't support. Your file is unchanged.",
  'engine-failed': 'The editor ran into an error while opening this document. Reload the page to try again. Your file is unchanged.',
  'decrypt-failed': "This file couldn't be decrypted on this device, so there is nothing to open. Your file is unchanged.",
}

export class OfficeOpenError extends Error {
  readonly kind: OfficeOpenErrorKind
  /** The raw engine/crypto text, for the mono detail line and support. */
  readonly detail: string
  constructor(kind: OfficeOpenErrorKind, detail: string) {
    super(OFFICE_OPEN_ERROR_MESSAGES[kind])
    this.name = 'OfficeOpenError'
    this.kind = kind
    this.detail = detail
  }
}

function isKind(value: unknown): value is OfficeOpenErrorKind {
  return typeof value === 'string' && (KINDS as readonly string[]).includes(value)
}

/** Reads a rejection's text without `instanceof` (the value may come from another realm). */
export function errorText(err: unknown): string {
  if (typeof err === 'string') return err
  if (err && typeof err === 'object') {
    const message = (err as { message?: unknown }).message
    if (typeof message === 'string' && message.length > 0) return message
    try {
      const s = String(err)
      if (s && s !== '[object Object]') return s
    } catch {
      // A hostile/revoked proxy — fall through.
    }
  }
  return ''
}

const ENGINE_NOT_LOADED = /Module\.uno_main never appeared|did not become ready in time/i
const INVALID_DOCUMENT = /loadComponentFromURL returned null|com\.sun\.star\.(io\.IOException|lang\.IllegalArgumentException|io\.WrongFormatException)|WrongFormat/i

/** Classifies a `bbOffice.open()` rejection (any realm) into a host-realm `OfficeOpenError`. */
export function toOfficeOpenError(err: unknown): OfficeOpenError {
  if (err instanceof OfficeOpenError) return err
  const detail = errorText(err)
  const carried = err && typeof err === 'object' ? (err as { kind?: unknown }).kind : undefined
  if (isKind(carried)) return new OfficeOpenError(carried, detail)
  if (ENGINE_NOT_LOADED.test(detail)) return new OfficeOpenError('engine-not-loaded', detail)
  if (INVALID_DOCUMENT.test(detail)) return new OfficeOpenError('invalid-document', detail)
  return new OfficeOpenError('engine-failed', detail)
}

/**
 * For the /office route's decrypt steps (the name metadata, then the content):
 * a transport failure is passed through unchanged, so the user sees the real
 * download error. Transport failures are the server's ApiError, fetch's
 * TypeError when the network is gone, and an abort. Anything else thrown by
 * those steps is the decryption itself failing, and becomes 'decrypt-failed'
 * instead of whatever the crypto layer's message happens to be.
 * ApiError is matched by name, not instanceof, to keep this module free of
 * the API client.
 */
export function asDecryptFailure(err: unknown): unknown {
  if (err instanceof OfficeOpenError || err instanceof TypeError) return err
  if (err instanceof Error && err.name === 'ApiError') return err
  if (err instanceof DOMException && err.name === 'AbortError') return err
  return new OfficeOpenError('decrypt-failed', errorText(err))
}
