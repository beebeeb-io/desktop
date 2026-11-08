// ─── File kind tables ──────────────────────────────
// Which renderer a file gets is decided from its MIME type and extension
// (preview-load-policy.ts owns the media/no-pull routing; these tables cover
// text, code, images and the formats that have no in-browser renderer).
// Kept out of file-preview.tsx so that component holds only orchestration.

import { AUDIO_EXTENSIONS_SET, getExtension } from '../lib/preview-load-policy'

/** Map file extensions to language names for syntax coloring */
export const EXT_LANGUAGE: Record<string, string> = {
  // JavaScript / TypeScript
  js: 'javascript', jsx: 'javascript', ts: 'typescript', tsx: 'typescript',
  mjs: 'javascript', cjs: 'javascript',
  // Systems
  rs: 'rust', go: 'go', c: 'c', h: 'c', cpp: 'cpp', hpp: 'cpp',
  // JVM
  java: 'java', kt: 'kotlin', scala: 'scala',
  // Scripting
  py: 'python', rb: 'ruby', php: 'php', lua: 'lua', pl: 'perl', sh: 'shell',
  bash: 'shell', zsh: 'shell', fish: 'shell',
  // Web
  html: 'html', htm: 'html', css: 'css', scss: 'css', less: 'css', svg: 'xml',
  // Data / config
  json: 'json', yaml: 'yaml', yml: 'yaml', toml: 'toml', xml: 'xml',
  sql: 'sql', graphql: 'graphql', gql: 'graphql',
  // DevOps / config
  dockerfile: 'docker', makefile: 'make', cmake: 'cmake',
  tf: 'terraform', hcl: 'terraform',
  // Other
  swift: 'swift', dart: 'dart', r: 'r', ex: 'elixir', exs: 'elixir',
  erl: 'erlang', hs: 'haskell', ml: 'ocaml', zig: 'zig', nim: 'nim',
  v: 'v', vue: 'vue', svelte: 'svelte', astro: 'astro',
  // .NET — cs was missing entirely (task 1565 preview matrix finding): a
  // C# file fell through every branch in pickRenderer (not text/*, not in
  // TEXT_EXTENSIONS either) straight to the generic "Preview not available"
  // card, unlike every other ~20 code languages this map covers.
  cs: 'csharp',
}

/** Image extension set used when mime_type is null (ZK-uploaded files) */
export const IMAGE_EXTENSIONS_SET = new Set([
  'jpg', 'jpeg', 'png', 'gif', 'webp', 'avif', 'bmp', 'tiff', 'tif', 'svg', 'ico',
])

/** Extensions that should render as plain text (no syntax coloring) */
export const TEXT_EXTENSIONS = new Set([
  'txt', 'log', 'csv', 'tsv', 'env', 'conf', 'ini', 'cfg', 'properties',
  'editorconfig', 'gitignore', 'gitattributes', 'dockerignore', 'npmrc',
  'nvmrc', 'prettierrc', 'eslintrc', 'babelrc',
])

/** MIME types that should render as text */
export const TEXT_MIME_TYPES = new Set([
  'text/plain',
  'text/csv',
  'text/tab-separated-values',
  'text/x-log',
])

/** MIME types that should render as code */
export const CODE_MIME_TYPES: Record<string, string> = {
  'text/javascript': 'javascript',
  'application/javascript': 'javascript',
  'text/typescript': 'typescript',
  'application/typescript': 'typescript',
  'text/html': 'html',
  'text/css': 'css',
  'text/xml': 'xml',
  'application/xml': 'xml',
  'application/json': 'json',
  'application/x-yaml': 'yaml',
  'text/yaml': 'yaml',
  'text/x-python': 'python',
  'text/x-java-source': 'java',
  'text/x-c': 'c',
  'text/x-c++': 'cpp',
  'text/x-rust': 'rust',
  'text/x-go': 'go',
  'text/x-shellscript': 'shell',
  'application/sql': 'sql',
  'application/toml': 'toml',
}

/** The human label the top bar and Info rail show for a file. */
export function getKindLabel(mimeType: string | null | undefined, filename: string): string {
  const ext = getExtension(filename)
  if (mimeType?.startsWith('image/')) {
    const sub = mimeType.split('/')[1]?.toUpperCase() ?? 'Image'
    return sub === 'JPEG' ? 'JPEG Image' : `${sub} Image`
  }
  if (mimeType === 'application/pdf') return 'PDF Document'
  if (mimeType?.startsWith('audio/')) {
    const sub = mimeType.split('/')[1]?.toUpperCase() ?? 'Audio'
    return sub === 'MPEG' ? 'MP3' : sub === 'X-FLAC' ? 'FLAC' : sub
  }
  if (AUDIO_EXTENSIONS_SET.has(ext)) return ext.toUpperCase()
  if (mimeType?.startsWith('video/')) {
    const sub = mimeType.split('/')[1]?.toUpperCase() ?? 'Video'
    return `${sub} Video`
  }
  if (mimeType === 'text/markdown') return 'Markdown'

  if ((mimeType && TEXT_MIME_TYPES.has(mimeType)) || TEXT_EXTENSIONS.has(ext)) {
    if (ext === 'csv') return 'CSV File'
    if (ext === 'tsv') return 'TSV File'
    if (ext === 'log') return 'Log File'
    if (ext === 'env') return 'Environment File'
    if (ext === 'conf' || ext === 'ini' || ext === 'cfg') return 'Config File'
    return 'Plain Text'
  }

  const lang = (mimeType ? CODE_MIME_TYPES[mimeType] : undefined) ?? EXT_LANGUAGE[ext]
  if (lang) {
    return `${lang.charAt(0).toUpperCase() + lang.slice(1)} Source`
  }

  return mimeType ?? (ext.toUpperCase() || 'File')
}

// HEIC/HEIF — decoded client-side via a WASM HEIC decoder inside HeicPreview.
// These are routed away from the native <img> tag because browsers do not
// consistently support this container.
export const HEIC_IMAGE_MIMES = new Set([
  'image/heic',
  'image/heif',
  'image/heic-sequence',
  'image/heif-sequence',
])
export const HEIC_IMAGE_EXTS = new Set(['heic', 'heif'])

// Camera RAW formats — no in-browser decoder exists.
// Show the "unsupported preview" card with download button.
export const RAW_IMAGE_EXTS = new Set(['dng', 'cr2', 'cr3', 'nef', 'arw', 'orf', 'rw2', 'raf'])
export const RAW_IMAGE_MIMES = new Set([
  'image/x-adobe-dng',
  'image/x-canon-cr2',
  'image/x-canon-cr3',
  'image/x-nikon-nef',
  'image/x-sony-arw',
  'image/x-olympus-orf',
  'image/x-panasonic-rw2',
  'image/x-fuji-raf',
])

// PPTX — no reliable in-browser renderer exists. Task 1574 evaluated reusing
// the LibreOffice-WASM office editor (task 1567) for a read-only preview
// here; as of this task, 1567 is an unmerged, still-in-spike-phase effort
// (its own Notes: "Phase 1 spike done… STOP the native-macOS-host attempt
// here" — no working WASM build yet, let alone a mergeable editor). A small
// pure-JS PPTX slide renderer was also considered and rejected: PPTX slide
// XML (themes, layouts, placeholders, shape/text positioning) has no small
// maintained renderer that produces a faithful thumbnail — every option
// found was either a generator (pptxgenjs/officegen, write-only) or would
// have meant hand-building a slide layout engine, exactly the "throwaway
// renderer" this task's own brief says not to build. Legacy .doc/.xls/.ppt
// are binary OLE2 formats with no pure-JS reader in this repo's dependency
// tree at all. All four stay on the honest download card.
export const PPTX_MIMES = new Set([
  'application/vnd.openxmlformats-officedocument.presentationml.presentation',
  'application/vnd.ms-powerpoint',
])
export const PPTX_EXTS = new Set(['pptx', 'ppt', 'odp', 'key'])

// TIFF — Chromium has no native TIFF codec (task 1565 finding). Decoded in
// a Web Worker via utif2, see tiff-preview.tsx.
export const TIFF_MIMES = new Set(['image/tiff'])
export const TIFF_EXTS = new Set(['tiff', 'tif'])

// ZIP — a honest file LISTING (names/sizes/folders) from the Central
// Directory, never extraction. See zip-listing-preview.tsx.
export const ZIP_EXTS = new Set(['zip'])

/** Which in-place editor a file opens: markdown, a code language, or plain text. */
export interface EditableKind {
  isMarkdown: boolean
  language: string | undefined
}

/**
 * Resolve the editor a markdown/code/text file opens in, or null when the file
 * is not an editor candidate at all. Being a candidate does not mean it is
 * editable: checkEditability still applies its byte-level gate (over 2 MB,
 * binary, or not valid UTF-8 → read-only notice).
 */
export function resolveEditableKind(
  mimeType: string | null | undefined,
  filename: string,
): EditableKind | null {
  const ext = getExtension(filename)
  if (mimeType === 'text/markdown' || ext === 'md' || ext === 'mdx') {
    return { isMarkdown: true, language: 'markdown' }
  }
  const lang = (mimeType ? CODE_MIME_TYPES[mimeType] : undefined) ?? EXT_LANGUAGE[ext]
  if (lang) {
    return { isMarkdown: false, language: lang }
  }
  if ((mimeType ? TEXT_MIME_TYPES.has(mimeType) : false) || TEXT_EXTENSIONS.has(ext)) {
    return { isMarkdown: false, language: undefined }
  }
  return null
}
