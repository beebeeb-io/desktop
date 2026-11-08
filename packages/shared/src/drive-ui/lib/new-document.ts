/**
 * Drive "+ New" menu (task 1582) — the type table, default names and name
 * uniqueness for creating a brand-new file in the current folder.
 *
 * Pure: no DOM, no network, no crypto, so it is unit-tested in isolation
 * (new-document.test.ts). The impure half — the blank bytes, the encrypted
 * upload, opening the editor — lives in office/blank-documents.ts and
 * pages/drive.tsx.
 *
 * Which editor opens:
 *   - 'office' types open in the LibreOffice-WASM office editor
 *     (/office/:fileId, task 1567). They are offered ONLY when that editor is
 *     reachable: the build flag (VITE_FEATURE_OFFICE_EDITOR), the same gate as
 *     the preview's Edit button and the /office/:fileId route — a user must never be able to create a file the
 *     app then cannot open for editing.
 *   - 'text' types open in the in-preview text editor (task 1563), which is
 *     live for everyone.
 */

export type NewDocumentEditor = 'office' | 'text'
export type NewDocumentGroup = 'office' | 'opendocument' | 'text'

export interface NewDocumentType {
  /** Stable id, also the testid suffix (`new-menu-<id>`). */
  id: 'docx' | 'xlsx' | 'pptx' | 'odt' | 'ods' | 'odp' | 'txt' | 'md'
  /** Menu label. */
  label: string
  /** Dialog title / command-palette label ("New spreadsheet"). */
  title: string
  /** Secondary line in the menu, names the format honestly. */
  hint: string
  /** Extension, lowercase, no dot. */
  ext: string
  mimeType: string
  editor: NewDocumentEditor
  group: NewDocumentGroup
  /** Default name before the extension ("Untitled document"). */
  defaultBase: string
  /** Icon name from @beebeeb/shared's Icon set. */
  icon: 'file-text' | 'file-spreadsheet' | 'file-presentation' | 'file-code'
}

export const NEW_DOCUMENT_TYPES: readonly NewDocumentType[] = [
  {
    id: 'docx',
    title: 'New document',
    label: 'Document',
    hint: 'Word .docx',
    ext: 'docx',
    mimeType: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
    editor: 'office',
    group: 'office',
    defaultBase: 'Untitled document',
    icon: 'file-text',
  },
  {
    id: 'xlsx',
    title: 'New spreadsheet',
    label: 'Spreadsheet',
    hint: 'Excel .xlsx',
    ext: 'xlsx',
    mimeType: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
    editor: 'office',
    group: 'office',
    defaultBase: 'Untitled spreadsheet',
    icon: 'file-spreadsheet',
  },
  {
    id: 'pptx',
    title: 'New presentation',
    label: 'Presentation',
    hint: 'PowerPoint .pptx',
    ext: 'pptx',
    mimeType: 'application/vnd.openxmlformats-officedocument.presentationml.presentation',
    editor: 'office',
    group: 'office',
    defaultBase: 'Untitled presentation',
    icon: 'file-presentation',
  },
  {
    id: 'odt',
    title: 'New OpenDocument text',
    label: 'Text document',
    hint: 'OpenDocument .odt',
    ext: 'odt',
    mimeType: 'application/vnd.oasis.opendocument.text',
    editor: 'office',
    group: 'opendocument',
    defaultBase: 'Untitled document',
    icon: 'file-text',
  },
  {
    id: 'ods',
    title: 'New OpenDocument spreadsheet',
    label: 'Spreadsheet',
    hint: 'OpenDocument .ods',
    ext: 'ods',
    mimeType: 'application/vnd.oasis.opendocument.spreadsheet',
    editor: 'office',
    group: 'opendocument',
    defaultBase: 'Untitled spreadsheet',
    icon: 'file-spreadsheet',
  },
  {
    id: 'odp',
    title: 'New OpenDocument presentation',
    label: 'Presentation',
    hint: 'OpenDocument .odp',
    ext: 'odp',
    mimeType: 'application/vnd.oasis.opendocument.presentation',
    editor: 'office',
    group: 'opendocument',
    defaultBase: 'Untitled presentation',
    icon: 'file-presentation',
  },
  {
    id: 'txt',
    title: 'New text file',
    label: 'Text file',
    hint: 'Plain text .txt',
    ext: 'txt',
    mimeType: 'text/plain',
    editor: 'text',
    group: 'text',
    defaultBase: 'Untitled',
    icon: 'file-text',
  },
  {
    id: 'md',
    title: 'New Markdown file',
    label: 'Markdown',
    hint: 'Markdown .md',
    ext: 'md',
    mimeType: 'text/markdown',
    editor: 'text',
    group: 'text',
    defaultBase: 'Untitled',
    icon: 'file-code',
  },
]

export function getNewDocumentType(id: NewDocumentType['id']): NewDocumentType {
  const t = NEW_DOCUMENT_TYPES.find((x) => x.id === id)
  if (!t) throw new Error(`unknown new-document type: ${id}`)
  return t
}

/**
 * The types the menu shows. Office types only when the office editor is
 * actually reachable (`officeAvailable` = FEATURE_OFFICE_EDITOR, the build
 * flag, passed in by the caller so this stays pure).
 */
export function visibleNewDocumentTypes(officeAvailable: boolean): NewDocumentType[] {
  return NEW_DOCUMENT_TYPES.filter((t) => officeAvailable || t.editor !== 'office')
}

/** Locale-independent case fold for name comparisons (`toLocaleLowerCase`
 *  under a Turkish locale maps "I" to dotless "ı" and would let "TITLE.docx"
 *  and "title.docx" coexist). Used by every clash check (PR #117 review). */
export function foldName(name: string): string {
  return name.toLowerCase()
}

function splitExt(name: string): { base: string; ext: string } {
  const dot = name.lastIndexOf('.')
  if (dot <= 0 || dot === name.length - 1) return { base: name, ext: '' }
  return { base: name.slice(0, dot), ext: name.slice(dot) }
}

/**
 * `desired` if no sibling has that name, otherwise the first free
 * "<base> 2<.ext>", "<base> 3<.ext>", … Case-insensitive, like the file
 * systems these files will be synced to (macOS/Windows), so "Untitled.md"
 * and "untitled.md" never end up side by side after a sync.
 */
export function uniqueFileName(desired: string, existingNames: Iterable<string>): string {
  const taken = new Set<string>()
  for (const n of existingNames) taken.add(foldName(n))
  if (!taken.has(foldName(desired))) return desired
  const { base, ext } = splitExt(desired)
  for (let i = 2; ; i++) {
    const candidate = `${base} ${i}${ext}`
    if (!taken.has(foldName(candidate))) return candidate
  }
}

/** The pre-filled name in the name prompt: unique in the current folder. */
export function defaultNewDocumentName(type: NewDocumentType, existingNames: Iterable<string>): string {
  return uniqueFileName(`${type.defaultBase}.${type.ext}`, existingNames)
}

/** Thrown by Drive's create handler when the fresh listing already holds
 *  the name; its message is user-facing and shown as-is in the prompt. */
export class NewDocumentNameClashError extends Error {
  constructor(name: string) {
    super(`“${name}” already exists in this folder.`)
    this.name = 'NewDocumentNameClashError'
  }
}

export type NameCheck = { ok: true; name: string } | { ok: false; reason: string }

/**
 * Normalises what the user typed in the name prompt into the final file
 * name: trims, and appends the type's extension when it is missing (typing
 * "Budget" for a spreadsheet gives "Budget.xlsx" — the extension decides
 * which editor opens it, so it is never optional). A name that already ends
 * in the extension (any case) is kept as typed.
 *
 * It does NOT silently rename on a clash: the dialog shows the reason and
 * lets the user choose. Silent "Budget 2.xlsx" would be a surprise the
 * user only discovers in the file list.
 */
export function checkNewDocumentName(
  input: string,
  type: NewDocumentType,
  existingNames: Iterable<string>,
): NameCheck {
  const trimmed = input.trim()
  if (!trimmed) return { ok: false, reason: 'Give the file a name.' }
  if (/[/\\]/.test(trimmed)) return { ok: false, reason: 'A name cannot contain / or \\.' }
  const suffix = `.${type.ext}`
  const name = foldName(trimmed).endsWith(suffix) ? trimmed : `${trimmed}${suffix}`
  if (foldName(name) === suffix) return { ok: false, reason: 'Give the file a name.' }
  for (const n of existingNames) {
    if (foldName(n) === foldName(name)) {
      return { ok: false, reason: `“${name}” already exists in this folder.` }
    }
  }
  return { ok: true, name }
}
