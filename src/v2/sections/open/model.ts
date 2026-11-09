// Pure helpers for the Open section: which viewer a file gets, the CSV table, line counts, sizes and times.
import { DriveSourceError, insertBeforeExtension, MAX_EDITABLE_BYTES, RAW_IMAGE_EXTS, type DriveItem } from '@beebeeb/shared/drive-ui'

export type ViewerKind = 'raw' | 'image' | 'audio' | 'pdf' | 'markdown' | 'csv' | 'archive' | 'code' | 'text' | 'other'

const CODE_EXT = new Set(['py', 'js', 'ts', 'tsx', 'jsx', 'json', 'sh', 'rs', 'css', 'html', 'yml', 'yaml', 'toml', 'go', 'c', 'cpp', 'java'])
const TEXT_EXT = new Set(['txt', 'log', 'ini', 'cfg', 'env'])
const IMAGE_EXT = new Set(['jpg', 'jpeg', 'png', 'gif', 'webp', 'bmp', 'svg'])
const AUDIO_EXT = new Set(['mp3', 'wav', 'flac', 'm4a', 'ogg', 'aac'])

export function extOf(name: string): string {
  const dot = name.lastIndexOf('.')
  return dot <= 0 ? '' : name.slice(dot + 1).toLowerCase()
}

export function viewerKind(name: string): ViewerKind {
  const ext = extOf(name)
  if (RAW_IMAGE_EXTS.has(ext)) return 'raw'
  if (IMAGE_EXT.has(ext)) return 'image'
  if (AUDIO_EXT.has(ext)) return 'audio'
  if (ext === 'pdf') return 'pdf'
  if (ext === 'md' || ext === 'markdown') return 'markdown'
  if (ext === 'csv' || ext === 'tsv') return 'csv'
  if (ext === 'zip') return 'archive'
  if (CODE_EXT.has(ext)) return 'code'
  if (TEXT_EXT.has(ext)) return 'text'
  return 'other'
}

export const LANGUAGE_LABEL: Record<string, string> = { py: 'Python', js: 'JavaScript', ts: 'TypeScript', json: 'JSON', sh: 'Shell', rs: 'Rust', css: 'CSS', html: 'HTML', md: 'Markdown', txt: 'Plain text', log: 'Plain text' }
export const languageLabel = (name: string) => LANGUAGE_LABEL[extOf(name)] ?? 'Plain text'

/** The id the CodeMirror wrapper uses for a file (editor-langs.ts). */
export function editorLanguage(name: string): string | undefined {
  const ext = extOf(name)
  return ({ py: 'python', js: 'javascript', ts: 'typescript', tsx: 'typescript', json: 'json', md: 'markdown', markdown: 'markdown', css: 'css', html: 'html', rs: 'rust' } as Record<string, string>)[ext]
}

export const lineCount = (text: string) => (text === '' ? 0 : text.replace(/\n$/, '').split('\n').length)

/** The separator is detected: the one that appears most in the first line (comma, semicolon, tab). */
export function detectSeparator(text: string): { sep: string; label: string } {
  const first = text.split('\n', 1)[0] ?? ''
  const counts = [',', ';', '\t'].map((s) => [s, first.split(s).length - 1] as const).sort((a, b) => b[1] - a[1])
  const sep = counts[0][1] > 0 ? counts[0][0] : ','
  return { sep, label: sep === ',' ? 'Comma-separated' : sep === ';' ? 'Semicolon-separated' : 'Tab-separated' }
}

export function parseTable(text: string): { sep: { sep: string; label: string }; header: string[]; rows: string[][] } {
  const sep = detectSeparator(text)
  const lines = text.split(/\r?\n/).filter((l) => l !== '')
  const cells = (line: string) => {
    const out: string[] = []
    let cur = ''
    let quoted = false
    for (let i = 0; i < line.length; i++) {
      const c = line[i]
      if (quoted) {
        if (c === '"' && line[i + 1] === '"') { cur += '"'; i++ } else if (c === '"') { quoted = false } else { cur += c }
      } else if (c === '"') quoted = true
      else if (c === sep.sep) { out.push(cur); cur = '' } else cur += c
    }
    out.push(cur)
    return out
  }
  const [head = '', ...rest] = lines
  return { sep, header: cells(head), rows: rest.map(cells) }
}

export const isNumeric = (s: string) => /^-?\d+([.,]\d+)?$/.test(s.trim())

export function fmtBytes(bytes: number): string {
  if (bytes >= 1024 ** 3) return `${(bytes / 1024 ** 3).toFixed(1)} GB`
  if (bytes >= 1024 ** 2) return `${(bytes / 1024 ** 2).toFixed(1)} MB`
  if (bytes >= 1024) return `${Math.round(bytes / 1024)} KB`
  return `${bytes} B`
}

export const fmtClock = (seconds: number) => `${Math.floor(seconds / 60)}:${String(Math.floor(seconds % 60)).padStart(2, '0')}`
export const hhmm = (iso: string) => new Date(iso).toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit' })

/** The editor's limit is the web editor's (text-editability): UTF-8 text up to 2 MB. */
export const canEditSize = (bytes: number) => bytes <= MAX_EDITABLE_BYTES
export const READ_ONLY_LINES = 500

/** Where a pending-change warning applies: leaving the editor with edits. */
export const isDirty = (current: string, saved: string) => current !== saved

/** Lines added or removed against the other version: "+1 line". */
export function lineDelta(mine: string, theirs: string): string {
  const d = lineCount(mine) - lineCount(theirs)
  const n = Math.abs(d)
  return d === 0 ? 'edited' : `${d > 0 ? '+' : '−'}${n} line${n === 1 ? '' : 's'}`
}

/** A window-level key handler leaves a key alone while a field or a media element has it, while Space or Enter would
 *  press a focused button or link, and while a dialog is open over the view (its own keys come first). */
export function keyBelongsElsewhere(e: Pick<KeyboardEvent, 'key' | 'target'>, doc: Pick<Document, 'querySelector'> | undefined = globalThis.document): boolean {
  const t = e.target as { tagName?: string; isContentEditable?: boolean } | null
  if (t && typeof t.tagName === 'string') {
    if (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT|VIDEO|AUDIO)$/.test(t.tagName)) return true
    if ((e.key === ' ' || e.key === 'Enter') && /^(BUTTON|A)$/.test(t.tagName)) return true
  }
  return !!doc?.querySelector('[role="dialog"], [role="alertdialog"], dialog[open]')
}

/** The content part of Rust's read tag (`v3-<content>`, drive_v2 fsops::version_tag): two reads of the same content
 *  carry the same part, whatever the version number. Null for a tag without one. */
export const tagContent = (tag: string): string | null => (tag.includes('-') ? tag.slice(tag.indexOf('-') + 1) : null)
/** The version number in Rust's tag: the server's number, which Rust keeps until the engine has uploaded a save. */
export const tagVersion = (tag: string): number | null => { const m = /^v(\d+)/.exec(tag); return m ? Number(m[1]) : null }

/**
 * What the open file's version is, and what this window is waiting for (DW-1519, DW-1524). `base` is Rust's version
 * number (drive_details' versionCount) of what the window shows. `pending` is this window's own save or restore until
 * the engine has it on the server: Rust writes a save on this PC at once, and the engine encrypts and uploads it later.
 */
export interface VersionWatch {
  base: number
  pending: { kind: 'save'; from: number; content: string | null } | { kind: 'restore'; from: number } | null
}

/** What a re-read of the file found: its version number, its local state, and (for a save) the content part of a read. */
export interface VersionSeen { version: number; local?: DriveItem['local']; content?: string | null }

/**
 * What a re-read means: `same` (nothing has landed), `saved` (this window's save is on the server: the version moved
 * past the one it was saved over and the file holds the save's content), `restored` (this window's restore landed), or
 * `news` (another device's version). Decided by the versions and the content, never by a time window: a save the
 * engine uploads minutes later is still this window's, and another device's version that lands first is still news
 * (desktop #273 finding 3). A version still moving on this PC (`syncing`) has not landed.
 */
export function versionNews(w: VersionWatch, seen: VersionSeen): 'same' | 'saved' | 'restored' | 'news' {
  if (seen.version <= (w.pending?.from ?? w.base) || seen.local === 'syncing') return 'same'
  if (w.pending?.kind === 'restore') return 'restored'
  if (w.pending?.kind === 'save') return w.pending.content === null || seen.content === w.pending.content ? 'saved' : 'news'
  return 'news'
}

/** A save's note under the toolbar and on the status line (DW-1524, lead's ruling on #273 finding 12): saved on this PC
 *  until the engine has encrypted and uploaded it, then encrypted with Rust's version number. */
export type SaveNote = { kind: 'local' } | { kind: 'encrypted'; version: number }
export const saveNoteText = (n: SaveNote) => (n.kind === 'local' ? 'Saved on this PC · encrypting…' : `Saved · encrypted · version ${n.version}`)

/** A free name for a kept copy ("Keep both" in the text editor and in Office), by create-file's rule: a name is taken
 *  when the folder has it in any case. The label, then the label with 2, 3, …: "Meeting notes (This PC).md", then
 *  "Meeting notes (This PC 2).md" (#273 finding 2). */
export function keptCopyName(name: string, label: string, taken: Iterable<string>): string {
  const lower = new Set([...taken].map((n) => n.toLowerCase()))
  for (let n = 1; ; n++) {
    const candidate = insertBeforeExtension(name, ` (${n === 1 ? label : `${label} ${n}`})`)
    if (!lower.has(candidate.toLowerCase())) return candidate
  }
}

/** How many names "Keep both" tries when Rust still finds one taken (a cloud-only name, or Windows' 8.3 alias of one). */
export const KEEP_BOTH_TRIES = 5

/** Make the kept copy under the first free name. The folder's names come first; a name Rust refuses as taken (one the
 *  listing could not show) moves on to the next, so a second conflict on the same file never fails for good. */
export async function createKeptCopy<T>(name: string, label: string, folderNames: () => Promise<string[]>, create: (name: string) => Promise<T>): Promise<{ made: T; name: string }> {
  const taken = new Set((await folderNames().catch((): string[] => [])).map((n) => n.toLowerCase()))
  for (let i = 0; i < KEEP_BOTH_TRIES; i++) {
    const candidate = keptCopyName(name, label, taken)
    try {
      return { made: await create(candidate), name: candidate }
    } catch (e) {
      if (!(e instanceof DriveSourceError && e.code === 'name-taken')) throw e
      taken.add(candidate.toLowerCase())
    }
  }
  throw new DriveSourceError('name-taken', 'keep both')
}
