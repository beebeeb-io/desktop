// Pure helpers for the Open section: which viewer a file gets, the CSV table, line counts, sizes and times.
import { MAX_EDITABLE_BYTES, RAW_IMAGE_EXTS } from '@beebeeb/shared/drive-ui'

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

/** How long after this window's own save a new version is taken to be that save landing (DW-1519). */
export const OWN_SAVE_MS = 60_000

/** What a version change means while a file is open (DW-1519): the one already shown, this window's own save landing
 *  (saved over `ours.from` within OWN_SAVE_MS), or news from another device. */
export function versionNews(s: { base: string | null; ours: { from: string | null; at: number } | null }, current: string, now: number): 'same' | 'ours' | 'news' {
  if (current === s.base) return 'same'
  if (s.ours && now - s.ours.at < OWN_SAVE_MS) return 'ours'
  return 'news'
}
