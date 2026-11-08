/**
 * Office file-kind detection (task 1567) — which files route to the office
 * editor (OfficeEditor, our LibreOffice-WASM chrome), and which of the three
 * engine apps (Writer/Calc/Impress) they open in.
 *
 * Pure — no DOM, no network — so it is unit-testable in isolation, same
 * pattern as `../text-editability.ts` / `../editor-conflict.ts`.
 */

export type OfficeApp = 'writer' | 'calc' | 'impress'

export interface OfficeFileKind {
  app: OfficeApp
  /** Extension actually opened with (lowercase, no dot), e.g. "docx". */
  ext: string
}

const WRITER_EXTENSIONS = new Set(['docx', 'doc', 'odt', 'rtf'])
const CALC_EXTENSIONS = new Set(['xlsx', 'xls', 'ods', 'csv'])
const IMPRESS_EXTENSIONS = new Set(['pptx', 'ppt', 'odp'])

const WRITER_MIME_TYPES = new Set([
  'application/vnd.openxmlformats-officedocument.wordprocessingml.document', // .docx
  'application/msword', // .doc
  'application/vnd.oasis.opendocument.text', // .odt
])
const CALC_MIME_TYPES = new Set([
  'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet', // .xlsx
  'application/vnd.ms-excel', // .xls
  'application/vnd.oasis.opendocument.spreadsheet', // .ods
])
const IMPRESS_MIME_TYPES = new Set([
  'application/vnd.openxmlformats-officedocument.presentationml.presentation', // .pptx
  'application/vnd.ms-powerpoint', // .ppt
  'application/vnd.oasis.opendocument.presentation', // .odp
])

function getExtension(filename: string): string {
  const dot = filename.lastIndexOf('.')
  if (dot < 0 || dot === filename.length - 1) return ''
  return filename.slice(dot + 1).toLowerCase()
}

/**
 * Resolves a file to an office app, or null if it isn't an office document
 * at all (mime type wins when present — extension is the ZK-upload fallback,
 * same convention as `resolveEditableKind` in file-preview.tsx).
 */
export function resolveOfficeFileKind(
  mimeType: string | null | undefined,
  filename: string,
): OfficeFileKind | null {
  const ext = getExtension(filename)
  if ((mimeType && WRITER_MIME_TYPES.has(mimeType)) || WRITER_EXTENSIONS.has(ext)) {
    return { app: 'writer', ext: ext || 'docx' }
  }
  if ((mimeType && CALC_MIME_TYPES.has(mimeType)) || CALC_EXTENSIONS.has(ext)) {
    return { app: 'calc', ext: ext || 'xlsx' }
  }
  if ((mimeType && IMPRESS_MIME_TYPES.has(mimeType)) || IMPRESS_EXTENSIONS.has(ext)) {
    return { app: 'impress', ext: ext || 'pptx' }
  }
  return null
}

/** Human label for the ribbon/breadcrumb/status bar. */
export function officeAppLabel(app: OfficeApp): string {
  switch (app) {
    case 'writer':
      return 'Writer'
    case 'calc':
      return 'Calc'
    case 'impress':
      return 'Impress'
  }
}

/** The extension a brand-new document of this kind is saved as by default. */
export function defaultExtensionFor(app: OfficeApp): string {
  switch (app) {
    case 'writer':
      return 'docx'
    case 'calc':
      return 'xlsx'
    case 'impress':
      return 'pptx'
  }
}
